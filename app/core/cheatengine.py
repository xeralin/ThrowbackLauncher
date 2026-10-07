import re
import subprocess
from pathlib import Path

from core import log
from core.constants import NO_PROTON, NOT_INSTALLED, PREFIX_DIR
from core.manifest import installed_path
from core.steam import proton_env, resolve_proton
from core.throwbackloader import read_tl_autorun, write_tl_autorun


def _find_ce_exe(key: str) -> str | None:
    drive_c = PREFIX_DIR / key / "pfx" / "drive_c"
    exes = [
        exe
        for base in ("Program Files", "Program Files (x86)")
        for exe in (drive_c / base).glob("Cheat Engine*/Cheat Engine.exe")
    ]
    if not exes:
        return None
    newest = max(exes, key=lambda exe: exe.parent.stat().st_mtime)
    return "C:\\" + str(newest.relative_to(drive_c)).replace("/", "\\")


def _season_folder(key: str) -> Path:
    path = installed_path(key, False)
    if path is None:
        raise OSError(NOT_INSTALLED)
    return path


CE_MARKER = "cheat engine"


def _without_ce(autorun: list[str]) -> list[str]:
    return [t for t in autorun if CE_MARKER not in t.lower()]


def _add_to_autorun(folder: Path, exe: str) -> None:
    try:
        write_tl_autorun(folder, [*_without_ce(read_tl_autorun(folder)), exe])
    except Exception as e:
        raise OSError(log.fail("Cheat Engine setup failed", e)) from e


def is_cheat_engine_present(key: str) -> bool:
    return _find_ce_exe(key) is not None


def remove_cheat_engine(key: str) -> None:
    folder = _season_folder(key)
    try:
        write_tl_autorun(folder, _without_ce(read_tl_autorun(folder)))
    except Exception as e:
        raise OSError(log.fail("Cheat Engine removal failed", e)) from e


def add_cheat_engine(key: str) -> None:
    folder = _season_folder(key)
    exe = _find_ce_exe(key)
    if exe is None:
        raise OSError("Cheat Engine not found")
    _add_to_autorun(folder, exe)


_WINE_NOISE = re.compile(r"^(\d+:)?(fixme|warn|trace):")


def install_cheat_engine(key: str, installer: Path, settings: dict) -> None:
    folder = _season_folder(key)
    proton = resolve_proton(settings, key=key)
    if proton is None:
        raise OSError(NO_PROTON)
    try:
        env = proton_env(PREFIX_DIR / key)
        proc = subprocess.run(
            [str(proton["binary"]), "run", str(installer)],
            env=env,
            stdout=subprocess.DEVNULL,
            stderr=subprocess.PIPE,
            check=False,
        )
    except Exception as e:
        raise OSError(log.fail("Cheat Engine installer failed to start", e)) from e
    exe = _find_ce_exe(key)
    if exe is None:
        stderr_line = next(
            (
                line
                for raw in proc.stderr.decode(errors="replace").splitlines()
                if (line := raw.strip()) and not _WINE_NOISE.match(line)
            ),
            "",
        )
        detail = f"installer exit={proc.returncode}" + (f": {stderr_line}" if stderr_line else "")
        raise OSError(log.fail("Cheat Engine was not found after the installer closed", detail))
    _add_to_autorun(folder, exe)
