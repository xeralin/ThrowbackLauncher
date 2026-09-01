import threading
import time

from PySide6.QtCore import QObject, QTimer, Signal

from core import log
from core.steam import running_game_folders

SCAN_INTERVAL_MS = 2000
FAST_WINDOW_S = 15.0


class Watchdog(QObject):
    tick = Signal(object)
    _scanned_in = Signal(object)
    _poked_in = Signal()

    def __init__(self) -> None:
        super().__init__()
        self._folders: dict[str, list[int]] = {}
        self._fast_until = 0.0
        self._scanning = False
        self._rescan = False
        self._scan_failed = False
        self._timer = QTimer(self)
        self._timer.timeout.connect(self._scan)
        self._scanned_in.connect(self._on_scanned)
        self._poked_in.connect(self._scan)

    def start(self) -> None:
        self._scan()

    def poke(self) -> None:
        self.keep_fast()
        self._poked_in.emit()

    def keep_fast(self) -> None:
        self._fast_until = time.monotonic() + FAST_WINDOW_S

    def folders(self) -> dict[str, list[int]]:
        return self._folders

    def _scan(self) -> None:
        if self._scanning:
            self._rescan = True
            return
        self._scanning = True
        threading.Thread(target=self._scan_worker, daemon=True).start()

    def _scan_worker(self) -> None:
        try:
            folders = running_game_folders()
        except Exception as e:
            if not self._scan_failed:
                self._scan_failed = True
                log.fail("Game scan failed", e)
            self.keep_fast()
            folders = {}
        self._scanned_in.emit(folders)

    def _on_scanned(self, folders: dict) -> None:
        self._scanning = False
        fast = bool(folders) or bool(self._folders) or time.monotonic() < self._fast_until
        if fast:
            self._timer.start(SCAN_INTERVAL_MS)
        else:
            self._timer.stop()
        self._folders = folders
        self.tick.emit(folders)
        if self._rescan:
            self._rescan = False
            self._scan()
