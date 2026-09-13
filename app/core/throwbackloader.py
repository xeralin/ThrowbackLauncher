import re
import shutil
import struct
import tomllib
import zipfile
from pathlib import Path

from core import log
from core.constants import (
    DEFAULTARGS_DLL,
    HM_FOLDER_SUFFIX,
    TL_API_URL,
    TL_DIR,
    TL_DLLS_COMMON,
    TL_EXTRACT,
    TL_LAUNCHER,
    TL_LOADERS,
    TL_TOML,
)
from core.github import RateLimitError, fetch_to, github_asset
from core.reporter import NullReporter, Reporter
from core.settings import toml_str

TL_USERNAME_RX = re.compile(r"""username\s*=\s*["']([^"']*)["']""")


def tl_present() -> bool:
    return all((TL_DIR / f).exists() for f in TL_EXTRACT)


def pe_file_version(path: Path) -> str | None:
    try:
        data = path.read_bytes()
    except OSError:
        return None
    at = 0
    while (at := data.find(b"\xbd\x04\xef\xfe", at)) != -1:
        if at + 16 <= len(data):
            struc_version, ms, ls = struct.unpack_from("<III", data, at + 4)
            if struc_version == 0x00010000:
                return f"{ms >> 16}.{ms & 0xFFFF}.{ls >> 16}"
        at += 4
    return None


def tl_folder_version(folder: Path) -> str | None:
    return pe_file_version(folder / TL_LAUNCHER)


def tl_version() -> str | None:
    return tl_folder_version(TL_DIR)


def ensure_tl(reporter: Reporter | None = None, force: bool = False) -> None:
    if tl_present() and not force:
        return

    reporter = reporter or NullReporter()
    reporter.update("Fetching ThrowbackLoader")
    TL_DIR.mkdir(parents=True, exist_ok=True)
    zip_path = TL_DIR / "tl.zip"
    tmp_dir = TL_DIR / ".tl.tmp"
    try:
        _, asset_url = github_asset(TL_API_URL, ".zip")
        fetch_to(asset_url, zip_path, on_progress=reporter.progress)
        shutil.rmtree(tmp_dir, ignore_errors=True)
        with zipfile.ZipFile(zip_path) as z:
            for name in TL_EXTRACT:
                z.extract(name, tmp_dir)
        for name in TL_EXTRACT:
            (tmp_dir / name).replace(TL_DIR / name)
    except RateLimitError:
        raise
    except Exception as e:
        raise OSError(log.fail("ThrowbackLoader download failed", e)) from e
    finally:
        zip_path.unlink(missing_ok=True)
        shutil.rmtree(tmp_dir, ignore_errors=True)


def write_tl_toml(target_dir: Path, username: str) -> None:
    config = target_dir / TL_TOML
    config.write_text(
        TL_USERNAME_RX.sub(
            lambda _: f"username = {toml_str(username)}",
            config.read_text(encoding="utf-8"),
            count=1,
        ),
        encoding="utf-8",
    )


def _launch_table(target_dir: Path) -> dict:
    try:
        data = tomllib.loads((target_dir / TL_TOML).read_text(encoding="utf-8"))
    except OSError, tomllib.TOMLDecodeError:
        return {}
    return data.get("Launch", {})


def read_tl_tools(target_dir: Path) -> list[str]:
    tools = _launch_table(target_dir).get("tools", [])
    return [t for t in tools if isinstance(t, str)]


def _write_launch_line(target_dir: Path, key: str, value: str) -> None:
    config = target_dir / TL_TOML
    line = f"{key} = {value}"
    text = config.read_text(encoding="utf-8")
    updated, count = re.subn(rf"(?m)^{key}\s*=.*$", lambda _: line, text, count=1)
    if not count:
        updated, count = re.subn(
            r"(?m)^\[Launch\]\s*$", lambda m: f"{m.group(0)}\n{line}", text, count=1
        )
    if not count:
        raise OSError("Config.toml has no [Launch] section")
    config.write_text(updated, encoding="utf-8")


def write_tl_tools(target_dir: Path, tools: list[str]) -> None:
    _write_launch_line(target_dir, "tools", "[" + ", ".join(toml_str(t) for t in tools) + "]")


def read_tl_args(target_dir: Path) -> str:
    args = _launch_table(target_dir).get("args", "")
    return args if isinstance(args, str) else ""


def write_tl_args(target_dir: Path, args: str) -> None:
    _write_launch_line(target_dir, "args", toml_str(args))


def apply_tl(target_dir: Path, username: str) -> None:
    hm = target_dir.name.endswith(HM_FOLDER_SUFFIX)
    for name in (*TL_DLLS_COMMON, *TL_LOADERS):
        if hm and name == DEFAULTARGS_DLL:
            continue
        shutil.copy2(TL_DIR / name, target_dir / name)
    if not (target_dir / TL_TOML).exists():
        shutil.copy2(TL_DIR / TL_TOML, target_dir / TL_TOML)
    write_tl_toml(target_dir, username)


def write_launcher(target_dir: Path) -> None:
    shutil.copy2(TL_DIR / TL_LAUNCHER, target_dir / TL_LAUNCHER)
