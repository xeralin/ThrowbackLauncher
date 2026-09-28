from pathlib import Path

from core import log
from core.constants import ERRORS_FILE, PREFIX_DIR
from core.settings import get_setting, libraries, save_settings, set_setting
from core.steam import running_game_folders

_RENAMED_KEYS = {"Y10S2_DayBreak": "Y10S2_Daybreak"}
_KEYED_SETTINGS = ("home_order", "home_sizes", "launch_args", "season_proton", "queue")


def _has_entry(parent: Path, name: str) -> bool:
    try:
        return any(entry.name == name for entry in parent.iterdir())
    except OSError:
        return False


def _rename(parent: Path, old: str, new: str) -> bool:
    if not _has_entry(parent, old) or _has_entry(parent, new):
        return True
    try:
        (parent / old).rename(parent / new)
    except OSError as e:
        log.fail("Season rename failed", e)
        return False
    return True


def _renamed(value: object) -> object:
    if isinstance(value, str):
        return _RENAMED_KEYS.get(value, value)
    if isinstance(value, list):
        return [_renamed(item) for item in value]
    if isinstance(value, dict):
        return {_RENAMED_KEYS.get(k, k): _renamed(v) for k, v in value.items()}
    return value


def _migrate_errors_file() -> None:
    old = ERRORS_FILE.with_name("log.txt")
    if not old.exists():
        return
    try:
        if ERRORS_FILE.exists():
            old.unlink()
        else:
            old.rename(ERRORS_FILE)
    except OSError as e:
        log.fail("Errors file migration failed", e)


def migrate(settings: dict) -> None:
    _migrate_errors_file()
    for old, new in _RENAMED_KEYS.items():
        roots = [root for root in libraries() if _has_entry(root, old)]
        if not roots and not _has_entry(PREFIX_DIR, old):
            continue
        if old in running_game_folders():
            continue
        failed = [root for root in roots if not _rename(root, old, new)]
        if not failed:
            _rename(PREFIX_DIR, old, new)
    changed = False
    for name in _KEYED_SETTINGS:
        value = get_setting(settings, name, None)
        renamed = _renamed(value)
        if renamed != value:
            set_setting(settings, name, renamed)
            changed = True
    if changed:
        save_settings(settings)
