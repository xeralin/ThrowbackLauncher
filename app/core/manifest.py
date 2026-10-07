import re
import tomllib
from pathlib import Path

from core import log
from core.constants import (
    DEFAULT_USERNAME,
    HM_FOLDER_SUFFIX,
    MANIFEST_FILE,
    TL_LAUNCHER,
    TL_TOML,
)
from core.settings import get_setting, libraries
from core.throwbackloader import write_tl_toml

_INSTALL_PATTERN = re.compile(r"^Y\d+S\d+_")


def load_downloads() -> list[dict]:
    try:
        with MANIFEST_FILE.open("rb") as f:
            data = tomllib.load(f)
    except FileNotFoundError:
        raise RuntimeError(log.fail("manifest.toml not found", MANIFEST_FILE)) from None
    except OSError as e:
        raise RuntimeError(log.fail("manifest.toml read failed", e)) from e
    except (tomllib.TOMLDecodeError, UnicodeDecodeError) as e:
        raise RuntimeError(log.fail("manifest.toml is malformed", e)) from e

    defaults = {key: value for key, value in data.items() if not isinstance(value, dict)}
    downloads = [
        {"key": key, **defaults, **block} for key, block in data.items() if isinstance(block, dict)
    ]
    if not downloads:
        raise RuntimeError(log.fail("manifest.toml has no entries", MANIFEST_FILE))
    return downloads


def is_season_folder(d: Path) -> bool:
    return bool(_INSTALL_PATTERN.match(d.name))


def local_downloads() -> list[Path]:
    return [
        d
        for root in libraries()
        if root.exists()
        for d in sorted(root.glob("*"))
        if d.is_dir() and is_season_folder(d)
    ]


def resolve_install(folder_name: str, downloads: list[dict]) -> tuple[dict, bool] | None:
    if folder_name.endswith(HM_FOLDER_SUFFIX):
        prefix = folder_name.removesuffix(HM_FOLDER_SUFFIX) + "_"
        for d in downloads:
            if d["key"].startswith(prefix) and d.get("hm"):
                return d, True
        return None
    for d in downloads:
        if d["key"] == folder_name:
            return d, False
    return None


def is_installed(d: Path) -> bool:
    return (d / TL_LAUNCHER).exists()


def installed_downloads() -> list[Path]:
    return [d for d in local_downloads() if is_installed(d)]


def hm_folder_name(key: str) -> str:
    return f"{key.split('_', 1)[0]}{HM_FOLDER_SUFFIX}"


def edition_folder(key: str, hm: bool) -> str:
    return hm_folder_name(key) if hm else key


def installed_path(key: str, hm: bool) -> Path | None:
    folder = edition_folder(key, hm)
    for root in libraries():
        if is_installed(root / folder):
            return root / folder
    return None


def partial_path(key: str, hm: bool) -> Path | None:
    if installed_path(key, hm) is not None:
        return None
    folder = edition_folder(key, hm)
    for root in libraries():
        if (root / folder).is_dir():
            return root / folder
    return None


def effective_username(settings: dict) -> str:
    return get_setting(settings, "username", DEFAULT_USERNAME)


def write_download_username(d: Path, username: str) -> None:
    if (d / TL_TOML).exists():
        write_tl_toml(d, username)
