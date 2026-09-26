import contextlib
import sys
import tempfile
from pathlib import Path

from core import log
from core.constants import BIN_DIR, DEFAULT_DOWNLOADS_DIR, FROZEN, IS_WINDOWS
from core.depot import wipe_depot_token
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

    wipe_depot_token()
    bat = Path(tempfile.gettempdir()) / f"{DIR_NAME}.uninstall.bat"
    bat_dir = str(install_dir).replace("%", "%%")
    prefix = bat_dir.replace("'", "''") + "\\"
    keep = DEFAULT_DOWNLOADS_DIR.name
    kill = (
        "powershell -NoProfile -NonInteractive -Command "
        f"\"Get-Process | Where-Object {{ $_.Path -and $_.Path.StartsWith('{prefix}',"
        ' [System.StringComparison]::OrdinalIgnoreCase) } | Stop-Process -Force"'
    )
    script = (
        "@echo off\n"
        "chcp 65001>nul\n"
        "ping 127.0.0.1 -n 3 >nul\n"
        f"{kill} >nul 2>&1\n"
        "set n=0\n"
        ":retry\n"
        f'for /d %%d in ("{bat_dir}\\*") do '
        f'if /i not "%%~nxd"=="{keep}" rmdir /s /q "%%~fd" >nul 2>&1\n'
        f'del /f /q /a "{bat_dir}\\*" >nul 2>&1\n'
        f'rmdir "{bat_dir}\\{keep}" >nul 2>&1\n'
        f'rmdir "{bat_dir}" >nul 2>&1\n'
        f'if not exist "{bat_dir}\\{APP_SUBDIR}" '
        f'if not exist "{bat_dir}\\{BIN_DIR.name}" goto done\n'
        "set /a n+=1\n"
        "if %n% lss 10 (ping 127.0.0.1 -n 2 >nul & goto retry)\n"
        ":done\n"
        'del "%~f0"\n'
    )
    try:
        bat.write_text(script, encoding="utf-8")
        spawn_detached(["cmd", "/d", "/c", str(bat)])
    except OSError as e:
        log.fail("Uninstall helper failed to start", e)
