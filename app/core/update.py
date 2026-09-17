import contextlib
import os
import re
import shutil
import subprocess
import zipfile
from collections.abc import Callable
from concurrent.futures import ThreadPoolExecutor
from dataclasses import dataclass
from pathlib import Path
from tempfile import TemporaryDirectory

from core import log
from core.constants import (
    DD_API_URL,
    DD_BIN,
    GAME_RUNNING,
    HM_API_URL,
    IS_WINDOWS,
    SELF_UPDATABLE,
    SEVENZ_API_URL,
    SEVENZ_BIN,
    TL_API_URL,
    UPDATE_API_URL,
    version_tuple,
)
from core.depot import ensure_depotdownloader
from core.github import (
    RateLimitError,
    fetch_to,
    github_asset,
    github_json,
    github_tag,
    invalidate_api_cache,
)
from core.heatedmetal import (
    apply_hm,
    clear_release_cache,
    ensure_7z,
    hm_installed_version,
    resolve_hm_release,
)
from core.manifest import (
    effective_username,
    hm_folder_name,
    installed_downloads,
    is_installed,
    load_downloads,
)
from core.reporter import Reporter
from core.self_update import helper_argv, pending_dir, write_outcome
from core.settings import libraries, load_settings
from core.steam import is_game_running
from core.throwbackloader import (
    apply_tl,
    ensure_tl,
    tl_folder_version,
    tl_present,
    tl_version,
    write_launcher,
)
from core.winspawn import NOWINDOW, spawn_detached
from layout import APP_NAME, APPIMAGE_ASSET, PENDING_FILE, RUNTIME_ASSET, VERSION


def _newer(latest: str, current: str) -> bool:
    try:
        return version_tuple(latest) > version_tuple(current)
    except ValueError:
        return latest != current


_VERSION_TIMEOUT_S = 30


def _binary_version(binary: Path, args: list[str], pattern: str) -> str | None:
    try:
        out = subprocess.run(
            [str(binary), *args],
            capture_output=True,
            text=True,
            timeout=_VERSION_TIMEOUT_S,
            check=False,
            creationflags=NOWINDOW,
        ).stdout
    except Exception:
        return None
    match = re.search(pattern, out)
    return match.group(1) if match else None


_throwback_release: dict | None = None


def _throwback_fetch() -> dict:
    global _throwback_release
    if _throwback_release is None:
        asset = RUNTIME_ASSET if IS_WINDOWS else APPIMAGE_ASSET
        tag, url = github_asset(UPDATE_API_URL, asset)
        _throwback_release = {"tag": tag, "url": url}
    return _throwback_release


def _throwback_latest() -> str:
    tag = _throwback_fetch()["tag"].removeprefix("v")
    version_tuple(tag)
    return tag


_REPOSITORY_RX = re.compile(r"/repos/([^/]+/[^/]+)/")


@dataclass
class ReleaseNotes:
    body: str = ""
    url: str = ""
    repository: str = ""


def _release_notes(api_url: str) -> ReleaseNotes:
    release = github_json(api_url)
    repository = _REPOSITORY_RX.search(api_url)
    return ReleaseNotes(
        release.get("body") or "",
        release.get("html_url") or "",
        repository.group(1) if repository else "",
    )


def _throwback_apply_windows(reporter: Reporter, url: str, tag: str) -> bool:
    pending = pending_dir()
    try:
        with TemporaryDirectory() as tmp:
            archive = Path(tmp) / RUNTIME_ASSET
            fetch_to(url, archive, on_progress=reporter.progress)
            shutil.rmtree(pending, ignore_errors=True)
            pending.mkdir(parents=True)
            with zipfile.ZipFile(archive) as z:
                z.extractall(pending)
                names = z.namelist()
            missing = [n for n in names if not (pending / n).exists()]
            if missing:
                raise OSError(f"{len(missing)} update files are missing")
            (pending / PENDING_FILE).write_text(tag.removeprefix("v"), encoding="ascii")
        spawn_detached(helper_argv(pending))
    except BaseException:
        shutil.rmtree(pending, ignore_errors=True)
        raise
    return True


def _throwback_apply_appimage(reporter: Reporter, url: str) -> bool:
    appimage = os.environ.get("APPIMAGE", "")
    if not appimage:
        reporter.fail("Update failed, not running from an AppImage")
        return False
    target = Path(appimage)
    replacement = target.with_name(target.name + ".update")
    try:
        fetch_to(url, replacement, on_progress=reporter.progress)
        replacement.chmod(0o755)
        replacement.replace(target)
    except BaseException:
        replacement.unlink(missing_ok=True)
        raise
    write_outcome(True)
    return True


def _throwback_apply(reporter: Reporter) -> bool:
    if is_game_running():
        raise OSError(GAME_RUNNING)
    release = _throwback_fetch()
    try:
        if IS_WINDOWS:
            ok = _throwback_apply_windows(reporter, release["url"], release["tag"])
        else:
            ok = _throwback_apply_appimage(reporter, release["url"])
    except RateLimitError:
        raise
    except Exception as e:
        reporter.fail(log.fail("Update failed", e))
        return False
    return ok


def _tl_current() -> str | None:
    versions = [tl_version(), *(tl_folder_version(d) for d in installed_downloads())]
    if any(v is None for v in versions):
        return None
    return min(versions, key=version_tuple)


def _tl_latest() -> str:
    tag = github_tag(TL_API_URL).removeprefix("v")
    version_tuple(tag)
    return tag


def _tl_apply(reporter: Reporter) -> bool:
    if is_game_running():
        raise OSError(GAME_RUNNING)
    ensure_tl(reporter, force=True)
    username = effective_username(load_settings())
    for folder in installed_downloads():
        apply_tl(folder, username)
        write_launcher(folder)
    return True


_hm_installs: list[tuple[dict, Path]] | None = None


def _hm_latest_installs() -> list[tuple[dict, Path]]:
    global _hm_installs
    if _hm_installs is not None:
        return _hm_installs
    try:
        downloads = load_downloads()
    except Exception as e:
        log.fail("Heated Metal check failed", e)
        return []
    installs: list[tuple[dict, Path]] = []
    for download in downloads:
        if download.get("hm_version") != "latest":
            continue
        for root in libraries():
            folder = root / hm_folder_name(download["key"])
            if is_installed(folder):
                installs.append((download, folder))
    _hm_installs = installs
    return installs


def _hm_current() -> str | None:
    versions = [hm_installed_version(folder) for _, folder in _hm_latest_installs()]
    if not versions or any(v is None for v in versions):
        return None
    try:
        return min(versions, key=version_tuple)
    except ValueError:
        return min(versions)


def _hm_apply(reporter: Reporter) -> bool:
    global _hm_installs
    _hm_installs = None
    installs = _hm_latest_installs()
    if not installs:
        return False
    if is_game_running():
        raise OSError(GAME_RUNNING)
    ok = True
    username = effective_username(load_settings())
    try:
        for download, folder in installs:
            try:
                apply_hm(folder, username, download, reporter=reporter)
            except OSError as e:
                reporter.fail(str(e))
                ok = False
    finally:
        _hm_installs = None
    return ok


@dataclass
class Component:
    name: str
    present: Callable[[], bool]
    current: Callable[[], str | None]
    latest: Callable[[], str | None]
    apply: Callable[..., bool]
    restart: bool = False
    release_url: str = ""


COMPONENTS = [
    Component(
        APP_NAME,
        lambda: SELF_UPDATABLE,
        lambda: VERSION,
        _throwback_latest,
        _throwback_apply,
        restart=True,
        release_url=UPDATE_API_URL,
    ),
    Component(
        "DepotDownloader",
        DD_BIN.exists,
        lambda: _binary_version(DD_BIN, ["--version"], r"v(\d+(?:\.\d+)+)"),
        lambda: github_tag(DD_API_URL).removeprefix("DepotDownloader_") or None,
        lambda reporter: bool(ensure_depotdownloader(reporter, force=True)),
        release_url=DD_API_URL,
    ),
    Component(
        "7z",
        SEVENZ_BIN.exists,
        lambda: _binary_version(SEVENZ_BIN, [], r"7-Zip(?:\s+\([arz]\))?\s+(\d+(?:\.\d+)+)"),
        lambda: github_tag(SEVENZ_API_URL),
        lambda reporter: bool(ensure_7z(reporter, force=True)),
    ),
    Component(
        "ThrowbackLoader",
        tl_present,
        _tl_current,
        _tl_latest,
        _tl_apply,
        release_url=TL_API_URL,
    ),
    Component(
        "Heated Metal",
        lambda: bool(_hm_latest_installs()),
        _hm_current,
        lambda: resolve_hm_release("latest")[0],
        _hm_apply,
        release_url=HM_API_URL,
    ),
]


type Pending = list[tuple[Component, str, ReleaseNotes]]


def available(force: bool = False) -> tuple[Pending, str, str]:
    global _throwback_release, _hm_installs
    _throwback_release = None
    _hm_installs = None
    clear_release_cache()
    if force:
        invalidate_api_cache()
    present = [c for c in COMPONENTS if c.present()]
    if not present:
        return [], "", ""

    def probe(component: Component) -> tuple[str | None, str, str]:
        try:
            return component.latest(), "", ""
        except RateLimitError as e:
            return None, "rate_limit", e.message()
        except Exception as e:
            log.fail(f"{component.name} check failed", e)
            return None, "error", ""

    with ThreadPoolExecutor(max_workers=len(present)) as ex:
        results = list(ex.map(probe, present))
    failures = {reason for _, reason, _ in results if reason}
    failure = "rate_limit" if "rate_limit" in failures else ("error" if failures else "")
    detail = next((d for _, _, d in results if d), "")
    pending: Pending = []
    for component, (latest, _, _) in zip(present, results, strict=True):
        if not latest:
            continue
        current = component.current()
        if current is not None and not _newer(latest, current):
            continue
        notes = ReleaseNotes()
        if component.release_url:
            with contextlib.suppress(Exception):
                notes = _release_notes(component.release_url)
        pending.append((component, latest, notes))
    return pending, failure, detail
