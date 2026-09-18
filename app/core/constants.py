import contextlib
import hashlib
import os
import platform
import re
import sys
from pathlib import Path

from layout import (
    APP_SUBDIR,
    DIR_NAME,
    OWNER,
    PENDING_SUBDIR,
    PREVIOUS_SUBDIR,
    REPO,
    user_data_base,
)


def version_tuple(v: str) -> tuple[int, ...]:
    return tuple(int(p) for p in v.split("."))


FROZEN = bool(getattr(sys, "frozen", False)) or "__compiled__" in globals()
IS_WINDOWS = sys.platform.startswith("win")


def _asset_root() -> Path:
    if FROZEN:
        return Path(sys.executable).resolve().parent
    return Path(__file__).resolve().parent.parent.parent


def _data_root() -> Path:
    if FROZEN and IS_WINDOWS:
        if ASSET_ROOT.name in (APP_SUBDIR, PENDING_SUBDIR, PREVIOUS_SUBDIR):
            return ASSET_ROOT.parent
        return ASSET_ROOT
    return user_data_base() / DIR_NAME


ASSET_ROOT = _asset_root()
DATA_ROOT = _data_root().resolve()
SELF_UPDATABLE = FROZEN and (
    ASSET_ROOT.name == APP_SUBDIR if IS_WINDOWS else bool(os.environ.get("APPIMAGE"))
)
_ROOT_HASH = hashlib.blake2s(str(DATA_ROOT).encode(), digest_size=4).hexdigest()
INSTANCE_KEY = f"{DIR_NAME}-{_ROOT_HASH}"

MANIFEST_FILE = ASSET_ROOT / "manifest.toml"
NEXT_OUT_DIR = ASSET_ROOT / "next" / "out"
ICON_FILE = ASSET_ROOT / "icon.png" if FROZEN else ASSET_ROOT / ".github" / "icon.png"

SETTINGS_FILE = DATA_ROOT / "settings.toml"
BIN_DIR = DATA_ROOT / "bin"
API_CACHE_FILE = BIN_DIR / "api_cache.json"
LOG_FILE = BIN_DIR / "log.txt"


DEFAULT_DOWNLOADS_DIR = DATA_ROOT / "downloads"


def _os_arch() -> tuple[str, str]:
    machine = platform.machine().lower()
    arch = "arm64" if machine in ("arm64", "aarch64") else "x64"
    return ("windows", arch) if IS_WINDOWS else ("linux", arch)


_OS, _ARCH = _os_arch()

TL_DIR = BIN_DIR / "tl"
DD_MEMBER = "DepotDownloader.exe" if IS_WINDOWS else "DepotDownloader"
DD_ASSET = f"DepotDownloader-{_OS}-{_ARCH}.zip"
DD_BIN = BIN_DIR / DD_MEMBER
DD_ZIP = BIN_DIR / DD_ASSET
DD_API_URL = "https://api.github.com/repos/SteamRE/DepotDownloader/releases/latest"

LIBERATOR_BIN = ASSET_ROOT / "Liberator.exe"

TL_API_URL = f"https://api.github.com/repos/{OWNER}/ThrowbackLoader/releases/latest"
DEFAULTARGS_DLL = "defaultargs.dll"
TL_DLLS_COMMON = (DEFAULTARGS_DLL, "steam_api64.dll")
TL_LOADERS = (
    "uplay_r1_loader64.dll",
    "upc_r1_loader64.dll",
    "upc_r2_loader64.dll",
    "uplay_r2_loader64.dll",
)
TL_TOML = "Config.toml"
TL_LAUNCHER = "LaunchR6.exe"
TL_EXTRACT = (*TL_DLLS_COMMON, *TL_LOADERS, TL_TOML, TL_LAUNCHER)

HM_FOLDER_SUFFIX = "_HeatedMetal"

if IS_WINDOWS:
    SEVENZ_ASSET = "7zr.exe"
    SEVENZ_BIN = BIN_DIR / "7zr.exe"
else:
    SEVENZ_ASSET = f"{_OS}-{_ARCH}.tar.xz"
    SEVENZ_BIN = BIN_DIR / "7zz"

UPDATE_API_URL = f"https://api.github.com/repos/{REPO}/releases/latest"
SEVENZ_API_URL = "https://api.github.com/repos/ip7z/7zip/releases/latest"
HM_API_URL = "https://api.github.com/repos/DataCluster0/HeatedMetal/releases/latest"
HM_TAG_API_URL_FMT = "https://api.github.com/repos/DataCluster0/HeatedMetal/releases/tags/{tag}"

HM_LAUNCHER = "RainbowSix.exe"
VULKAN_EXE = "RainbowSix_Vulkan.exe"
DX11_EXE = "RainbowSix_DX11.exe"

RVPN_BIN_DIR = ASSET_ROOT / "rvpn"
RVPN_STATE_DIR = DATA_ROOT / "rvpn"
RVPN_PREFIX = RVPN_STATE_DIR / "wineprefix"
RVPN_MAC_FILE = RVPN_STATE_DIR / "mac"
RVPN_FONTS_DIR = RVPN_STATE_DIR / "fonts"

WINE_DIR = BIN_DIR / "wine"
WINE_BIN = WINE_DIR / "bin" / "wine"
WINE_API_URL = "https://api.github.com/repos/Kron4ek/Wine-Builds/releases/latest"
WINE_ASSET_SUFFIX = "staging-amd64-wow64.tar.xz"

HTTP_TIMEOUT = 30


_STEAM_HOME_DIRS = (
    ".local/share/Steam",
    ".var/app/com.valvesoftware.Steam/data/Steam",
    ".steam/steam",
    "snap/steam/common/.steam/root",
)


def _steam_roots() -> list[Path]:
    if IS_WINDOWS:
        import winreg

        with contextlib.suppress(OSError):
            with winreg.OpenKey(winreg.HKEY_CURRENT_USER, r"Software\Valve\Steam") as key:
                value, _ = winreg.QueryValueEx(key, "SteamPath")
            if value:
                return [Path(value)]
        program_files = os.environ.get("PROGRAMFILES(X86)", r"C:\Program Files (x86)")
        return [Path(program_files) / "Steam"]
    roots: list[Path] = []
    for name in _STEAM_HOME_DIRS:
        path = Path.home() / name
        if path.is_dir() and path.resolve() not in roots:
            roots.append(path.resolve())
    return roots


STEAM_ROOTS = _steam_roots()
STEAM_DIR = STEAM_ROOTS[0] if STEAM_ROOTS else Path.home() / _STEAM_HOME_DIRS[0]

PREFIX_DIR = DATA_ROOT / "prefixes"
PROTON_DIR = BIN_DIR / "proton"

PROTON_BUILTIN = (
    ("Proton - Experimental", "proton_experimental", "Proton Experimental"),
    ("Proton Hotfix", "proton_hotfix", "Proton Hotfix"),
)

DEFAULT_USERNAME = "ThrowbackUser"
DEFAULT_MAX_DOWNLOADS = 25
DOWNLOADS_MIN = 1
DOWNLOADS_MAX = 100
SCALE_LEVELS = (75, 80, 90, 100, 110, 125)

GIB = 2**30

BUSY_MESSAGE = "A download is running for this season"
DOWNLOAD_RUNNING = "A download is running"
UPDATE_RUNNING = "An update is running"
UNINSTALL_RUNNING = "Uninstall is running"
TRANSFER_RUNNING = "A download or update is running"
CACHE_CLEARING = "The cache is being cleared"
REMOVING_FILES = "Files are still being removed"
PARTIAL_EXISTS = "A partial download exists"
ALREADY_INSTALLED = "Already installed"
NOT_INSTALLED = "Not installed"
NOT_INSTALLED_HM = "Not installed as Heated Metal"
FOLDER_NOT_FOUND = "Folder not found, is the drive connected?"
NO_PROTON = "No Proton found"
GAME_RUNNING = "Stop R6S first"
SEASON_RUNNING = "Stop this season first"

NAME_PATTERN = re.compile(r"^[A-Za-z0-9_.-]+$")
MAX_USERNAME_LENGTH = 16

HEX_PATTERN = re.compile(r"^#[0-9a-fA-F]{6}$")
DEFAULT_ACCENT = "#c0152a"

EVENT_SEASONS = {
    "Y3S1_Chimera": "fury",
    "Y4S1_BurntHorizon": "events",
    "Y4S2_PhantomSight": "events",
    "Y4S4_ShiftingTides": "events",
    "Y5S1_VoidEdge": "events",
}

TEXTURE_QUALITIES = ("Low", "Medium", "High", "Very High", "Ultra")
