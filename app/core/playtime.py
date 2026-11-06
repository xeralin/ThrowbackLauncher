import tomllib

from core import log
from core.constants import PLAYTIME_FILE
from core.settings import toml_section, write_atomic


def read_playtime() -> dict[str, dict]:
    try:
        with PLAYTIME_FILE.open("rb") as f:
            data = tomllib.load(f)
    except FileNotFoundError:
        return {}
    except (OSError, ValueError) as e:
        log.fail("Play time read failed", e)
        return {}
    return {key: entry for key, entry in data.items() if isinstance(entry, dict)}


def write_playtime(table: dict[str, dict]) -> None:
    blocks = []
    for key, entry in table.items():
        values = {k: v for k, v in entry.items() if not isinstance(v, dict)}
        if values:
            blocks.append(toml_section([key], values))
        blocks += [toml_section([key, k], v) for k, v in entry.items() if isinstance(v, dict)]
    write_atomic(PLAYTIME_FILE, "\n\n".join(blocks) + "\n")
