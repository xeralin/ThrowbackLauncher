import contextlib
import sys
import tempfile
from pathlib import Path

from core import log
from core.constants import FROZEN, IS_WINDOWS
from core.winspawn import spawn_detached
from layout import APP_SUBDIR, DIR_NAME, UNINSTALL_KEY, desktop_shortcut, start_menu_shortcut


def _registered_dir() -> Path | None:
    import winreg

    with contextlib.suppress(OSError), winreg.OpenKey(winreg.HKEY_CURRENT_USER, UNINSTALL_KEY) as k:
        value, _ = winreg.QueryValueEx(k, "InstallLocation")
        if value:
            return Path(value)
    return None


def run() -> None:
    if not IS_WINDOWS or not FROZEN:
        return
    import winreg

    exe_dir = Path(sys.executable).resolve().parent
    if exe_dir.name != APP_SUBDIR:
        return
    install_dir = exe_dir.parent

    registered = _registered_dir()
    if registered is None or registered == install_dir:
        if (lnk := start_menu_shortcut()) is not None:
            with contextlib.suppress(OSError):
                lnk.unlink(missing_ok=True)
        with contextlib.suppress(OSError):
            desktop_shortcut().unlink(missing_ok=True)
        with contextlib.suppress(OSError):
            winreg.DeleteKey(winreg.HKEY_CURRENT_USER, UNINSTALL_KEY)

    bat = Path(tempfile.gettempdir()) / f"{DIR_NAME}.uninstall.bat"
    bat_dir = str(install_dir).replace("%", "%%")
    prefix = bat_dir.replace("'", "''") + "\\"
    kill = (
        "powershell -NoProfile -NonInteractive -Command "
        f"\"Get-Process | Where-Object {{ $_.Path -and $_.Path.StartsWith('{prefix}',"
        ' [System.StringComparison]::OrdinalIgnoreCase) } | Stop-Process -Force"'
    )
    script = (
        "@echo off\r\n"
        "chcp 65001>nul\r\n"
        "ping 127.0.0.1 -n 3 >nul\r\n"
        f"{kill} >nul 2>&1\r\n"
        "for /l %%i in (1,1,10) do (\r\n"
        f'    rmdir /s /q "{bat_dir}" >nul 2>&1\r\n'
        f'    if not exist "{bat_dir}" goto done\r\n'
        "    ping 127.0.0.1 -n 2 >nul\r\n"
        ")\r\n"
        ":done\r\n"
        'del "%~f0"\r\n'
    )
    try:
        bat.write_text(script, encoding="utf-8")
        spawn_detached(["cmd", "/d", "/c", str(bat)])
    except OSError as e:
        log.fail("Uninstall helper failed to start", e)
