from collections.abc import Callable
from dataclasses import dataclass


def _ignore(_: object) -> None:
    pass


@dataclass
class Reporter:
    progress: Callable[[float], None] = _ignore
    fail: Callable[[str], None] = _ignore
    update: Callable[[str], None] = _ignore
