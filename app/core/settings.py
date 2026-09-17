import os
import re
import tomllib
from pathlib import Path

from core import log
from core.constants import DEFAULT_DOWNLOADS_DIR, SETTINGS_FILE

_BARE_KEY = re.compile(r"^[A-Za-z0-9_-]+$")

warning: str | None = None


def load_settings() -> dict:
    global warning
    try:
        with SETTINGS_FILE.open("rb") as f:
            return tomllib.load(f)
    except FileNotFoundError:
        return {}
    except OSError as e:
        warning = log.fail("Settings could not be read", e)
        return {}
    except (tomllib.TOMLDecodeError, UnicodeDecodeError) as e:
        log.fail("Settings were malformed", e)
        broken = SETTINGS_FILE.with_name(SETTINGS_FILE.name + ".broken")
        try:
            SETTINGS_FILE.replace(broken)
            warning = f"Settings were malformed, backup saved as {broken.name}"
        except OSError as replace_error:
            log.fail("Settings backup failed", replace_error)
            warning = "Settings were malformed and reset"
        return {}


def toml_str(value: str) -> str:
    if "'" not in value and not any(ord(c) < 0x20 or c == "\x7f" for c in value):
        return f"'{value}'"
    esc = value.replace("\\", "\\\\").replace('"', '\\"')
    esc = "".join(f"\\u{ord(c):04X}" if ord(c) < 0x20 or c == "\x7f" else c for c in esc)
    return f'"{esc}"'


def _fmt_value(value: object) -> str:
    if isinstance(value, str):
        return toml_str(value)
    if isinstance(value, bool):
        return str(value).lower()
    if isinstance(value, list):
        return f"[{', '.join(_fmt_value(item) for item in value)}]"
    if isinstance(value, dict):
        items = ", ".join(f"{_fmt_key(str(key))} = {_fmt_value(val)}" for key, val in value.items())
        return f"{{ {items} }}"
    return str(value)


def _fmt_key(key: str) -> str:
    return key if _BARE_KEY.fullmatch(key) else _fmt_value(key)


def save_settings(settings: dict) -> None:
    lines = ["[settings]"]
    for k, v in settings.get("settings", {}).items():
        lines.append(f"{_fmt_key(k)} = {_fmt_value(v)}")
    SETTINGS_FILE.parent.mkdir(parents=True, exist_ok=True)
    tmp = SETTINGS_FILE.with_name(SETTINGS_FILE.name + ".tmp")
    with tmp.open("w", encoding="utf-8") as f:
        f.write("\n".join(lines) + "\n")
        f.flush()
        os.fsync(f.fileno())
    tmp.replace(SETTINGS_FILE)


def get_setting[T](settings: dict, key: str, default: T) -> T:
    return settings.get("settings", {}).get(key, default)


def set_setting(settings: dict, key: str, value: object) -> None:
    settings.setdefault("settings", {})[key] = value


def pop_settings(settings: dict, *keys: str) -> bool:
    table = settings.get("settings", {})
    removed = [table.pop(key, None) for key in keys]
    if all(value is None for value in removed):
        return False
    save_settings(settings)
    return True


def _resolve_libraries(values: list[str]) -> list[Path]:
    roots: list[Path] = []
    for value in values:
        if not value:
            continue
        path = Path(value).resolve()
        if path not in roots:
            roots.append(path)
    if DEFAULT_DOWNLOADS_DIR not in roots:
        roots.append(DEFAULT_DOWNLOADS_DIR)
    return roots


_libraries_current: list[Path] = _resolve_libraries([])


def libraries() -> list[Path]:
    return _libraries_current


def default_library() -> Path:
    return libraries()[0]


def set_libraries(values: object) -> None:
    global _libraries_current
    stored = [v for v in values if isinstance(v, str)] if isinstance(values, list) else []
    _libraries_current = _resolve_libraries(stored)
