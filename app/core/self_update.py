import contextlib
import ctypes
import functools
import os
import shutil
import sys
import time
from pathlib import Path

from core import log
from core.constants import DATA_ROOT, FROZEN, INSTANCE_KEY, IS_WINDOWS, version_tuple
from core.winspawn import spawn_detached
from layout import (
    APP_SUBDIR,
    ATTEMPTED_FILE,
    EXE_NAME,
    PENDING_FILE,
    PENDING_SUBDIR,
    PREVIOUS_SUBDIR,
    UNINSTALL_KEY,
    VERSION,
)

_OUTCOME = ".update-outcome"
_POLL_INTERVAL = 0.25
_MOVE_DEADLINE_S = 60
_ROLLBACK_DEADLINE_S = 15
_PREVIOUS_CLEAR_S = 5
_WAIT_PID_MS = 30_000
_MAX_BOUNCE = 2
_MUTEX_NAME = f"Local\\{INSTANCE_KEY}.update"
_SYNCHRONIZE = 0x00100000
_WAIT_OBJECT_0 = 0
_ERROR_ALREADY_EXISTS = 183
_mutex = None


@functools.cache
def _kernel32() -> ctypes.CDLL:
    return ctypes.WinDLL("kernel32", use_last_error=True)


def _claim() -> bool:
    global _mutex
    if _mutex is not None:
        return True
    kernel32 = _kernel32()
    handle = kernel32.CreateMutexW(None, True, _MUTEX_NAME)
    if not handle:
        return False
    if ctypes.get_last_error() == _ERROR_ALREADY_EXISTS:
        kernel32.CloseHandle(handle)
        return False
    _mutex = handle
    return True


def _helper_running() -> bool:
    kernel32 = _kernel32()
    handle = kernel32.CreateMutexW(None, True, _MUTEX_NAME)
    if not handle:
        return True
    held = ctypes.get_last_error() == _ERROR_ALREADY_EXISTS
    kernel32.CloseHandle(handle)
    return held


def _paths() -> tuple[Path, Path, Path]:
    return DATA_ROOT / APP_SUBDIR, DATA_ROOT / PENDING_SUBDIR, DATA_ROOT / PREVIOUS_SUBDIR


def pending_dir() -> Path:
    return _paths()[1]


def _wait_pid(pid: int) -> bool:
    kernel32 = _kernel32()
    handle = kernel32.OpenProcess(_SYNCHRONIZE, False, pid)
    if not handle:
        return True
    result = kernel32.WaitForSingleObject(handle, _WAIT_PID_MS)
    kernel32.CloseHandle(handle)
    return result == _WAIT_OBJECT_0


def _move(src: Path, dst: Path, deadline: float) -> bool:
    while True:
        try:
            src.rename(dst)
            return True
        except OSError as e:
            if dst.exists() and not src.exists():
                return True
            if time.monotonic() > deadline:
                log.fail("App folder move failed", f"{src} -> {dst}: {e}")
                return False
            time.sleep(_POLL_INTERVAL)


def _remove(path: Path) -> None:
    with contextlib.suppress(OSError):
        if path.is_dir():
            shutil.rmtree(path, ignore_errors=True)
        else:
            path.unlink(missing_ok=True)


def _clear(path: Path, deadline: float) -> bool:
    while True:
        _remove(path)
        if not path.exists():
            return True
        if time.monotonic() > deadline:
            return False
        time.sleep(_POLL_INTERVAL)


def _pending_ok(pending: Path) -> bool:
    marker = pending / PENDING_FILE
    if not pending.is_dir() or not marker.is_file():
        return False
    try:
        version = marker.read_text(encoding="ascii").strip()
        return version_tuple(version) >= version_tuple(VERSION)
    except OSError, ValueError:
        return False


def write_outcome(ok: bool) -> None:
    with contextlib.suppress(OSError):
        (DATA_ROOT / _OUTCOME).write_text("ok" if ok else "", encoding="ascii")


def helper_argv(target: Path, bounce: int = 0) -> list[str]:
    exe = target / EXE_NAME
    return [str(exe), "--relaunch", str(os.getpid()), str(bounce)]


def maybe_apply_pending() -> bool:
    if not (IS_WINDOWS and FROZEN):
        return False
    app, pending, previous = _paths()
    if Path(sys.executable).resolve().parent != app:
        return False
    _clear(previous, time.monotonic() + _PREVIOUS_CLEAR_S)
    if not _pending_ok(pending) or not (pending / EXE_NAME).is_file():
        if pending.is_dir():
            log.fail("Pending update was discarded", pending)
        _remove(pending)
        return False
    attempted = pending / ATTEMPTED_FILE
    if attempted.exists():
        if _helper_running():
            return True
        _remove(pending)
        write_outcome(False)
        return False
    with contextlib.suppress(OSError):
        attempted.touch()
    try:
        spawn_detached(helper_argv(pending))
    except OSError as e:
        log.fail("Update helper failed to start", e)
        _remove(pending)
        write_outcome(False)
        return False
    return True


def take_outcome() -> bool | None:
    marker = DATA_ROOT / _OUTCOME
    try:
        outcome = marker.read_text(encoding="ascii")
    except OSError:
        return None
    with contextlib.suppress(OSError):
        marker.unlink()
    return outcome == "ok"


def _flip(app: Path, pending: Path, previous: Path, bounce: int) -> None:
    deadline = time.monotonic() + _MOVE_DEADLINE_S
    if not app.is_dir():
        if bounce < _MAX_BOUNCE and _pending_ok(pending):
            with contextlib.suppress(OSError):
                spawn_detached(helper_argv(previous, bounce + 1))
                return
        if _move(previous, app, deadline):
            write_outcome(False)
            with contextlib.suppress(OSError):
                spawn_detached([str(app / EXE_NAME)])
        return
    if _pending_ok(pending) and _clear(previous, deadline) and _move(app, previous, deadline):
        try:
            spawn_detached(helper_argv(previous, bounce))
            return
        except OSError:
            if not _move(previous, app, time.monotonic() + _ROLLBACK_DEADLINE_S):
                return
    with contextlib.suppress(OSError):
        (pending / PENDING_FILE).unlink(missing_ok=True)
    write_outcome(False)
    with contextlib.suppress(OSError):
        spawn_detached([str(app / EXE_NAME)])


def _swap_in(app: Path, pending: Path, bounce: int) -> None:
    if not _move(pending, app, time.monotonic() + _MOVE_DEADLINE_S):
        with contextlib.suppress(OSError):
            (pending / PENDING_FILE).unlink(missing_ok=True)
        write_outcome(False)
        with contextlib.suppress(OSError):
            spawn_detached(helper_argv(pending, bounce + 1))
        return
    for name in (PENDING_FILE, ATTEMPTED_FILE):
        with contextlib.suppress(OSError):
            (app / name).unlink(missing_ok=True)
    write_outcome(True)
    with contextlib.suppress(OSError):
        spawn_detached([str(app / EXE_NAME)])


def refresh_registry() -> None:
    if not (IS_WINDOWS and FROZEN):
        return
    import winreg

    app = _paths()[0]
    if Path(sys.executable).resolve().parent != app:
        return
    with (
        contextlib.suppress(OSError),
        winreg.OpenKey(
            winreg.HKEY_CURRENT_USER,
            UNINSTALL_KEY,
            0,
            winreg.KEY_QUERY_VALUE | winreg.KEY_SET_VALUE,
        ) as k,
    ):
        with contextlib.suppress(OSError):
            location, _ = winreg.QueryValueEx(k, "InstallLocation")
            if location and Path(location).resolve() != DATA_ROOT:
                return
        with contextlib.suppress(OSError):
            value, _ = winreg.QueryValueEx(k, "DisplayVersion")
            if value == VERSION:
                return
        size_kb = sum(f.stat().st_size for f in app.rglob("*") if f.is_file()) // 1024
        winreg.SetValueEx(k, "DisplayVersion", 0, winreg.REG_SZ, VERSION)
        winreg.SetValueEx(k, "EstimatedSize", 0, winreg.REG_DWORD, size_kb)


def run_relaunch(argv: list[str]) -> int:
    if not (IS_WINDOWS and FROZEN):
        return 0
    app, pending, previous = _paths()
    here = Path(sys.executable).resolve().parent
    relaunch = [str(app / EXE_NAME)]
    index = argv.index("--relaunch")
    with contextlib.suppress(ValueError, IndexError):
        pid = int(argv[index + 1])
        if not _wait_pid(pid):
            with contextlib.suppress(OSError):
                spawn_detached(relaunch)
            return 0
    bounce = 0
    with contextlib.suppress(ValueError, IndexError):
        bounce = int(argv[index + 2])
    if not _claim():
        return 0
    if here == pending:
        _flip(app, pending, previous, bounce)
    elif here == previous:
        _swap_in(app, pending, bounce)
    else:
        with contextlib.suppress(OSError):
            spawn_detached(relaunch)
    return 0
