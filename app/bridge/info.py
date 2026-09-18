import threading
from pathlib import Path

from PySide6.QtCore import QObject, QUrl, Signal, Slot
from PySide6.QtGui import QDesktopServices

from core import log, settings
from core.constants import GIB
from core.manifest import installed_path, local_downloads
from core.shears import folder_size


class InfoController(QObject):
    disk_usage_changed = Signal(float)
    _usage_in = Signal(float)

    def __init__(self) -> None:
        super().__init__()
        self._scanning = False
        self._rescan = False
        self._usage_in.connect(self._on_disk_usage)

    @Slot(result="QVariantMap")
    def snapshot(self) -> dict:
        return {"warning": settings.warning}

    @Slot()
    def refresh_disk_usage(self) -> None:
        if self._scanning:
            self._rescan = True
            return
        self._scanning = True
        threading.Thread(target=self._emit_disk_usage, daemon=True).start()

    def _emit_disk_usage(self) -> None:
        total = sum(folder_size(folder) for folder in local_downloads())
        self._usage_in.emit(round(total / GIB, 1))

    def _on_disk_usage(self, value: float) -> None:
        self._scanning = False
        self.disk_usage_changed.emit(value)
        if self._rescan:
            self._rescan = False
            self.refresh_disk_usage()

    @Slot(str)
    def open_library(self, path: str) -> None:
        target = Path(path)
        if target not in settings.libraries() or not target.exists():
            return
        if not QDesktopServices.openUrl(QUrl.fromLocalFile(str(target))):
            log.fail("Folder open failed", target)

    @Slot(str, bool)
    def open_season(self, key: str, hm: bool) -> None:
        target = installed_path(key, hm)
        if target is None:
            return
        if not QDesktopServices.openUrl(QUrl.fromLocalFile(str(target))):
            log.fail("Folder open failed", target)
