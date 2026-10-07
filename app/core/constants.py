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
    user_data_base,
)


def version_tuple(v: str) -> tuple[int, ...]:
    return tuple(int(p) for p in v.split("."))


FROZEN = bool(getattr(sys, "frozen", False)) or "__compiled__" in globals()
IS_WINDOWS = sys.platform.startswith("win")


ASSET_ROOT = (
    Path(sys.executable).resolve().parent if FROZEN else Path(__file__).resolve().parents[2]
)


def _data_root() -> Path:
    if FROZEN and IS_WINDOWS:
        if ASSET_ROOT.name in (APP_SUBDIR, PENDING_SUBDIR, PREVIOUS_SUBDIR):
            return ASSET_ROOT.parent
        return ASSET_ROOT
    return user_data_base() / DIR_NAME


DATA_ROOT = _data_root().resolve()
SELF_UPDATABLE = FROZEN and (
    ASSET_ROOT.name == APP_SUBDIR if IS_WINDOWS else bool(os.environ.get("APPIMAGE"))
)
INSTANCE_KEY = f"{DIR_NAME}-{hashlib.blake2s(str(DATA_ROOT).encode(), digest_size=4).hexdigest()}"

MANIFEST_FILE = ASSET_ROOT / "manifest.toml"
NEXT_OUT_DIR = ASSET_ROOT / "next" / "out"
ICON_FILE = ASSET_ROOT / "icon.png" if FROZEN else ASSET_ROOT / ".github" / "icon.png"

SETTINGS_FILE = DATA_ROOT / "settings.toml"
BIN_DIR = DATA_ROOT / "bin"
API_CACHE_FILE = BIN_DIR / "api_cache.json"
ERRORS_FILE = BIN_DIR / "errors.txt"


DEFAULT_DOWNLOADS_DIR = DATA_ROOT / "downloads"


_OS = "windows" if IS_WINDOWS else "linux"
_ARCH = "arm64" if platform.machine().lower() in ("arm64", "aarch64") else "x64"

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
    SEVENZ_BIN = BIN_DIR / SEVENZ_ASSET
else:
    SEVENZ_ASSET = f"{_OS}-{_ARCH}.tar.xz"
    SEVENZ_BIN = BIN_DIR / "7zz"

UPDATE_API_URL = f"https://api.github.com/repos/{OWNER}/ThrowbackLauncher/releases/latest"
SEVENZ_API_URL = "https://api.github.com/repos/ip7z/7zip/releases/latest"
HM_API_URL = "https://api.github.com/repos/DataCluster0/HeatedMetal/releases/latest"

RVPN_BIN_DIR = ASSET_ROOT / "rvpn"
RVPN_STATE_DIR = DATA_ROOT / "rvpn"
RVPN_PREFIX = RVPN_STATE_DIR / "wineprefix"
RVPN_MAC_FILE = RVPN_STATE_DIR / "mac"

WINE_DIR = BIN_DIR / "wine"
WINE_BIN = WINE_DIR / "bin" / "wine"

HTTP_TIMEOUT = 30


_STEAM_HOME_DIRS = (
    ".local/share/Steam",
    ".steam/steam",
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
        path = (Path.home() / name).resolve()
        if path.is_dir() and path not in roots:
            roots.append(path)
    return roots


STEAM_ROOTS = _steam_roots()
STEAM_DIR = STEAM_ROOTS[0] if STEAM_ROOTS else Path.home() / _STEAM_HOME_DIRS[0]

PREFIX_DIR = DATA_ROOT / "prefixes"
PROTON_DIR = BIN_DIR / "proton"
SYSTEM_COMPAT_DIRS = (
    Path("/usr/share/steam/compatibilitytools.d"),
    Path("/usr/local/share/steam/compatibilitytools.d"),
)

PROTON_BUILTIN = (
    ("Proton - Experimental", "proton_experimental", "Proton Experimental"),
    ("Proton Hotfix", "proton_hotfix", "Proton Hotfix"),
)

DEFAULT_USERNAME = "ThrowbackUser"
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
NOT_INSTALLED = "Not installed"
NOT_INSTALLED_HM = "Not installed as Heated Metal"
FOLDER_NOT_FOUND = "Folder not found, is the drive connected?"
NO_PROTON = "No Proton found"
GAME_RUNNING = "Stop R6S first"
SEASON_RUNNING = "Stop this season first"

NAME_CHARS = "A-Za-z0-9_.-"
NAME_PATTERN = re.compile(rf"^[{NAME_CHARS}]+$")
MAX_USERNAME_LENGTH = 16

HEX_PATTERN = re.compile(r"^#[0-9a-fA-F]{6}$")
DEFAULT_ACCENT = "#c0152a"

EVENT_SEASONS = {
    "Y3S1_Chimera": "fury",
    "Y4S1_BurntHorizon": "events",
    "Y4S2_PhantomSight": "events",
    "Y4S4_ShiftingTides": "events",
    "Y5S1_VoidEdge": "events",
    "Y5S2_SteelWave": "events",
    "Y5S3_ShadowLegacy": "events",
    "Y5S4_NeonDawn": "events",
    "Y6S1_CrimsonHeist": "events",
    "Y6S2_NorthStar": "events",
    "Y6S3_CrystalGuard": "events",
    "Y6S4_HighCalibre": "events",
    "Y7S1_DemonVeil": "events",
    "Y7S2_VectorGlare": "events",
    "Y7S3_BrutalSwarm": "events",
    "Y7S4_SolarRaid": "events",
    "Y8S1_CommandingForce": "events",
    "Y8S2_DreadFactor": "events",
    "Y8S3_HeavyMettle": "events",
    "Y8S4_DeepFreeze": "events",
    "Y9S1_DeadlyOmen": "events",
    "Y9S2_NewBlood": "events",
    "Y9S3_TwinShells": "events",
    "Y9S4_CollisionPoint": "events",
    "Y10S1_PrepPhase": "events",
    "Y10S2_Daybreak": "events",
    "Y10S3_HighStakes": "events",
    "Y10S4_TenfoldPursuit": "events",
    "Y11S1_SilentHunt": "events",
    "Y11S2_SystemOverride": "events",
}

TEXTURE_QUALITIES = ("Low", "Medium", "High", "Very High", "Ultra")
