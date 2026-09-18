import ctypes
import os
import subprocess

NOWINDOW = getattr(subprocess, "CREATE_NO_WINDOW", 0)
_BREAKAWAY = getattr(subprocess, "CREATE_BREAKAWAY_FROM_JOB", 0)


def spawn_detached(argv: list[str]) -> None:
    kw = {
        "cwd": os.environ.get("SYSTEMROOT", r"C:\Windows"),
        "close_fds": True,
        "stdin": subprocess.DEVNULL,
        "stdout": subprocess.DEVNULL,
        "stderr": subprocess.DEVNULL,
    }
    try:
        proc = subprocess.Popen(argv, creationflags=NOWINDOW | _BREAKAWAY, **kw)
    except OSError:
        proc = subprocess.Popen(argv, creationflags=NOWINDOW, **kw)
    ctypes.windll.user32.AllowSetForegroundWindow(proc.pid)
