import threading

from PySide6.QtCore import QObject, Signal, Slot

from core import log
from core.constants import BUSY_MESSAGE, UPDATE_RUNNING
from core.steam import uninstall


class UninstallController(QObject):
    done = Signal(bool, str)
    _done_in = Signal(bool, str)

    def __init__(self, downloader: QObject, updater: QObject) -> None:
        super().__init__()
        self._downloader = downloader
        self._updater = updater
        self._busy: tuple[str, bool] | None = None
        self._done_in.connect(self._on_done)

    def busy_key(self) -> str | None:
        return self._busy[0] if self._busy is not None else None

    def _on_done(self, ok: bool, message: str) -> None:
        busy = self._busy
        self._busy = None
        if ok and busy is not None:
            self._downloader.dequeue(*busy)
        self.done.emit(ok, message)

    @Slot(str, bool)
    def run(self, key: str, hm: bool) -> None:
        if self._busy is not None:
            return
        if self._downloader.busy_with(key):
            self.done.emit(False, BUSY_MESSAGE)
            return
        if self._updater.busy:
            self.done.emit(False, UPDATE_RUNNING)
            return
        self._busy = (key, hm)

        def work() -> None:
            try:
                message = uninstall(key, hm)
            except Exception as e:
                message = log.fail("Uninstall failed", e)
            self._done_in.emit(not message, message)

        threading.Thread(target=work, daemon=True).start()
