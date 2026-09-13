import contextlib
import shutil
from collections.abc import Iterator
from pathlib import Path

from core import log
from core.constants import TEXTURE_QUALITIES, TEXTURE_RX

KINDS = ("videos", "events", "textures")


def folder_size(path: Path) -> int:
    total = 0
    for root, _dirs, files in path.walk():
        for name in files:
            with contextlib.suppress(OSError):
                total += (root / name).lstat().st_size
    return total


def _texture_forges(path: Path) -> Iterator[tuple[Path, int, int]]:
    for f in path.iterdir():
        if f.suffix.lower() != ".forge":
            continue
        m = TEXTURE_RX.search(f.stem)
        if not m:
            continue
        level = int(m.group(1))
        if not 0 <= level < len(TEXTURE_QUALITIES):
            continue
        try:
            yield f, level, f.stat().st_size
        except OSError:
            continue


def _texture_tiers(path: Path) -> dict[int, int]:
    tiers: dict[int, int] = {}
    for _, level, size in _texture_forges(path):
        tiers[level] = tiers.get(level, 0) + size
    return tiers


def _startup_dir(path: Path) -> Path:
    return path / "videos" / "startup"


def _video_files(path: Path) -> list[Path]:
    v = path / "videos"
    if not v.is_dir():
        return []
    try:
        return [f for f in v.iterdir() if f.is_file()]
    except OSError:
        return []


def _files_size(files: list[Path]) -> int:
    size = 0
    for f in files:
        with contextlib.suppress(OSError):
            size += f.stat().st_size
    return size


def _videos_size(path: Path) -> int:
    return _files_size(_video_files(path)) + folder_size(_startup_dir(path))


def _event_files(path: Path, pattern: str) -> list[Path]:
    try:
        return [
            f
            for f in path.iterdir()
            if f.is_file()
            and f.suffix.lower() in (".forge", ".depgraphbin")
            and pattern in f.stem.lower()
        ]
    except OSError:
        return []


def _delete_files(files: list[Path]) -> None:
    failed = 0
    last: OSError | None = None
    for f in files:
        try:
            f.unlink()
        except OSError as e:
            failed += 1
            last = e
    if failed:
        log.fail("Shears could not delete files", f"{failed} files, last: {last}")


def scan_download(d: Path, event_pattern: str | None) -> dict:
    return {
        "tiers": _texture_tiers(d),
        "videos": _videos_size(d),
        "events": _files_size(_event_files(d, event_pattern)) if event_pattern else 0,
    }


def cut_download(d: Path, kind: str, level: int, event_pattern: str | None) -> None:
    if kind == "videos":
        startup = _startup_dir(d)
        _delete_files(_video_files(d) + [f for f in startup.rglob("*") if f.is_file()])
        shutil.rmtree(startup, ignore_errors=True)
        return
    if kind == "events":
        if event_pattern:
            _delete_files(_event_files(d, event_pattern))
        return
    _delete_files([f for f, lvl, _ in _texture_forges(d) if lvl > level])
