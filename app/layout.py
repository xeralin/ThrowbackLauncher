import os
import sys
from pathlib import Path

VERSION = "0.1.1"

APP_NAME = "Throwback Launcher"
DIR_NAME = "ThrowbackLauncher"
OWNER = "xeralin"
REPO = f"{OWNER}/ThrowbackLauncher"
RUNTIME_ASSET = "Runtime.zip"
APPIMAGE_ASSET = "ThrowbackLauncher.AppImage"
EXE_NAME = "ThrowbackLauncher.exe"
APP_SUBDIR = "app"
PENDING_SUBDIR = "app.pending"
PREVIOUS_SUBDIR = "app.previous"
PENDING_FILE = ".pending"
ATTEMPTED_FILE = ".attempted"
UNINSTALL_KEY = rf"Software\Microsoft\Windows\CurrentVersion\Uninstall\{DIR_NAME}"


def user_data_base() -> Path:
    if sys.platform.startswith("win"):
        return Path(os.environ.get("LOCALAPPDATA") or str(Path.home() / "AppData" / "Local"))
    return Path(os.environ.get("XDG_DATA_HOME") or str(Path.home() / ".local" / "share"))


def start_menu_shortcut(appdata: str) -> Path:
    return Path(appdata) / "Microsoft" / "Windows" / "Start Menu" / "Programs" / f"{APP_NAME}.lnk"
