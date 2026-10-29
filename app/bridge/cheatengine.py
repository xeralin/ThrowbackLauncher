import threading
from pathlib import Path

from PySide6.QtCore import QObject, Signal, Slot

from bridge.dialogs import EXE_FILTER, pick_file
from core import log
from core.cheatengine import (
    add_cheat_engine,
    cheat_engine_status,
    install_cheat_engine,
    remove_cheat_engine,
)


class CheatEngineController(QObject):
    done = Signal()
    error = Signal(str)
    _done_in = Signal(str)

    def __init__(self, settings: dict) -> None:
        super().__init__()
        self._settings = settings
        self._busy = False
        self._done_in.connect(self._on_done)

    def _on_done(self, message: str) -> None:
        self._busy = False
        if message:
            self.error.emit(message)
        self.done.emit()

    @Slot(str, result="QVariantMap")
    def status(self, key: str) -> dict:
        return {**cheat_engine_status(key), "busy": self._busy}

    @Slot(str, result=str)
    def add(self, key: str) -> str:
        try:
            add_cheat_engine(key)
        except OSError as e:
            return str(e)
        return ""

    @Slot(str, result=str)
    def remove(self, key: str) -> str:
        try:
            remove_cheat_engine(key)
        except OSError as e:
            return str(e)
        return ""

    @Slot(str, result=bool)
    def install(self, key: str) -> bool:
        if self._busy:
            return False
        picked = pick_file("Select Cheat Engine installer", EXE_FILTER)
        if not picked:
            return False
        self._busy = True
        threading.Thread(target=self._install, args=(key, Path(picked)), daemon=True).start()
        return True

    def _install(self, key: str, installer: Path) -> None:
        try:
            install_cheat_engine(key, installer, self._settings)
        except OSError as e:
            self._done_in.emit(str(e))
        except Exception as e:
            self._done_in.emit(log.fail("Cheat Engine setup failed", e))
        else:
            self._done_in.emit("")
