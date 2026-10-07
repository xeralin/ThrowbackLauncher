import contextlib
import ctypes
import shutil
import subprocess
import tarfile
from pathlib import Path, PurePosixPath

from core import log
from core.constants import (
    BIN_DIR,
    DEFAULTARGS_DLL,
    HM_API_URL,
    IS_WINDOWS,
    SEVENZ_API_URL,
    SEVENZ_ASSET,
    SEVENZ_BIN,
)
from core.github import RateLimitError, fetch_to, github_asset
from core.reporter import Reporter
from core.throwbackloader import apply_tl, ensure_tl, write_launcher
from core.winspawn import NOWINDOW

_hm_release: tuple[str, str] | None = None


def clear_release_cache() -> None:
    global _hm_release
    _hm_release = None


_DEFAULT_ARGS_NAMES = ("DefaultArgs.dll", DEFAULTARGS_DLL)
_NOTICES_NAME = "ThirdPartyLegalNotices.txt"
_HM_DIR = "HeatedMetal"
_ARCHIVE_PREFIX = "HeatedMetal-"
HM_LATEST = "latest"


def _default_args(mod_dir: Path) -> Path | None:
    for name in _DEFAULT_ARGS_NAMES:
        candidate = mod_dir / name
        if candidate.exists():
            return candidate
    return None


def _hm_mod_complete(mod_dir: Path) -> bool:
    return _default_args(mod_dir) is not None and (mod_dir / _HM_DIR).is_dir()


def ensure_7z(reporter: Reporter, force: bool = False) -> Path:
    if SEVENZ_BIN.exists() and not force:
        return SEVENZ_BIN

    BIN_DIR.mkdir(parents=True, exist_ok=True)
    try:
        _, asset_url = github_asset(SEVENZ_API_URL, SEVENZ_ASSET)
        if IS_WINDOWS:
            fetch_to(asset_url, SEVENZ_BIN, on_progress=reporter.progress)
            return SEVENZ_BIN
        tarxz_path = BIN_DIR / "_7z.tar.xz"
        tmp_dir = BIN_DIR / ".7z.tmp"
        try:
            fetch_to(asset_url, tarxz_path, on_progress=reporter.progress)
            with tarfile.open(tarxz_path) as t:
                t.extract("7zz", tmp_dir, filter="data")
            tmp_bin = tmp_dir / "7zz"
            tmp_bin.chmod(tmp_bin.stat().st_mode | 0o111)
            tmp_bin.replace(SEVENZ_BIN)
            return SEVENZ_BIN
        finally:
            tarxz_path.unlink(missing_ok=True)
            shutil.rmtree(tmp_dir, ignore_errors=True)
    except RateLimitError:
        raise
    except Exception as e:
        raise OSError(log.fail("7z download failed", e)) from e


def resolve_hm_release(hm_version: str) -> tuple[str, str]:
    global _hm_release
    latest = hm_version == HM_LATEST
    if latest and _hm_release is not None:
        return _hm_release
    api_url = (
        HM_API_URL
        if latest
        else f"https://api.github.com/repos/DataCluster0/HeatedMetal/releases/tags/{hm_version}"
    )
    try:
        resolved = github_asset(api_url, ".7z")
    except RateLimitError:
        raise
    except Exception as e:
        raise OSError(log.fail("Heated Metal release lookup failed", e)) from e
    if latest:
        _hm_release = resolved
    return resolved


def _7z_error(proc: subprocess.CompletedProcess) -> str:
    stderr_line = next(
        (
            line.strip()
            for line in proc.stderr.decode(errors="replace").splitlines()
            if line.strip()
        ),
        "",
    )
    return stderr_line or f"7z exit={proc.returncode}"


def _fetch_hm_mod(hm_version: str, tmp_dir: Path, reporter: Reporter) -> tuple[str, Path]:
    tag, asset_url = resolve_hm_release(hm_version)

    BIN_DIR.mkdir(parents=True, exist_ok=True)
    archive_path = BIN_DIR / f"{_ARCHIVE_PREFIX}{tag}.7z"
    if not archive_path.exists():
        try:
            fetch_to(asset_url, archive_path, on_progress=reporter.progress)
        except RateLimitError:
            raise
        except Exception as e:
            raise OSError(log.fail("Heated Metal download failed", e)) from e
        for old in BIN_DIR.glob(f"{_ARCHIVE_PREFIX}*.7z"):
            if old != archive_path:
                old.unlink(missing_ok=True)
    return tag, _extract_hm_archive(archive_path, tmp_dir, reporter)


def _listed_mod_complete(root: tuple[str, ...], entries: list[tuple[str, ...]]) -> bool:
    args = {(*root, name) for name in _DEFAULT_ARGS_NAMES}
    hm_dir = (*root, _HM_DIR)
    return any(parts in args for parts in entries) and any(
        parts[: len(hm_dir)] == hm_dir for parts in entries
    )


class HmFilesRemovedError(OSError):
    pass


def _archive_has_mod(sevenz: Path, archive: Path) -> bool:
    try:
        proc = subprocess.run(
            [str(sevenz), "l", "-ba", "-slt", str(archive)],
            capture_output=True,
            check=False,
            creationflags=NOWINDOW,
        )
    except Exception as e:
        raise OSError(log.fail("Heated Metal archive check failed", e)) from e
    if proc.returncode != 0:
        raise OSError(log.fail("Heated Metal archive check failed", _7z_error(proc)))
    entries = [
        PurePosixPath(line[7:].replace("\\", "/")).parts
        for line in proc.stdout.decode(errors="replace").splitlines()
        if line.startswith("Path = ")
    ]
    roots = {()} | {parts[:1] for parts in entries if len(parts) > 1}
    return any(_listed_mod_complete(root, entries) for root in roots)


def cache_hm_archive(archive: Path) -> Path:
    if not _archive_has_mod(ensure_7z(Reporter()), archive):
        raise OSError(f"{archive.name} does not contain the Heated Metal files")
    cached = BIN_DIR / archive.name
    if archive.resolve() == cached.resolve():
        return cached
    try:
        shutil.copy2(archive, cached)
        for old in BIN_DIR.glob("*.7z"):
            if old != cached and not old.name.startswith(_ARCHIVE_PREFIX):
                old.unlink(missing_ok=True)
    except OSError as e:
        raise OSError(log.fail("Heated Metal archive copy failed", e)) from e
    return cached


def _mod_root(extracted: Path) -> Path | None:
    if _hm_mod_complete(extracted):
        return extracted
    for child in extracted.iterdir():
        if child.is_dir() and _hm_mod_complete(child):
            return child
    return None


def _extract_hm_archive(archive: Path, tmp_dir: Path, reporter: Reporter) -> Path:
    sevenz = ensure_7z(reporter)
    try:
        shutil.rmtree(tmp_dir, ignore_errors=True)
        tmp_dir.mkdir(parents=True)
        proc = subprocess.run(
            [str(sevenz), "x", "-y", f"-o{tmp_dir}", str(archive)],
            capture_output=True,
            check=False,
            creationflags=NOWINDOW,
        )
        mod_dir = _mod_root(tmp_dir) if proc.returncode == 0 else None
    except Exception as e:
        raise OSError(log.fail("Heated Metal extraction failed", e)) from e
    if mod_dir is not None:
        return mod_dir
    try:
        intact = _archive_has_mod(sevenz, archive)
    except OSError:
        intact = False
    if intact:
        raise HmFilesRemovedError(
            log.fail("Heated Metal extraction failed", "files missing after extraction")
        )
    archive.unlink(missing_ok=True)
    detail = (
        _7z_error(proc)
        if proc.returncode != 0
        else f"{archive.name} does not contain the Heated Metal files"
    )
    raise OSError(log.fail("Heated Metal extraction failed", detail))


_PF_SSE4_2_INSTRUCTIONS_AVAILABLE = 38
_PF_AVX_INSTRUCTIONS_AVAILABLE = 39


def hm_files_present(target_dir: Path) -> bool:
    return (
        _default_args(target_dir) is not None
        and (target_dir / _HM_DIR / "HeatedMetal.dll").is_file()
    )


def hm_build_time(target_dir: Path) -> int:
    try:
        with (target_dir / _HM_DIR / "HeatedMetal.dll").open("rb") as f:
            header = f.read(4096)
    except OSError:
        return 0
    pe = int.from_bytes(header[0x3C:0x40], "little")
    return int.from_bytes(header[pe + 8 : pe + 12], "little")


def hm_crash_log(target_dir: Path) -> list[Path]:
    logs = [
        path
        for path in (target_dir / _HM_DIR / "CrashLogs").glob("*")
        if path.suffix in (".dmp", ".txt")
    ]
    if not logs:
        return []
    try:
        newest = max(logs, key=lambda path: path.stat().st_mtime).stem
    except OSError:
        return []
    return sorted(path for path in logs if path.stem == newest)


def _hm_version_file(target_dir: Path) -> Path:
    return target_dir / _HM_DIR / ".version"


def hm_installed_version(target_dir: Path) -> str | None:
    try:
        return _hm_version_file(target_dir).read_text().strip() or None
    except OSError:
        return None


def _detect_cpu_variant() -> str:
    if IS_WINDOWS:
        present = ctypes.WinDLL("kernel32").IsProcessorFeaturePresent
        if present(_PF_AVX_INSTRUCTIONS_AVAILABLE):
            return "AVX"
        if present(_PF_SSE4_2_INSTRUCTIONS_AVAILABLE):
            return "SSE"
        return ""
    with contextlib.suppress(OSError):
        for line in Path("/proc/cpuinfo").read_text().splitlines():
            if line.startswith("flags"):
                tokens = line.split()
                if "avx" in tokens:
                    return "AVX"
                if "sse4_2" in tokens:
                    return "SSE"
                break
    return ""


def _apply_hm_mod(target_dir: Path, mod_dir: Path) -> None:
    source = _default_args(mod_dir)
    if source is None:
        raise OSError("DefaultArgs.dll is missing from the mod files")
    remove_hm_files(target_dir)
    target_hm = target_dir / _HM_DIR
    shutil.copy2(source, target_dir / DEFAULTARGS_DLL)
    shutil.copytree(mod_dir / _HM_DIR, target_hm)

    variant = _detect_cpu_variant()
    if variant:
        variant_dll = target_hm / f"HeatedMetal{variant}.dll"
        if variant_dll.exists():
            shutil.copy2(variant_dll, target_hm / "HeatedMetal.dll")

    notices = mod_dir / _NOTICES_NAME
    if notices.exists():
        shutil.copy2(notices, target_dir / _NOTICES_NAME)


def apply_hm(
    target_dir: Path,
    username: str,
    download: dict,
    reporter: Reporter,
    archive: Path | None = None,
) -> None:
    tmp_dir = BIN_DIR / ".hm.tmp"
    try:
        if download.get("hm_beta"):
            if archive is None:
                raise OSError("The Heated Metal archive is required")
            version = ""
            mod_dir = _extract_hm_archive(archive, tmp_dir, reporter)
        else:
            version, mod_dir = _fetch_hm_mod(download["hm_version"], tmp_dir, reporter)

        ensure_tl(reporter)
        try:
            apply_tl(target_dir, username)
            _apply_hm_mod(target_dir, mod_dir)
            if version:
                _hm_version_file(target_dir).write_text(version)
            write_launcher(target_dir)
        except OSError as e:
            raise OSError(log.fail("Heated Metal setup failed", e)) from e
    finally:
        shutil.rmtree(tmp_dir, ignore_errors=True)


def remove_hm_files(target_dir: Path) -> None:
    target_hm = target_dir / _HM_DIR
    if target_hm.exists():
        shutil.rmtree(target_hm)
    for name in (*_DEFAULT_ARGS_NAMES, _NOTICES_NAME):
        (target_dir / name).unlink(missing_ok=True)
