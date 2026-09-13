import contextlib
import shlex
import shutil
import threading
from pathlib import Path

from PySide6.QtCore import Property, QObject, Signal, Slot
from PySide6.QtWidgets import QFileDialog, QSystemTrayIcon

from bridge.slots import deferred_slot
from core import log
from core.constants import (
    BIN_DIR,
    DATA_ROOT,
    DD_BIN,
    DEFAULT_ACCENT,
    DEFAULT_BAR_FILL,
    DEFAULT_BAR_STRIPE,
    DEFAULT_DOWNLOADS_DIR,
    DEFAULT_MAX_DOWNLOADS,
    DEFAULT_USERNAME,
    DOWNLOADS_MAX,
    DOWNLOADS_MIN,
    FOLDER_NOT_FOUND,
    HEX_PATTERN,
    HOME_GRID_DEFAULT_SIZE,
    HOME_GRID_MAX_H,
    HOME_GRID_MAX_W,
    INVALID_NAME_CHARS,
    LOG_FILE,
    MAX_USERNAME_LENGTH,
    SEVENZ_BIN,
    TL_DIR,
    TRANSFER_RUNNING,
    UPDATE_RUNNING,
    version_tuple,
)
from core.manifest import (
    effective_username,
    installed_downloads,
    is_installed,
    is_season_folder,
    local_downloads,
    write_download_username,
)
from core.settings import (
    default_library,
    get_setting,
    pop_settings,
    save_settings,
    set_libraries,
    set_setting,
)
from core.settings import (
    libraries as library_roots,
)
from core.steam import is_game_running, list_protons, resolve_proton
from core.throwbackloader import apply_tl, tl_folder_version, tl_present, tl_version, write_launcher
from layout import user_data_base


def _wipe_depot_token() -> tuple[bool, list[str]]:
    iso_dir = user_data_base() / "IsolatedStorage"
    stores = (
        sorted({p.parent.parent for p in iso_dir.rglob("AssemFiles/account.config")})
        if iso_dir.exists()
        else []
    )
    if not stores:
        return False, []
    errors = []
    for store in stores:
        try:
            shutil.rmtree(store)
        except OSError as e:
            errors.append(log.fail("Could not remove the Steam token", e))
    return True, errors


def _clear_download_cache() -> None:
    DD_BIN.unlink(missing_ok=True)
    SEVENZ_BIN.unlink(missing_ok=True)
    LOG_FILE.unlink(missing_ok=True)
    for archive in BIN_DIR.glob("*.7z"):
        archive.unlink(missing_ok=True)
    if TL_DIR.exists():
        shutil.rmtree(TL_DIR)


PREF_DEFAULTS: dict[str, object] = {
    "username": DEFAULT_USERNAME,
    "steam_account": "",
    "max_downloads": DEFAULT_MAX_DOWNLOADS,
    "discord_rpc": True,
    "close_to_tray": False,
    "reduce_motion": False,
    "liberator_enabled": True,
    "rvpn_autorun": False,
    "home_order": [],
    "home_sizes": {},
    "launch_args": {},
    "accent": DEFAULT_ACCENT,
    "bar_fill": DEFAULT_BAR_FILL,
    "bar_stripe": DEFAULT_BAR_STRIPE,
}


def _display_path(root: Path) -> str:
    try:
        return str(Path("~") / root.relative_to(Path.home().resolve()))
    except ValueError:
        return str(root)


class SettingsController(QObject):
    username_changed = Signal()
    steam_account_changed = Signal()
    max_downloads_changed = Signal()
    discord_rpc_changed = Signal()
    close_to_tray_changed = Signal()
    reduce_motion_changed = Signal()
    settings_error = Signal(str, str)
    _library_tl_failed_in = Signal(str)
    logged_out = Signal(str, str)
    cache_cleared = Signal()
    libraries_changed = Signal()
    home_order_changed = Signal()
    home_sizes_changed = Signal()
    launch_args_changed = Signal()
    liberator_enabled_changed = Signal()
    rvpn_autorun_changed = Signal()
    proton_changed = Signal()
    bar_fill_changed = Signal()
    bar_stripe_changed = Signal()
    accent_changed = Signal()

    _cache_cleared_in = Signal(str)
    _logged_out_in = Signal(str, str)

    def __init__(self, settings: dict, downloader: QObject, updater: QObject) -> None:
        super().__init__()
        self._settings = settings
        self._downloader = downloader
        self._updater = updater
        self._library_tl_failed_in.connect(self._on_library_tl_failed)
        self._clearing_cache = False
        self._logging_out = False
        self._cache_cleared_in.connect(self._on_cache_cleared)
        self._logged_out_in.connect(self._on_logged_out)

    @Property(str, notify=username_changed)
    def username(self) -> str:
        return str(self._pref("username"))

    @Property(str, notify=steam_account_changed)
    def steam_account(self) -> str:
        return str(self._pref("steam_account"))

    @Property(int, notify=max_downloads_changed)
    def max_downloads(self) -> int:
        return int(self._pref("max_downloads"))

    @Property(bool, notify=discord_rpc_changed)
    def discord_rpc(self) -> bool:
        return bool(self._pref("discord_rpc"))

    @Property(bool, constant=True)
    def tray_available(self) -> bool:
        return QSystemTrayIcon.isSystemTrayAvailable()

    @Property(bool, notify=close_to_tray_changed)
    def close_to_tray(self) -> bool:
        return bool(self._pref("close_to_tray"))

    @Property(bool, notify=reduce_motion_changed)
    def reduce_motion(self) -> bool:
        return bool(self._pref("reduce_motion"))

    @Property("QVariantList", notify=home_order_changed)
    def home_order(self) -> list:
        return [str(k) for k in self._pref("home_order")]

    @Slot("QVariantList")
    def set_home_order(self, order: list) -> None:
        value = [str(k) for k in order]
        if value == self.home_order:
            return
        self._store_pref("home_order", value)
        self.home_order_changed.emit()

    @Property("QVariantMap", notify=home_sizes_changed)
    def home_sizes(self) -> dict:
        raw = self._pref("home_sizes")
        return {str(k): v for k, v in raw.items() if isinstance(v, str)}

    @Slot(str, int, int)
    def set_home_size(self, key: str, width: int, height: int) -> None:
        size = f"{max(1, min(HOME_GRID_MAX_W, width))}x{max(1, min(HOME_GRID_MAX_H, height))}"
        sizes = self.home_sizes
        if size == HOME_GRID_DEFAULT_SIZE:
            if key not in sizes:
                return
            del sizes[key]
        elif sizes.get(key) == size:
            return
        else:
            sizes[key] = size
        self._store_pref("home_sizes", sizes)
        self.home_sizes_changed.emit()

    @Property("QVariantMap", notify=launch_args_changed)
    def launch_args(self) -> dict:
        raw = self._pref("launch_args")
        if not isinstance(raw, dict):
            return {}
        return {str(k): v for k, v in raw.items() if isinstance(v, str)}

    @Slot(str, str)
    def set_launch_args(self, key: str, value: str) -> None:
        value = value.strip()
        try:
            shlex.split(value)
        except ValueError:
            self.settings_error.emit(
                "launch_args", "Launch options have an unclosed quote or escape"
            )
            return
        args = self.launch_args
        if not value:
            if key not in args:
                return
            del args[key]
        elif args.get(key) == value:
            return
        else:
            args[key] = value
        self._store_pref("launch_args", args)
        self.launch_args_changed.emit()

    @Slot()
    def reset_home_layout(self) -> None:
        if not pop_settings(self._settings, "home_order", "home_sizes"):
            return
        self.home_order_changed.emit()
        self.home_sizes_changed.emit()

    @Property(bool, notify=liberator_enabled_changed)
    def liberator_enabled(self) -> bool:
        return bool(self._pref("liberator_enabled"))

    @Slot(bool)
    def set_liberator_enabled(self, value: bool) -> None:
        self._store_bool("liberator_enabled", value, self.liberator_enabled_changed)

    @Property(bool, notify=rvpn_autorun_changed)
    def rvpn_autorun(self) -> bool:
        return bool(self._pref("rvpn_autorun"))

    @Slot(bool)
    def set_rvpn_autorun(self, value: bool) -> None:
        self._store_bool("rvpn_autorun", value, self.rvpn_autorun_changed)

    @Property(str, notify=proton_changed)
    def proton(self) -> str:
        proton = resolve_proton(self._settings)
        return proton["internal"] if proton is not None else ""

    @Slot(result="QVariantList")
    def proton_options(self) -> list:
        return [{"internal": p["internal"], "display": p["display"]} for p in list_protons()]

    @Slot(str)
    def set_proton(self, internal: str) -> None:
        protons = list_protons()
        if internal not in {p["internal"] for p in protons}:
            return
        current = resolve_proton(self._settings, protons)
        if current is not None and internal == current["internal"]:
            return
        self._store("proton", internal)
        self.proton_changed.emit()

    @Property(str, notify=bar_fill_changed)
    def bar_fill(self) -> str:
        return self._color("bar_fill")

    @Slot(str)
    def set_bar_fill(self, value: str) -> None:
        self._store_color("bar_fill", value, self.bar_fill_changed)

    @Slot()
    def reset_accent(self) -> None:
        if not pop_settings(self._settings, "accent"):
            return
        self.accent_changed.emit()

    @Property(str, notify=accent_changed)
    def accent(self) -> str:
        return self._color("accent")

    @Slot(str)
    def set_accent(self, value: str) -> None:
        self._store_color("accent", value, self.accent_changed)

    @Property(str, notify=bar_stripe_changed)
    def bar_stripe(self) -> str:
        return self._color("bar_stripe")

    @Slot(str)
    def set_bar_stripe(self, value: str) -> None:
        self._store_color("bar_stripe", value, self.bar_stripe_changed)

    def _color(self, key: str) -> str:
        value = str(get_setting(self._settings, key, ""))
        return value if HEX_PATTERN.match(value) else ""

    def _store_color(self, key: str, value: str, changed: Signal) -> None:
        default = PREF_DEFAULTS[key]
        color = value.strip().lower()
        if not HEX_PATTERN.match(color):
            self.settings_error.emit(key, f"Enter a color like {default}")
            return
        if color != default and color == self._color(key):
            return
        if self._store_pref(key, color):
            changed.emit()

    @Property("QVariantMap", constant=True)
    def download_bounds(self) -> dict:
        return {"min": DOWNLOADS_MIN, "max": DOWNLOADS_MAX}

    def _pref(self, key: str) -> object:
        return get_setting(self._settings, key, PREF_DEFAULTS[key])

    def _store(self, key: str, value: object) -> None:
        set_setting(self._settings, key, value)
        save_settings(self._settings)

    def _store_pref(self, key: str, value: object) -> bool:
        if value == PREF_DEFAULTS[key]:
            return pop_settings(self._settings, key)
        self._store(key, value)
        return True

    def _store_bool(self, key: str, value: bool, changed: Signal) -> None:
        if bool(value) == bool(self._pref(key)):
            return
        self._store_pref(key, bool(value))
        changed.emit()

    @Slot(str)
    def set_username(self, value: str) -> None:
        value = INVALID_NAME_CHARS.sub("", value)
        if not value:
            self.settings_error.emit("username", "Username is empty")
            return
        if len(value) > MAX_USERNAME_LENGTH:
            self.settings_error.emit(
                "username", f"Username is too long (max {MAX_USERNAME_LENGTH} characters)"
            )
            return
        if value == self.username:
            return
        self._store_pref("username", value)
        for d in installed_downloads():
            with contextlib.suppress(OSError):
                write_download_username(d, value)
        self.username_changed.emit()

    def set_steam_account(self, value: str) -> None:
        value = value.strip()
        if value == self.steam_account:
            return
        self._store_pref("steam_account", value)
        self.steam_account_changed.emit()

    @Slot(int)
    def set_max_downloads(self, value: int) -> None:
        if not DOWNLOADS_MIN <= value <= DOWNLOADS_MAX:
            self.settings_error.emit(
                "max_downloads", f"Parallel downloads must be {DOWNLOADS_MIN}–{DOWNLOADS_MAX}"
            )
            return
        if value == self.max_downloads:
            return
        self._store_pref("max_downloads", value)
        self.max_downloads_changed.emit()

    @Slot(bool)
    def set_discord_rpc(self, value: bool) -> None:
        self._store_bool("discord_rpc", value, self.discord_rpc_changed)

    @Slot(bool)
    def set_close_to_tray(self, value: bool) -> None:
        self._store_bool("close_to_tray", value, self.close_to_tray_changed)

    @Slot(bool)
    def set_reduce_motion(self, value: bool) -> None:
        self._store_bool("reduce_motion", value, self.reduce_motion_changed)

    @Slot()
    def logout(self) -> None:
        if self._logging_out:
            return
        if self._downloader.running:
            self.settings_error.emit("logout", "A download is using your Steam session")
            return
        if self._updater.busy:
            self.settings_error.emit("logout", UPDATE_RUNNING)
            return
        self.set_steam_account("")
        self._logging_out = True

        def work() -> None:
            found, errors = _wipe_depot_token()
            if errors:
                self._logged_out_in.emit("error", errors[0])
            elif found:
                self._logged_out_in.emit("done", "Logged out")
            else:
                self._logged_out_in.emit("error", "No Steam token found")

        threading.Thread(target=work, daemon=True).start()

    def _on_logged_out(self, kind: str, message: str) -> None:
        self._logging_out = False
        self.logged_out.emit(kind, message)

    def _transfers_busy(self) -> bool:
        return bool(self._downloader.running or self._updater.busy)

    @Slot(result=str)
    def launcher_folder(self) -> str:
        return str(DATA_ROOT)

    @Slot(result="QVariantList")
    def libraries(self) -> list:
        folders = local_downloads()
        return [
            {
                "path": str(root),
                "display": _display_path(root),
                "default": i == 0,
                "fixed": root == DEFAULT_DOWNLOADS_DIR,
                "exists": root.exists(),
                "seasons": sum(1 for d in folders if d.parent == root),
            }
            for i, root in enumerate(library_roots())
        ]

    def _save_libraries(self, roots: list[Path]) -> None:
        values = [str(root) for root in roots]
        self._store("libraries", values)
        set_libraries(values)
        self.libraries_changed.emit()

    @deferred_slot()
    def add_library(self) -> None:
        if self._transfers_busy():
            self.settings_error.emit("libraries", TRANSFER_RUNNING)
            return
        picked = QFileDialog.getExistingDirectory(
            None, "Choose library folder", str(default_library())
        )
        if not picked:
            return
        path = Path(picked).resolve()
        if is_season_folder(path):
            path = path.parent
        roots = library_roots()
        if path in roots:
            self.settings_error.emit("libraries", "Folder is already a library")
            return
        if any(path.is_relative_to(root) or root.is_relative_to(path) for root in roots):
            self.settings_error.emit("libraries", "Library folders cannot contain each other")
            return
        self._save_libraries([*roots, path])
        threading.Thread(target=self._align_library_tl, args=(path,), daemon=True).start()

    def _on_library_tl_failed(self, message: str) -> None:
        self.settings_error.emit("libraries", message)

    def _align_library_tl(self, root: Path) -> None:
        latest = tl_version()
        if not tl_present() or latest is None or is_game_running():
            return
        username = effective_username(self._settings)
        for folder in sorted(root.glob("*")):
            if root not in library_roots():
                return
            if not is_season_folder(folder) or not is_installed(folder):
                continue
            current = tl_folder_version(folder)
            try:
                if current is not None and version_tuple(current) >= version_tuple(latest):
                    write_download_username(folder, username)
                    continue
                apply_tl(folder, username)
                write_launcher(folder)
            except OSError as e:
                self._library_tl_failed_in.emit(log.fail("ThrowbackLoader setup failed", e))
                return

    @Slot(str)
    def remove_library(self, path: str) -> None:
        roots = library_roots()
        target = Path(path)
        if target == DEFAULT_DOWNLOADS_DIR:
            self.settings_error.emit("libraries", "The Launcher folder cannot be removed")
            return
        if target not in roots:
            return
        if self._updater.busy:
            self.settings_error.emit("libraries", UPDATE_RUNNING)
            return
        if self._downloader.uses_library(target):
            self.settings_error.emit("libraries", "A download is using this library")
            return
        self._save_libraries([root for root in roots if root != target])

    @Slot(str)
    def set_default_library(self, path: str) -> None:
        roots = library_roots()
        target = Path(path)
        if target not in roots or target == roots[0]:
            return
        if not target.exists():
            self.settings_error.emit("libraries", FOLDER_NOT_FOUND)
            return
        self._save_libraries([target, *(root for root in roots if root != target)])

    def clearing_cache(self) -> bool:
        return self._clearing_cache

    @Slot()
    def clear_cache(self) -> None:
        if self._clearing_cache:
            return
        if self._transfers_busy():
            self.settings_error.emit("cache", TRANSFER_RUNNING)
            return
        self._clearing_cache = True

        def work() -> None:
            message = ""
            try:
                _clear_download_cache()
            except Exception as e:
                message = log.fail("Could not clear the cache", e)
            self._cache_cleared_in.emit(message)

        threading.Thread(target=work, daemon=True).start()

    def _on_cache_cleared(self, message: str) -> None:
        self._clearing_cache = False
        if message:
            self.settings_error.emit("cache", message)
            return
        self.cache_cleared.emit()
