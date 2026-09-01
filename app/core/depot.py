import shutil
import zipfile
from pathlib import Path

from core import log
from core.constants import (
    BIN_DIR,
    DD_API_URL,
    DD_ASSET,
    DD_BIN,
    DD_MEMBER,
    DD_ZIP,
    IS_WINDOWS,
)
from core.github import RateLimited, fetch_to, github_asset
from core.reporter import NullReporter, Reporter


def ensure_depotdownloader(reporter: Reporter | None = None, force: bool = False) -> Path:
    if DD_BIN.exists() and not force:
        return DD_BIN

    reporter = reporter or NullReporter()
    reporter.update("Fetching DepotDownloader")
    BIN_DIR.mkdir(parents=True, exist_ok=True)
    part = DD_BIN.with_name(DD_BIN.name + ".part")
    try:
        _, asset_url = github_asset(DD_API_URL, DD_ASSET)
        fetch_to(asset_url, DD_ZIP, on_progress=reporter.progress)
        with zipfile.ZipFile(DD_ZIP) as z, z.open(DD_MEMBER) as src, part.open("wb") as f:
            shutil.copyfileobj(src, f)
        if not IS_WINDOWS:
            part.chmod(part.stat().st_mode | 0o111)
        part.replace(DD_BIN)
    except RateLimited:
        raise
    except Exception as e:
        raise OSError(log.fail("DepotDownloader download failed", e)) from e
    finally:
        part.unlink(missing_ok=True)
        DD_ZIP.unlink(missing_ok=True)
    return DD_BIN


def depot_commands(
    download: dict, steam_account: str, target: Path, max_downloads: int
) -> list[dict]:
    depots: list[tuple[int, str, str, bool]] = [
        (download["depot_ww"], download["manifest_ww"], "Worldwide", False),
        (download["depot_rus"], download["manifest_rus"], "Russian", True),
        (download["depot_content"], download["manifest_content"], "Content", False),
    ]
    common = [
        "-app",
        str(download["app"]),
        "-username",
        steam_account,
        "-remember-password",
        "-dir",
        str(target),
        "-validate",
        "-max-downloads",
        str(max_downloads),
    ]
    return [
        {
            "args": ["-depot", str(depot_id), "-manifest", manifest_id, *common],
            "name": name,
            "optional": optional,
        }
        for depot_id, manifest_id, name, optional in depots
    ]
