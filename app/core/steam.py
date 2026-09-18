import contextlib
import os
import re
import shlex
import shutil
from collections.abc import Iterator
from pathlib import Path

import psutil

from core import log
from core.constants import (
    IS_WINDOWS,
    NOT_INSTALLED,
    PREFIX_DIR,
    PROTON_BUILTIN,
    PROTON_DIR,
    SEASON_RUNNING,
    STEAM_DIR,
    STEAM_ROOTS,
)
from core.manifest import edition_folder, hm_folder_name, is_installed
from core.settings import get_setting, libraries

_GAME_PROC_RE = re.compile(r"(?:RainbowSix.*|LaunchR6)\.exe")
_WINE_ROOT_RE = re.compile(r"^[Zz]:\\")
_DEFAULT_ORDER = tuple(internal for _, internal, _ in PROTON_BUILTIN)


def _proc_environ(pid: int) -> dict[str, str]:
    try:
        return dict(psutil.Process(pid).environ())
    except psutil.Error, OSError:
        return {}


def _proc_cwd(pid: int) -> Path | None:
    try:
        return Path(psutil.Process(pid).cwd())
    except psutil.Error, OSError:
        return None


def _game_candidates() -> Iterator[int]:
    if IS_WINDOWS:
        for proc in psutil.process_iter(["name"]):
            if _GAME_PROC_RE.search(proc.info["name"] or ""):
                yield proc.pid
        return
    for proc in psutil.process_iter(["name", "cmdline"]):
        cmdline = proc.info["cmdline"] or []
        haystack = " ".join(cmdline) if cmdline else (proc.info["name"] or "")
        if _GAME_PROC_RE.search(haystack):
            yield proc.pid


def running_game_env() -> dict[str, str] | None:
    for pid in _game_candidates():
        env = _proc_environ(pid)
        if env.get("STEAM_COMPAT_DATA_PATH"):
            return env
    return None


def _game_exe_path(pid: int) -> Path | None:
    try:
        argv = psutil.Process(pid).cmdline()
    except psutil.Error, OSError:
        return None
    for arg in argv:
        if not _GAME_PROC_RE.search(arg):
            continue
        if not IS_WINDOWS and _WINE_ROOT_RE.match(arg):
            arg = "/" + arg[3:].replace("\\", "/")
        return Path(arg)
    return None


def _game_folder(pid: int, roots: list[Path]) -> str | None:
    for path in (_game_exe_path(pid), _proc_cwd(pid)):
        if path is None or not path.is_absolute():
            continue
        try:
            resolved = path.resolve()
        except OSError:
            continue
        for root in roots:
            try:
                rel = resolved.relative_to(root)
            except ValueError:
                continue
            if rel.parts:
                return rel.parts[0]
    return None


def running_game_folders() -> dict[str, list[int]]:
    roots = sorted(libraries(), key=lambda p: len(p.parts), reverse=True)
    found: dict[str, list[int]] = {}
    for pid in _game_candidates():
        if not IS_WINDOWS and not _proc_environ(pid).get("STEAM_COMPAT_DATA_PATH"):
            continue
        folder = _game_folder(pid, roots)
        if folder is not None:
            found.setdefault(folder, []).append(pid)
    return found


def running_game_pids() -> list[int]:
    return [pid for pids in running_game_folders().values() for pid in pids]


def is_game_running() -> bool:
    return bool(running_game_folders())


def is_season_running(key: str) -> bool:
    return not {key, hm_folder_name(key)}.isdisjoint(running_game_folders())


def prefix_pids(prefix: Path) -> list[int]:
    compat = str(prefix)
    wineprefix = str(prefix / "pfx")
    pids: list[int] = []
    for proc in psutil.process_iter(["environ"]):
        env = proc.info["environ"] or {}
        if env.get("STEAM_COMPAT_DATA_PATH") == compat or env.get("WINEPREFIX") == wineprefix:
            pids.append(proc.pid)
    return pids


def stop_game(pids: list[int]) -> None:
    procs = []
    for pid in pids:
        try:
            proc = psutil.Process(pid)
            proc.terminate()
            procs.append(proc)
        except psutil.Error:
            continue
    alive = procs
    with contextlib.suppress(psutil.Error):
        _, alive = psutil.wait_procs(procs, timeout=2)
    for proc in alive:
        with contextlib.suppress(psutil.Error):
            proc.kill()
    with contextlib.suppress(psutil.Error):
        psutil.wait_procs(alive, timeout=3)


def _proton_entry(folder: Path) -> dict | None:
    binary = folder / "proton"
    if not binary.exists():
        return None
    internal = display = folder.name
    vdf = folder / "compatibilitytool.vdf"
    if vdf.exists():
        text = vdf.read_text(errors="replace")
        m = re.search(r'"compat_tools"\s*\{\s*"([^"]+)"', text)
        if m:
            internal = m.group(1)
            display_m = re.search(r'"display_name"\s+"([^"]+)"', text)
            display = display_m.group(1) if display_m else internal
    return {"display": display, "internal": internal, "binary": binary}


def _steam_libraries(root: Path) -> list[Path]:
    roots = [root]
    with contextlib.suppress(OSError):
        text = (root / "steamapps" / "libraryfolders.vdf").read_text(errors="replace")
        for path in re.findall(r'"path"\s+"([^"]+)"', text):
            library = Path(path)
            if library not in roots:
                roots.append(library)
    return roots


def _proton_folders() -> list[Path]:
    folders: list[Path] = []
    for root in STEAM_ROOTS:
        for library in _steam_libraries(root):
            folders += sorted((library / "steamapps" / "common").glob("Proton*"))
        compat = root / "compatibilitytools.d"
        if compat.is_dir():
            folders += sorted(compat.iterdir())
    if PROTON_DIR.is_dir():
        folders += sorted(PROTON_DIR.iterdir())
    return folders


def list_protons() -> list[dict]:
    legacy = {folder: (internal, display) for folder, internal, display in PROTON_BUILTIN}
    protons: list[dict] = []
    seen: set[Path] = set()
    for folder in _proton_folders():
        entry = _proton_entry(folder)
        if entry is None:
            continue
        binary = entry["binary"].resolve()
        if binary in seen:
            continue
        seen.add(binary)
        if folder.name in legacy:
            entry["internal"], entry["display"] = legacy[folder.name]
        protons.append(entry)
    return protons


def resolve_proton(settings: dict, protons: list[dict] | None = None, key: str = "") -> dict | None:
    if protons is None:
        protons = list_protons()
    if not protons:
        return None
    per_season = get_setting(settings, "season_proton", {})
    choices = [
        per_season.get(key, "") if isinstance(per_season, dict) else "",
        get_setting(settings, "proton", ""),
    ]
    for choice in choices:
        for proton in protons:
            if proton["internal"] == choice:
                return proton
    for internal in _DEFAULT_ORDER:
        for proton in protons:
            if proton["internal"] == internal:
                return proton
    return protons[-1]


def proton_env(prefix: Path) -> dict[str, str]:
    prefix.mkdir(parents=True, exist_ok=True)
    return {
        **os.environ,
        "STEAM_COMPAT_DATA_PATH": str(prefix),
        "STEAM_COMPAT_CLIENT_INSTALL_PATH": str(STEAM_DIR),
    }


def expand_launch_args(
    settings: dict, key: str, command: list[str]
) -> tuple[list[str], dict[str, str]]:
    stored = get_setting(settings, "launch_args", {})
    raw = stored.get(key, "") if isinstance(stored, dict) else ""
    if not isinstance(raw, str):
        raw = ""
    if "%command%" not in raw:
        return [*command, *shlex.split(raw)], {}
    tokens = shlex.split(raw.replace("%command%", shlex.join(command)))
    env: dict[str, str] = {}
    while tokens and "=" in tokens[0] and tokens[0].partition("=")[0].isidentifier():
        name, _, value = tokens.pop(0).partition("=")
        env[name] = value
    return tokens, env


def prune_prefixes() -> None:
    if not PREFIX_DIR.is_dir():
        return
    roots = libraries()
    if not all(root.exists() for root in roots) or is_game_running():
        return
    for prefix in PREFIX_DIR.iterdir():
        if not prefix.is_dir():
            continue
        key = prefix.name
        if any((root / key).is_dir() or (root / hm_folder_name(key)).is_dir() for root in roots):
            continue
        shutil.rmtree(prefix, ignore_errors=True)


def uninstall(key: str, hm: bool) -> str:
    folder = edition_folder(key, hm)
    installs = [path for root in libraries() if is_installed(path := root / folder)]
    prefix = PREFIX_DIR / key
    if not installs and not prefix.exists():
        return NOT_INSTALLED
    if is_season_running(key):
        return SEASON_RUNNING
    for path in installs:
        try:
            shutil.rmtree(path)
        except OSError as e:
            return log.fail("File removal failed", e)
    other = edition_folder(key, not hm)
    if prefix.exists() and not any((root / other).is_dir() for root in libraries()):
        try:
            shutil.rmtree(prefix)
        except OSError as e:
            return log.fail("Proton prefix removal failed", e)
    return ""
