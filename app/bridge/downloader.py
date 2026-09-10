import math
import re
import shutil
import threading
from collections import deque
from collections.abc import Callable
from pathlib import Path
from typing import NamedTuple

from PySide6.QtCore import QObject, QProcess, QTimer, Signal, Slot

from bridge.dialogs import pick_file
from bridge.reporter import SignalReporter
from bridge.slots import deferred_slot
from core import log
from core.constants import (
    ALREADY_INSTALLED,
    BIN_DIR,
    BUSY_MESSAGE,
    CACHE_CLEARING,
    DEFAULT_MAX_DOWNLOADS,
    DOWNLOAD_RUNNING,
    DOWNLOADS_MAX,
    DOWNLOADS_MIN,
    FOLDER_NOT_FOUND,
    GIB,
    NAME_PATTERN,
    NOT_INSTALLED,
    NOT_INSTALLED_HM,
    PARTIAL_EXISTS,
    REMOVING_FILES,
    SEASON_RUNNING,
    TL_EXTRACT,
    TL_LAUNCHER,
    UNINSTALL_RUNNING,
    UPDATE_RUNNING,
)
from core.depot import depot_commands, ensure_depotdownloader
from core.github import RateLimitError
from core.heatedmetal import apply_hm, cache_hm_archive, remove_hm_files
from core.manifest import (
    edition_folder,
    effective_username,
    installed_path,
    is_installed,
    partial_path,
    season_code,
)
from core.reporter import Reporter
from core.settings import default_library, get_setting, libraries, save_settings, set_setting
from core.shears import folder_size
from core.steam import is_season_running
from core.throwbackloader import apply_tl, ensure_tl, write_launcher

_PERCENT = re.compile(r"^\s*(\d+(?:\.\d+)?)\s*%")
_STAGING_HEADROOM = 2 * GIB
_LOG_HISTORY_LIMIT = 2000
_SHUTDOWN_WAIT_MS = 2000


class QueueEntry(NamedTuple):
    key: str
    hm: bool
    library: str
    verify: bool


class PendingRequest(NamedTuple):
    kind: str
    key: str
    hm: bool = False
    library: str = ""
    shortfall: float = 0.0


def _out_of_space(root: Path) -> bool:
    try:
        return shutil.disk_usage(root).free < _STAGING_HEADROOM
    except OSError:
        return False


def _apply_install(
    target: Path,
    download: dict,
    is_hm: bool,
    username: str,
    reporter: Reporter,
    archive: Path | None = None,
) -> bool:
    if is_hm:
        try:
            apply_hm(target, username, download, reporter=reporter, archive=archive)
        except OSError as e:
            reporter.fail(str(e))
            return False
        return True
    reporter.update("Copying files")
    try:
        apply_tl(target, username)
        write_launcher(target)
    except OSError as e:
        reporter.fail(log.fail("ThrowbackLoader setup failed", e))
        return False
    return True


class DownloadController(QObject):
    log_line = Signal(str)
    log_history = Signal(str, str)
    progress_changed = Signal()
    state_changed = Signal()
    verifying_changed = Signal()
    done = Signal(str, str)
    login_required = Signal(str)
    disk_space_required = Signal(float)
    error = Signal(str)
    active_key_changed = Signal()
    active_hm_changed = Signal()
    partial_deleted = Signal(str, bool, bool, str)
    rate_limited = Signal(str)
    warning = Signal(str)
    queue_changed = Signal()

    _prepare_done_in = Signal(int, str, str)
    _apply_done_in = Signal(int, bool)
    _deleted_in = Signal(str, bool, bool, str)
    _hm_cached_in = Signal(str, str)
    _error_in = Signal(str)
    _rate_limited_in = Signal(str)

    def __init__(self, settings: dict, downloads: list[dict]) -> None:
        super().__init__()
        self._settings = settings
        self._downloads = downloads
        self._process: QProcess | None = None
        self._proc_done = False
        self._progress = 0.0
        self._state = "idle"
        self._active_key = ""
        self._cancelled = False
        self._generation = 0
        self._login_pending = False
        self._login_kind = ""
        self._pending_password = ""
        self._pending_request: PendingRequest | None = None
        self._buffer = ""
        self._history: deque[str] = deque(maxlen=_LOG_HISTORY_LIMIT)
        self._commands: list[dict] = []
        self._index = 0
        self._step = 0
        self._steps = 0
        self._download: dict = {}
        self._active_hm = False
        self._verifying = False
        self._steam_account = ""
        self._max_downloads = DEFAULT_MAX_DOWNLOADS
        self._target = default_library()
        self._dd = ""
        self._deleting_key: str | None = None
        self._picking = False
        self._picked_archive: Path | None = None
        self._hm_resume: Callable[[], None] | None = None
        self._archive: Path | None = None
        self._rate_limit_hit = False
        self._switching = ""
        self._paused = bool(get_setting(settings, "queue_paused", False))
        known = {d["key"] for d in downloads}
        self._queue: list[QueueEntry] = [
            QueueEntry(
                item["key"],
                bool(item.get("hm")),
                str(item.get("library", "")),
                bool(item.get("verify")),
            )
            for item in get_setting(settings, "queue", [])
            if isinstance(item, dict)
            and item.get("key") in known
            and (installed_path(item["key"], bool(item.get("hm"))) is not None)
            == bool(item.get("verify"))
        ]
        if self._paused:
            if self._enter_paused_from_queue():
                self._state = "paused"
            else:
                self._clear_paused_flag()
        self.log_line.connect(self._history.append)
        self._prepare_done_in.connect(self._on_prepare_done_in)
        self._apply_done_in.connect(self._on_apply_done_in)
        self._deleted_in.connect(self._on_deleted_in)
        self._hm_cached_in.connect(self._on_hm_cached_in)
        self._error_in.connect(self.error)
        self._rate_limited_in.connect(self._on_rate_limited_in)

    @property
    def progress(self) -> float:
        return self._progress

    @property
    def step(self) -> int:
        return self._step

    @property
    def steps(self) -> int:
        return self._steps

    @property
    def running(self) -> bool:
        return self._state in ("preparing", "downloading", "applying")

    @property
    def state(self) -> str:
        return self._state

    @property
    def active_key(self) -> str:
        return self._active_key

    @property
    def active_hm(self) -> bool:
        return self._active_hm

    @property
    def verifying(self) -> bool:
        return self._verifying

    def _set_progress(self, value: float) -> None:
        if value != self._progress:
            self._progress = value
            self.progress_changed.emit()

    def _set_step(self, index: int, total: int) -> None:
        if (index, total) != (self._step, self._steps):
            self._step = index
            self._steps = total
            self.progress_changed.emit()

    def _set_state(self, value: str) -> None:
        if value != self._state:
            self._state = value
            self.state_changed.emit()

    def _set_verifying(self, value: bool) -> None:
        if value != self._verifying:
            self._verifying = value
            self.verifying_changed.emit()

    def _set_active_hm(self, value: bool) -> None:
        if value != self._active_hm:
            self._active_hm = value
            self.active_hm_changed.emit()

    def _set_active_key(self, value: str) -> None:
        if value != self._active_key:
            self._active_key = value
            self.active_key_changed.emit()

    def set_peers(
        self,
        updater: QObject,
        settings_bridge: QObject,
        uninstaller: QObject,
        launch: QObject,
        shears: QObject,
    ) -> None:
        self._updater = updater
        self._settings_bridge = settings_bridge
        self._uninstaller = uninstaller
        self._shears = shears
        updater.changed.connect(self._start_next)
        settings_bridge.steam_account_changed.connect(self._start_next)
        settings_bridge.cache_cleared.connect(self._start_next)
        uninstaller.done.connect(lambda *_: self._start_next())
        shears.cut_done.connect(lambda *_: self._start_next())
        launch.running_changed.connect(lambda *_: self._start_next())
        QTimer.singleShot(0, self._start_next)

    def _peer_blocker(self, season_key: str) -> str | None:
        if self._uninstaller.busy_key() == season_key:
            return UNINSTALL_RUNNING
        if self._shears.busy_key() == season_key:
            return "Shears is running"
        if self._updater.busy:
            return UPDATE_RUNNING
        if self._settings_bridge.clearing_cache():
            return CACHE_CLEARING
        return None

    def _conflicts(self, season_key: str) -> bool:
        blocker = self._peer_blocker(season_key)
        if blocker is None:
            return False
        self.error.emit(blocker)
        return True

    def _find_download(self, season_key: str) -> dict | None:
        return next((d for d in self._downloads if d["key"] == season_key), None)

    def _accept(self, season_key: str, resume: PendingRequest) -> dict | None:
        if self._blocked(season_key):
            return None
        download = self._find_download(season_key)
        if download is None:
            self.error.emit("Unknown download")
            return None
        steam_account = get_setting(self._settings, "steam_account", "")
        if not steam_account or not NAME_PATTERN.match(steam_account):
            self._pending_request = resume
            self._login_kind = "account"
            self.login_required.emit("account")
            return None
        self._steam_account = steam_account
        return download

    @deferred_slot(str, bool, str)
    def start(self, season_key: str, enable_hm: bool, library: str) -> None:
        self._start(season_key, enable_hm, library)

    @deferred_slot(str, bool, str)
    def enqueue(self, season_key: str, enable_hm: bool, library: str) -> None:
        if not self.running:
            self._start(season_key, enable_hm, library)
            return
        if self._active_or_queued(season_key, enable_hm):
            return
        if installed_path(season_key, enable_hm) is not None:
            self.error.emit(ALREADY_INSTALLED)
            return
        self._queue.append(QueueEntry(season_key, enable_hm, library, False))
        self._queue_updated()

    @Slot(str, bool)
    def dequeue(self, season_key: str, hm: bool) -> None:
        remaining = [item for item in self._queue if item.key != season_key or item.hm != hm]
        if len(remaining) != len(self._queue):
            self._queue = remaining
            self._queue_updated()

    def queued_items(self) -> list:
        return [{"key": item.key, "hm": item.hm, "verify": item.verify} for item in self._queue]

    @Slot("QVariantList")
    def reorder_queue(self, entries: list) -> None:
        refs = [
            (str(entry["key"]), bool(entry["hm"]))
            for entry in list(entries)
            if isinstance(entry, dict)
        ]
        by_ref = {(item.key, item.hm): item for item in self._queue}
        reordered = [by_ref.pop(ref) for ref in refs if ref in by_ref]
        reordered.extend(by_ref.values())
        if reordered != self._queue:
            self._queue = reordered
            self._queue_updated()

    @deferred_slot(bool)
    def set_paused(self, value: bool) -> None:
        self._set_paused(value)

    def _enter_paused_from_queue(self) -> bool:
        index = next((i for i, item in enumerate(self._queue) if not item.verify), None)
        if index is None:
            return False
        key, hm, library, _ = self._queue.pop(index)
        self._set_active_key(key)
        self._set_active_hm(hm)
        partial, root = self._resolve_root(key, hm, library)
        self._target = partial if partial is not None else root / edition_folder(key, hm)
        self._set_progress(0.0)
        return True

    def _set_paused(self, value: bool) -> None:
        if value == self._paused:
            return
        if value:
            self._paused = True
            set_setting(self._settings, "queue_paused", True)
            if self.running:
                save_settings(self._settings)
                if not self._verifying:
                    self.cancel()
            elif self._state != "paused" and self._enter_paused_from_queue():
                self._set_state("paused")
                self._queue_updated()
            else:
                save_settings(self._settings)
        elif self._state == "paused" and self._active_key:
            self._start(self._active_key, self._active_hm, str(self._target.parent))
        else:
            self._clear_paused_flag()
            self._start_next()

    def _queue_updated(self) -> None:
        self._persist_queue()
        self.queue_changed.emit()

    def _persist_queue(self) -> None:
        entries = list(self._queue)
        active = (self.running or self._state == "paused") and self._active_key
        request = self._pending_request
        if (
            request is not None
            and request.kind == "disk"
            and (not active or (request.key, request.hm) != (self._active_key, self._active_hm))
        ):
            entries.insert(0, QueueEntry(request.key, request.hm, request.library, False))
        if active:
            entries.insert(
                0,
                QueueEntry(
                    self._active_key, self._active_hm, str(self._target.parent), self._verifying
                ),
            )
        set_setting(
            self._settings,
            "queue",
            [
                {"key": key, "hm": hm, "library": library, "verify": verify}
                for key, hm, library, verify in entries
            ],
        )
        save_settings(self._settings)

    def _active_or_queued(self, key: str, hm: bool) -> bool:
        return (key, hm) == (self._active_key, self._active_hm) or any(
            item.key == key and item.hm == hm for item in self._queue
        )

    def busy_with(self, key: str) -> bool:
        return self.running and self._active_key == key

    def uses_library(self, root: Path) -> bool:
        target = Path(root).resolve()
        if (
            (self.running or self._state == "paused")
            and self._active_key
            and self._target.parent.resolve() == target
        ):
            return True
        return any(self._queued_root(*item) == target for item in self._queue)

    def _resolve_root(self, key: str, hm: bool, library: str) -> tuple[Path | None, Path]:
        partial = partial_path(key, hm)
        if partial is not None:
            return partial, partial.parent
        return None, (Path(library).resolve() if library else default_library())

    def _queued_root(self, key: str, hm: bool, library: str, verify: bool) -> Path | None:
        if verify:
            installed = installed_path(key, hm)
            return installed.parent.resolve() if installed is not None else None
        return self._resolve_root(key, hm, library)[1].resolve()

    def _start_next(self) -> None:
        if self._paused:
            return
        account = get_setting(self._settings, "steam_account", "")
        if not account or not NAME_PATTERN.match(account):
            return
        while (
            self._queue and not self.running and not self._picking and self._pending_request is None
        ):
            season_key, enable_hm, library, verify = self._queue[0]
            if (
                season_key == self._deleting_key
                or self._peer_blocker(season_key) is not None
                or (verify and is_season_running(season_key))
            ):
                return
            self._queue.pop(0)
            self._queue_updated()
            if verify:
                self._verify(season_key, enable_hm)
            else:
                self._start(season_key, enable_hm, library)

    def _start(
        self, season_key: str, enable_hm: bool, library: str, skip_disk_check: bool = False
    ) -> None:
        self.dequeue(season_key, enable_hm)
        download = self._accept(season_key, PendingRequest("start", season_key, enable_hm, library))
        if download is None:
            return
        partial, root = self._resolve_root(season_key, enable_hm, library)
        if partial is not None:
            target = partial
            if library and Path(library).resolve() != root:
                self.error.emit(PARTIAL_EXISTS)
                return
        else:
            if installed_path(season_key, enable_hm) is not None:
                self.error.emit(ALREADY_INSTALLED)
                return
            if root not in libraries():
                self.error.emit("Unknown library")
                return
            if not root.exists():
                self.error.emit(FOLDER_NOT_FOUND)
                return
            target = root / edition_folder(season_key, enable_hm)
        if enable_hm and not self._ensure_hm_archive(
            download, lambda: self._start(season_key, enable_hm, library, skip_disk_check)
        ):
            return
        if not skip_disk_check:
            present = folder_size(target) if partial is not None else 0
            missing = max(download["size_gb"] * GIB - present, 0)
            required = missing + _STAGING_HEADROOM
            free = shutil.disk_usage(target.parent).free
            if free < required:
                shortfall = math.ceil((required - free) / GIB * 10) / 10
                self._pending_request = PendingRequest(
                    "disk", season_key, enable_hm, library, shortfall
                )
                self._persist_queue()
                self.disk_space_required.emit(shortfall)
                return
        self._start_run(download, season_key, target, enable_hm, verify=False)

    @Slot(str, bool)
    def delete_partial(self, season_key: str, hm: bool) -> None:
        if self.running and (season_key, hm) == (self._active_key, self._active_hm):
            self.partial_deleted.emit(season_key, hm, False, BUSY_MESSAGE)
            return
        if self._deleting_key is not None:
            self.partial_deleted.emit(season_key, hm, False, REMOVING_FILES)
            return
        blocker = self._peer_blocker(season_key)
        if blocker is not None:
            self.partial_deleted.emit(season_key, hm, False, blocker)
            return
        if is_season_running(season_key):
            self.partial_deleted.emit(season_key, hm, False, SEASON_RUNNING)
            return
        folder = edition_folder(season_key, hm)
        targets = [
            path
            for root in libraries()
            if (path := root / folder).is_dir() and not is_installed(path)
        ]
        code = season_code(season_key)
        removed_message = f"{code} download removed"
        if not targets:
            if not self.running and (season_key, hm) == (self._active_key, self._active_hm):
                self._on_deleted_in(season_key, hm, True, removed_message)
            else:
                self.partial_deleted.emit(season_key, hm, False, "Nothing to remove")
            return
        self._deleting_key = season_key

        def work() -> None:
            try:
                for path in targets:
                    shutil.rmtree(path)
            except OSError as e:
                self._deleted_in.emit(season_key, hm, False, log.fail("Remove failed", e))
            else:
                self._deleted_in.emit(season_key, hm, True, removed_message)

        threading.Thread(target=work, daemon=True).start()

    def _on_deleted_in(self, season_key: str, hm: bool, ok: bool, message: str) -> None:
        self._deleting_key = None
        if ok and not self.running and (season_key, hm) == (self._active_key, self._active_hm):
            if self._paused and self._enter_paused_from_queue():
                self._set_state("paused")
            else:
                self._set_active_key("")
                self._set_state("idle")
                self._clear_paused_flag()
            self._queue_updated()
        self.partial_deleted.emit(season_key, hm, ok, message)
        self._start_next()

    @deferred_slot(str, bool)
    def verify(self, season_key: str, hm: bool) -> None:
        self._verify(season_key, hm)

    def _verify(self, season_key: str, hm: bool) -> None:
        if self.running:
            if self._active_or_queued(season_key, hm):
                return
            if installed_path(season_key, hm) is None:
                self.error.emit(NOT_INSTALLED)
                return
            self._queue.append(QueueEntry(season_key, hm, "", True))
            self._queue_updated()
            return
        download = self._accept(season_key, PendingRequest("verify", season_key, hm))
        if download is None:
            return
        target = installed_path(season_key, hm)
        if target is None:
            self.error.emit(NOT_INSTALLED)
            return
        if is_season_running(season_key):
            self.error.emit(SEASON_RUNNING)
            return
        if hm and not self._ensure_hm_archive(download, lambda: self._verify(season_key, hm)):
            return
        self._start_run(download, season_key, target, hm, verify=True)

    def _clear_paused_flag(self, save: bool = True) -> None:
        if self._paused:
            self._paused = False
            set_setting(self._settings, "queue_paused", False)
            if save:
                save_settings(self._settings)

    def _requeue_paused(self, season_key: str, enable_hm: bool) -> None:
        if (
            self._state == "paused"
            and self._active_key
            and (self._active_key, self._active_hm) != (season_key, enable_hm)
        ):
            self._queue.insert(
                0,
                QueueEntry(self._active_key, self._active_hm, str(self._target.parent), False),
            )
            self.queue_changed.emit()
        self._clear_paused_flag()

    def _pick_hm_archive(self, resume: Callable[[], None]) -> None:
        self._picking = True
        try:
            picked = pick_file("Choose the Heated Metal archive", ".7z archive (*.7z)")
        finally:
            self._picking = False
        if not picked:
            return
        self._picking = True
        self._hm_resume = resume

        def work() -> None:
            try:
                cached = cache_hm_archive(Path(picked))
            except OSError as e:
                self._hm_cached_in.emit("", log.fail("Could not store the Heated Metal archive", e))
            else:
                self._hm_cached_in.emit(str(cached), "")

        threading.Thread(target=work, daemon=True).start()

    def _on_hm_cached_in(self, cached: str, err: str) -> None:
        self._picking = False
        resume = self._hm_resume
        self._hm_resume = None
        if err:
            self.error.emit(err)
        else:
            self._picked_archive = Path(cached)
            set_setting(self._settings, "hm_archive", self._picked_archive.name)
            save_settings(self._settings)
            if resume is not None:
                resume()
        self._start_next()

    def _ensure_hm_archive(self, download: dict, resume: Callable[[], None]) -> bool:
        if not download.get("hm_beta") or self._picked_archive is not None:
            return True
        cached = BIN_DIR / Path(str(get_setting(self._settings, "hm_archive", ""))).name
        if cached.is_file():
            self._picked_archive = cached
            return True
        self._pick_hm_archive(resume)
        return False

    def _begin_run(
        self,
        download: dict,
        season_key: str,
        target: Path,
        enable_hm: bool,
        verify: bool,
        state: str,
    ) -> None:
        self._generation += 1
        self._download = download
        self._switching = ""
        self._set_active_hm(enable_hm)
        self._set_verifying(verify)
        self._target = target
        self._archive = self._picked_archive
        self._picked_archive = None
        self._cancelled = False
        self._rate_limit_hit = False
        self._history.clear()
        self._set_active_key(season_key)
        self._set_progress(0.0)
        self._set_step(0, 0)
        self._set_state(state)
        self._persist_queue()

    def _start_run(
        self,
        download: dict,
        season_key: str,
        target: Path,
        enable_hm: bool,
        verify: bool,
        pre: Callable[[], None] | None = None,
    ) -> None:
        self._requeue_paused(season_key, enable_hm)
        value = get_setting(self._settings, "max_downloads", DEFAULT_MAX_DOWNLOADS)
        value = int(value) if isinstance(value, int) else DEFAULT_MAX_DOWNLOADS
        self._max_downloads = min(max(value, DOWNLOADS_MIN), DOWNLOADS_MAX)
        self._begin_run(download, season_key, target, enable_hm, verify, "preparing")
        threading.Thread(
            target=self._prepare,
            args=(self._generation, enable_hm, pre),
            daemon=True,
        ).start()

    def _prepare(self, generation: int, enable_hm: bool, pre: Callable[[], None] | None) -> None:
        try:
            if pre is not None:
                pre()
        except OSError as e:
            self._prepare_done_in.emit(generation, "", str(e))
            return
        except Exception as e:
            self._prepare_done_in.emit(generation, "", log.fail("Setup failed", e))
            return
        try:
            dd = ensure_depotdownloader()
            if not enable_hm:
                ensure_tl()
        except RateLimitError as e:
            self._rate_limited_in.emit(e.message())
            self._prepare_done_in.emit(generation, "", str(e))
        except OSError as e:
            self._prepare_done_in.emit(generation, "", str(e))
        else:
            self._prepare_done_in.emit(generation, str(dd), "")

    def _on_prepare_done_in(self, generation: int, dd: str, err: str) -> None:
        if generation != self._generation:
            return
        if self._cancelled:
            self._end_run(1)
            return
        if err:
            if self._rate_limit_hit:
                self._cancelled = True
                self._set_paused(True)
            else:
                self.error.emit(err)
            self._end_run(1)
            return
        self._dd = dd
        try:
            self._commands = depot_commands(
                self._download, self._steam_account, self._target, self._max_downloads
            )
            self._target.mkdir(parents=True, exist_ok=True)
        except Exception as e:
            self.error.emit(log.fail("Setup failed", e))
            self._end_run(1)
            return
        self._set_state("downloading")
        self._index = 0
        self._run_next()

    def _run_next(self) -> None:
        if self._index >= len(self._commands):
            self._set_progress(100.0)
            self._start_apply()
            return
        cmd = self._commands[self._index]
        self._set_step(self._index + 1, len(self._commands))
        self._set_progress(0.0)
        self._buffer = ""
        self._login_pending = False
        self._proc_done = False
        proc = QProcess(self)
        proc.setProcessChannelMode(QProcess.ProcessChannelMode.MergedChannels)
        proc.readyReadStandardOutput.connect(self._on_output)
        proc.finished.connect(self._on_process_done)
        proc.errorOccurred.connect(self._on_process_error)
        self._process = proc
        proc.start(self._dd, cmd["args"])

    def _on_output(self) -> None:
        proc = self.sender()
        if proc is None or proc is not self._process:
            return
        self._buffer += bytes(proc.readAllStandardOutput()).decode("utf-8", errors="replace")
        *split, self._buffer = self._buffer.split("\n")
        lines = [line.rstrip("\r") for line in split]
        for line in lines:
            match = _PERCENT.match(line)
            if match:
                depot_pct = min(float(match.group(1)), 100.0)
                if depot_pct - self._progress >= 0.1:
                    self._set_progress(depot_pct)
        if lines:
            self.log_line.emit("\n".join(lines))
        self._check_prompt(self._buffer)

    def _check_prompt(self, text: str) -> None:
        if self._login_pending:
            return
        stripped = text.rstrip()
        if not stripped.endswith(":"):
            return
        low = stripped.lower()
        if any(token in low for token in ("auth code", "authentication code", "steam guard")):
            kind = "guard_email" if "email" in low else "guard"
        elif "password" in low:
            kind = "password"
        else:
            return
        self._login_pending = True
        if kind == "password" and self._pending_password and self._process is not None:
            password = self._pending_password
            self._pending_password = ""
            self._login_pending = False
            self._process.write((password + "\n").encode())
            return
        self._login_kind = kind
        self.login_required.emit(kind)

    @Slot(str)
    def submit_login(self, text: str) -> None:
        if self._process is None or not self._login_pending:
            return
        self._login_pending = False
        self._login_kind = ""
        self._process.write((text + "\n").encode())

    @deferred_slot(str, str)
    def submit_account_login(self, account: str, password: str) -> None:
        account = account.strip()
        if not account or not NAME_PATTERN.match(account):
            self.error.emit("Invalid Steam account")
            return
        self._login_kind = ""
        self._settings_bridge.set_steam_account(account)
        self._pending_password = password
        request = self._pending_request
        self._pending_request = None
        if request is None:
            return
        if request.kind == "verify":
            self._verify(request.key, request.hm)
        elif request.kind == "remove_hm":
            self.remove_hm(request.key)
        else:
            self._start(request.key, request.hm, request.library)

    @deferred_slot()
    def confirm_disk_space(self) -> None:
        request = self._pending_request
        if request is None or request.kind != "disk":
            return
        self._pending_request = None
        self._start(request.key, request.hm, request.library, skip_disk_check=True)

    def _on_process_done(self, code: int, status: QProcess.ExitStatus) -> None:
        proc = self.sender()
        if proc is not self._process:
            if isinstance(proc, QProcess):
                proc.deleteLater()
            return
        if self._proc_done:
            return
        self._proc_done = True
        self._process = None
        proc.deleteLater()
        if self._cancelled:
            self._end_run(1)
            return
        cmd = self._commands[self._index]
        if status == QProcess.ExitStatus.CrashExit:
            self.error.emit(
                log.fail(
                    "DepotDownloader stopped unexpectedly",
                    f"{cmd['name']}\n{self._history_tail()}",
                )
            )
            self._end_run(1)
            return
        if code != 0:
            if not cmd["optional"]:
                self.error.emit(
                    log.fail(f"{cmd['name']} depot failed, exit code {code}", self._history_tail())
                )
                self._end_run(code)
                return
            self.warning.emit(
                log.fail(
                    f"{cmd['name']} depot failed", f"optional, exit={code}\n{self._history_tail()}"
                )
            )
        self._index += 1
        self._run_next()

    def _history_tail(self) -> str:
        lines = [
            line
            for line in "\n".join(self._history).splitlines()
            if not line.lstrip().startswith(("at ", "--- End of"))
        ]
        return "\n".join(lines[-8:]) if lines else "no output"

    def _on_process_error(self, err: QProcess.ProcessError) -> None:
        proc = self.sender()
        if proc is not self._process or self._proc_done:
            return
        if err != QProcess.ProcessError.FailedToStart:
            return
        self._proc_done = True
        self._process = None
        proc.deleteLater()
        self.error.emit(
            log.fail("DepotDownloader failed to start", f"{self._dd}: {proc.errorString()}")
        )
        self._end_run(1)

    def _start_apply(self) -> None:
        self._set_step(0, 0)
        self._set_state("applying")
        username = effective_username(self._settings)
        threading.Thread(
            target=self._apply,
            args=(
                self._generation,
                self._target,
                self._download,
                self._active_hm,
                username,
                self._verifying,
                self._archive,
            ),
            daemon=True,
        ).start()

    def _apply(
        self,
        generation: int,
        target: Path,
        download: dict,
        is_hm: bool,
        username: str,
        verify: bool,
        archive: Path | None,
        pre: Callable[[], None] | None = None,
        fail_label: str = "Install failed",
    ) -> None:
        reporter = SignalReporter(fail_emit=self._error_in.emit)
        try:
            if pre is not None:
                pre()
            ok = _apply_install(
                target,
                download,
                is_hm,
                username,
                reporter=reporter,
                archive=archive,
            )
        except RateLimitError as e:
            self._rate_limited_in.emit(e.message())
            ok = False
        except Exception as e:
            self._error_in.emit(log.fail(fail_label, e))
            ok = False
        self._apply_done_in.emit(generation, ok)

    def _on_apply_done_in(self, generation: int, ok: bool) -> None:
        if generation != self._generation:
            return
        if ok:
            self._cancelled = False
            self._end_run(0)
        else:
            self._end_run(1)

    def _blocked(self, season_key: str) -> bool:
        if self.running:
            self.error.emit(DOWNLOAD_RUNNING)
            return True
        if self._picking or self._pending_request is not None:
            return True
        if self._deleting_key == season_key:
            self.error.emit(REMOVING_FILES)
            return True
        return self._conflicts(season_key)

    @deferred_slot(str)
    def switch_to_hm(self, season_key: str) -> None:
        self._switch_to_hm(season_key)

    def _switch_to_hm(self, season_key: str) -> None:
        if self._blocked(season_key):
            return
        folder = installed_path(season_key, False)
        if folder is None:
            self.error.emit("Not installed as Throwback")
            return
        download = self._find_download(season_key)
        if download is None or not download.get("hm"):
            self.error.emit("Heated Metal is not available for this season")
            return
        if installed_path(season_key, True) is not None:
            self.error.emit("Heated Metal is already installed")
            return
        if partial_path(season_key, True) is not None:
            self.error.emit(PARTIAL_EXISTS)
            return
        if is_season_running(season_key):
            self.error.emit(SEASON_RUNNING)
            return
        if not self._ensure_hm_archive(download, lambda: self._switch_to_hm(season_key)):
            return
        self.dequeue(season_key, True)
        self._requeue_paused(season_key, True)
        target = folder.parent / edition_folder(season_key, True)
        self._begin_run(download, season_key, target, True, False, "applying")
        self._switching = "hm"
        threading.Thread(
            target=self._switch_worker,
            args=(self._generation, folder, self._target, download, self._archive),
            daemon=True,
        ).start()

    def _switch_worker(
        self, generation: int, folder: Path, target: Path, download: dict, archive: Path | None
    ) -> None:
        def move_to_hm() -> None:
            folder.rename(target)
            for name in TL_EXTRACT:
                (target / name).unlink(missing_ok=True)

        self._apply(
            generation,
            target,
            download,
            True,
            effective_username(self._settings),
            False,
            archive,
            pre=move_to_hm,
            fail_label="Switch failed",
        )

    @Slot(str)
    def remove_hm(self, season_key: str) -> None:
        download = self._accept(season_key, PendingRequest("remove_hm", season_key))
        if download is None:
            return
        folder = installed_path(season_key, True)
        if folder is None:
            self.error.emit(NOT_INSTALLED_HM)
            return
        if installed_path(season_key, False) is not None:
            self.error.emit("Already installed as Throwback")
            return
        if partial_path(season_key, False) is not None:
            self.error.emit(PARTIAL_EXISTS)
            return
        if is_season_running(season_key):
            self.error.emit(SEASON_RUNNING)
            return
        self.dequeue(season_key, False)
        target = folder.parent / edition_folder(season_key, False)

        def pre() -> None:
            try:
                folder.rename(target)
                (target / TL_LAUNCHER).unlink(missing_ok=True)
                remove_hm_files(target)
            except OSError as e:
                raise OSError(log.fail("Switch failed", e)) from e

        self._start_run(download, season_key, target, False, verify=True, pre=pre)
        self._switching = "tb"

    @deferred_slot(str)
    def import_hm(self, season_key: str) -> None:
        if self._blocked(season_key):
            return
        target = installed_path(season_key, True)
        if target is None:
            self.error.emit(NOT_INSTALLED_HM)
            return
        download = self._find_download(season_key)
        if download is None or not download.get("hm_beta"):
            self.error.emit("Heated Metal beta is not available for this season")
            return
        if is_season_running(season_key):
            self.error.emit(SEASON_RUNNING)
            return

        def proceed() -> None:
            if is_season_running(season_key):
                self.error.emit(SEASON_RUNNING)
                return
            current = installed_path(season_key, True)
            if current is None:
                self.error.emit(NOT_INSTALLED_HM)
                return
            self._requeue_paused(season_key, True)
            self._begin_run(download, season_key, current, True, False, "applying")
            self._start_apply()

        self._pick_hm_archive(proceed)

    @Slot()
    def cancel(self) -> None:
        if not self.running:
            request = self._pending_request
            self._pending_request = None
            self._login_kind = ""
            self._pending_password = ""
            self._picked_archive = None
            if request is not None and request.kind == "disk":
                self._persist_queue()
                QTimer.singleShot(0, self._start_next)
            return
        self._cancelled = True
        proc = self._process
        if proc is not None and proc.state() != QProcess.ProcessState.NotRunning:
            proc.kill()

    def shutdown(self) -> None:
        proc = self._process
        if proc is not None and proc.state() != QProcess.ProcessState.NotRunning:
            self._proc_done = True
            proc.kill()
            proc.waitForFinished(_SHUTDOWN_WAIT_MS)

    def _on_rate_limited_in(self, message: str) -> None:
        self._rate_limit_hit = True
        self.rate_limited.emit(message)

    @Slot()
    def request_log(self) -> None:
        self.log_history.emit(self._active_key, "\n".join(self._history))

    @Slot(result="QVariantMap")
    def snapshot(self) -> dict:
        request = self._pending_request
        return {
            "state": self._state,
            "progress": self._progress,
            "step": self._step,
            "steps": self._steps,
            "activeKey": self._active_key,
            "activeHm": self._active_hm,
            "verifying": self._verifying,
            "loginKind": self._login_kind,
            "diskShortfall": request.shortfall if request is not None else 0.0,
            "queue": self.queued_items(),
        }

    def _end_run(self, code: int) -> None:
        if self._process is not None:
            self._process.deleteLater()
            self._process = None
        self._login_pending = False
        self._login_kind = ""
        self._pending_password = ""
        self._pending_request = None
        self._buffer = ""
        done_key = self._active_key
        verifying = self._verifying
        self._set_verifying(False)
        if self._cancelled:
            outcome = ""
            keep_active = bool(self._active_key) and not verifying
            if self._paused and (keep_active or self._enter_paused_from_queue()):
                self._set_state("paused")
                if not keep_active:
                    self.queue_changed.emit()
            else:
                self._clear_paused_flag(save=False)
                self._set_state("idle")
                self._set_active_key("")
        elif code != 0:
            outcome = (
                "rate_limited"
                if self._rate_limit_hit
                else "no_space"
                if _out_of_space(self._target.parent)
                else "verify_failed"
                if verifying
                else "failed"
            )
            self._set_state("failed")
        else:
            outcome = (
                f"switched_{self._switching}"
                if self._switching
                else "verified"
                if verifying
                else "done"
            )
            if self._paused and self._enter_paused_from_queue():
                self._set_state("paused")
                self.queue_changed.emit()
            else:
                self._set_state("idle")
                self._clear_paused_flag(save=False)
        self._persist_queue()
        self.done.emit(done_key, outcome)
        if self._queue:
            QTimer.singleShot(0, self._start_next)
