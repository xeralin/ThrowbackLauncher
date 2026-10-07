import threading
from pathlib import Path

from PySide6.QtCore import QObject, Signal, Slot

from bridge.dialogs import EXE_FILTER, pick_file
from core import log
from core.cheatengine import (
    CE_MARKER,
    add_cheat_engine,
    install_cheat_engine,
    is_cheat_engine_present,
    remove_cheat_engine,
)
from core.manifest import installed_path
from core.throwbackloader import read_tl_autorun


class CheatEngineController(QObject):
    done = Signal()
    error = Signal(str)
    _done_in = Signal(str)

    def __init__(self, settings: dict) -> None:
        super().__init__()
        self._settings = settings
        self._installer: Path | None = None
        self._busy = False
        self._done_in.connect(self._on_done)

    def _on_done(self, message: str) -> None:
        self._busy = False
        if message:
            self.error.emit(message)
        self.done.emit()

    @Slot(result=str)
    def pick_installer(self) -> str:
        picked = pick_file("Select Cheat Engine installer", EXE_FILTER)
        if picked:
            self._installer = Path(picked)
        return picked

    @Slot(str, result="QVariantMap")
    def status(self, key: str) -> dict:
        folder = installed_path(key, False)
        enabled = folder is not None and any(
            CE_MARKER in t.lower() for t in read_tl_autorun(folder)
        )
        return {"enabled": enabled, "present": is_cheat_engine_present(key), "busy": self._busy}

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

    @Slot(str)
    def install(self, key: str) -> None:
        if self._busy or self._installer is None:
            return
        self._busy = True
        threading.Thread(target=self._install, args=(key, self._installer), daemon=True).start()

    def _install(self, key: str, installer: Path) -> None:
        try:
            install_cheat_engine(key, installer, self._settings)
        except OSError as e:
            self._done_in.emit(str(e))
        except Exception as e:
            self._done_in.emit(log.fail("Cheat Engine setup failed", e))
        else:
            self._done_in.emit("")
