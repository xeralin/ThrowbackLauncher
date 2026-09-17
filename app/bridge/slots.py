from collections.abc import Callable
from functools import wraps

from PySide6.QtCore import QTimer, Slot


def deferred_slot(*types: object) -> Callable:
    def wrap(fn: Callable) -> Callable:
        @Slot(*types)
        @wraps(fn)
        def slot(self: object, *args: object) -> None:
            QTimer.singleShot(0, lambda: fn(self, *args))

        return slot

    return wrap
