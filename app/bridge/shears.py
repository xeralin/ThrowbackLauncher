import threading

from PySide6.QtCore import QObject, Signal, Slot

from core import log
from core.constants import (
    BUSY_MESSAGE,
    EVENT_SEASONS,
    NOT_INSTALLED,
    SEASON_RUNNING,
    TEXTURE_QUALITIES,
)
from core.manifest import installed_path
from core.shears import KINDS, cut_download, scan_download
from core.steam import is_season_running


def _empty() -> dict:
    return {"videos": 0, "events": 0, "tiers": []}


def _fail(key: str, message: str) -> dict:
    return {"key": key, "ok": False, "message": message}


def _serialize(scan: dict) -> dict:
    return {
        "videos": scan["videos"],
        "events": scan["events"],
        "tiers": [
            {"level": level, "quality": TEXTURE_QUALITIES[level], "size": size}
            for level, size in sorted(scan["tiers"].items())
        ],
    }


class ShearsController(QObject):
    scan_done = Signal("QVariantMap")
    cut_done = Signal("QVariantMap")
    _scan_done_in = Signal("QVariantMap")
    _cut_done_in = Signal("QVariantMap")

    def __init__(self, downloader: QObject) -> None:
        super().__init__()
        self._downloader = downloader
        self._busy_key: str | None = None
        self._scan_done_in.connect(self.scan_done)
        self._cut_done_in.connect(self._on_cut_done)

    def busy_key(self) -> str | None:
        return self._busy_key

    def _on_cut_done(self, result: dict) -> None:
        self._busy_key = None
        self.cut_done.emit(result)

    @Slot(str)
    def scan(self, key: str) -> None:
        threading.Thread(target=self._scan_work, args=(key,), daemon=True).start()

    def _scan_work(self, key: str) -> None:
        try:
            path = installed_path(key, False)
            if path is None:
                self._scan_done_in.emit({"key": key, "ok": True, "scan": _empty()})
                return
            scan = _serialize(scan_download(path, EVENT_SEASONS.get(key)))
            self._scan_done_in.emit({"key": key, "ok": True, "scan": scan})
        except Exception as e:
            self._scan_done_in.emit(
                {"key": key, "ok": False, "message": log.fail("Shears failed", e)}
            )

    @Slot(str, str, int)
    def cut(self, key: str, kind: str, level: int) -> None:
        if self._busy_key is not None:
            return
        if kind not in KINDS:
            self.cut_done.emit(_fail(key, "Invalid target"))
            return
        if self._downloader.busy_with(key):
            self.cut_done.emit(_fail(key, BUSY_MESSAGE))
            return
        self._busy_key = key
        threading.Thread(target=self._cut_work, args=(key, kind, level), daemon=True).start()

    def _cut_work(self, key: str, kind: str, level: int) -> None:
        try:
            path = installed_path(key, False)
            if path is None:
                self._cut_done_in.emit(_fail(key, NOT_INSTALLED))
                return
            if is_season_running(key):
                self._cut_done_in.emit(_fail(key, SEASON_RUNNING))
                return
            if kind == "events" and key not in EVENT_SEASONS:
                self._cut_done_in.emit(_fail(key, "Invalid target"))
                return
            cut_download(path, kind, level, EVENT_SEASONS.get(key))
            self._cut_done_in.emit(
                {
                    "key": key,
                    "ok": True,
                    "scan": _serialize(scan_download(path, EVENT_SEASONS.get(key))),
                }
            )
        except Exception as e:
            self._cut_done_in.emit(_fail(key, log.fail("Shears failed", e)))
