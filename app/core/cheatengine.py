import re
import subprocess
from pathlib import Path

from core import log
from core.constants import NO_PROTON, PREFIX_DIR
from core.manifest import installed_path
from core.steam import proton_env, resolve_proton
from core.throwbackloader import read_tl_tools, write_tl_tools


def _drive_c(key: str) -> Path:
    return PREFIX_DIR / key / "pfx" / "drive_c"


def _find_ce_exe(drive_c: Path) -> str | None:
    folders = []
    for base in ("Program Files", "Program Files (x86)"):
        root = drive_c / base
        if root.is_dir():
            folders += [f for f in root.glob("Cheat Engine*") if (f / "Cheat Engine.exe").exists()]
    if not folders:
        return None
    newest = max(folders, key=lambda folder: folder.stat().st_mtime)
    exe = newest / "Cheat Engine.exe"
    return "C:\\" + str(exe.relative_to(drive_c)).replace("/", "\\")


def _season_folder(key: str) -> Path:
    path = installed_path(key, False)
    if path is None:
        raise OSError("Season is not installed")
    return path


CE_MARKER = "cheat engine"


def _without_ce(tools: list[str]) -> list[str]:
    return [t for t in tools if CE_MARKER not in t.lower()]


def _add_tool(folder: Path, exe: str) -> None:
    try:
        tools = _without_ce(read_tl_tools(folder))
        tools.append(exe)
        write_tl_tools(folder, tools)
    except Exception as e:
        raise OSError(log.fail("Cheat Engine setup failed", e)) from e


def is_cheat_engine_present(key: str) -> bool:
    return _find_ce_exe(_drive_c(key)) is not None


def remove_cheat_engine(key: str) -> None:
    folder = _season_folder(key)
    try:
        write_tl_tools(folder, _without_ce(read_tl_tools(folder)))
    except Exception as e:
        raise OSError(log.fail("Cheat Engine removal failed", e)) from e


def add_cheat_engine(key: str) -> None:
    folder = _season_folder(key)
    exe = _find_ce_exe(_drive_c(key))
    if exe is None:
        raise OSError("Cheat Engine is not installed for this season")
    _add_tool(folder, exe)


_WINE_NOISE = re.compile(r"^(\d+:)?(fixme|warn|trace):")


def install_cheat_engine(key: str, installer: Path, settings: dict) -> None:
    folder = _season_folder(key)
    proton = resolve_proton(settings)
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
        raise OSError(log.fail("Cheat Engine installer could not be started", e)) from e
    exe = _find_ce_exe(_drive_c(key))
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
    _add_tool(folder, exe)
