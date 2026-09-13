import contextlib
import shutil
import subprocess
import tarfile
from pathlib import Path

from core import log
from core.constants import (
    BIN_DIR,
    DEFAULTARGS_DLL,
    HM_API_URL,
    HM_TAG_API_URL_FMT,
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


def _default_args(mod_dir: Path) -> Path | None:
    for name in _DEFAULT_ARGS_NAMES:
        candidate = mod_dir / name
        if candidate.exists():
            return candidate
    return None


def _hm_mod_complete(mod_dir: Path) -> bool:
    return _default_args(mod_dir) is not None and (mod_dir / "HeatedMetal").is_dir()


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
    latest = hm_version == "latest"
    if latest and _hm_release is not None:
        return _hm_release
    api_url = HM_API_URL if latest else HM_TAG_API_URL_FMT.format(tag=hm_version)
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
    return stderr_line or f"7z exited with code {proc.returncode}"


def _prune_archives(keep: Path) -> None:
    for old in BIN_DIR.glob("*.7z"):
        if old != keep:
            old.unlink(missing_ok=True)


def _fetch_hm_mod(hm_version: str, tmp_dir: Path, reporter: Reporter) -> tuple[str, Path]:
    tag, asset_url = resolve_hm_release(hm_version)

    BIN_DIR.mkdir(parents=True, exist_ok=True)
    archive_path = BIN_DIR / f"HeatedMetal-{tag}.7z"
    if not archive_path.exists():
        try:
            fetch_to(asset_url, archive_path, on_progress=reporter.progress)
        except RateLimitError:
            raise
        except Exception as e:
            raise OSError(log.fail("Heated Metal download failed", e)) from e
        _prune_archives(archive_path)
    return tag, _extract_hm_archive(archive_path, tmp_dir, reporter)


def cache_hm_archive(archive: Path) -> Path:
    BIN_DIR.mkdir(parents=True, exist_ok=True)
    cached = BIN_DIR / archive.name
    if archive.resolve() == cached.resolve():
        return cached
    shutil.copy2(archive, cached)
    _prune_archives(cached)
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
    archive.unlink(missing_ok=True)
    if proc.returncode != 0:
        raise OSError(log.fail("Heated Metal extraction failed", _7z_error(proc)))
    raise OSError(f"{archive.name} does not contain the Heated Metal files")


_PF_SSE4_2_INSTRUCTIONS_AVAILABLE = 38
_PF_AVX_INSTRUCTIONS_AVAILABLE = 39


def hm_version_file(target_dir: Path) -> Path:
    return target_dir / "HeatedMetal" / ".version"


def hm_installed_version(target_dir: Path) -> str | None:
    try:
        return hm_version_file(target_dir).read_text().strip() or None
    except OSError:
        return None


def _detect_cpu_variant() -> str:
    if IS_WINDOWS:
        import ctypes

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
    for name in _DEFAULT_ARGS_NAMES:
        (target_dir / name).unlink(missing_ok=True)
    target_hm = target_dir / "HeatedMetal"
    if target_hm.exists():
        shutil.rmtree(target_hm)
    shutil.copy2(source, target_dir / DEFAULTARGS_DLL)
    shutil.copytree(mod_dir / "HeatedMetal", target_hm)

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
            version, mod_dir = _fetch_hm_mod(
                download.get("hm_version", "latest"), tmp_dir, reporter
            )

        ensure_tl(reporter)
        try:
            apply_tl(target_dir, username)
            _apply_hm_mod(target_dir, mod_dir)
            if version:
                hm_version_file(target_dir).write_text(version)
            write_launcher(target_dir)
        except OSError as e:
            raise OSError(log.fail("Heated Metal setup failed", e)) from e
    finally:
        shutil.rmtree(tmp_dir, ignore_errors=True)


def remove_hm_files(target_dir: Path) -> None:
    target_hm = target_dir / "HeatedMetal"
    if target_hm.exists():
        shutil.rmtree(target_hm)
    for name in (*_DEFAULT_ARGS_NAMES, _NOTICES_NAME):
        (target_dir / name).unlink(missing_ok=True)
