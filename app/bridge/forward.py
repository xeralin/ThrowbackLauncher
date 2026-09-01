import json

from PySide6.QtCore import QObject, QTimer
from PySide6.QtWebEngineWidgets import QWebEngineView

_FLUSH_INTERVAL_MS = 30


class EventForwarder(QObject):
    def __init__(self, view: QWebEngineView, target: str) -> None:
        super().__init__(view)
        self._view = view
        self._target = target
        self._pending: list[str] = []
        self._timer = QTimer(self)
        self._timer.setInterval(_FLUSH_INTERVAL_MS)
        self._timer.setSingleShot(True)
        self._timer.timeout.connect(self._flush)

    def _dispatch(self, details: list[str]) -> None:
        js = ";".join(
            f"window.dispatchEvent(new CustomEvent('throwback:event',{{detail:{detail}}}))"
            for detail in details
        )
        self._view.page().runJavaScript(js)

    def _flush(self) -> None:
        if self._pending:
            pending, self._pending = self._pending, []
            self._dispatch(pending)

    def _pend(self, event: str, args: tuple) -> None:
        payload = {"target": self._target, "event": event, "args": list(args)}
        self._pending.append(json.dumps(payload))

    def send(self, event: str, *args: object) -> None:
        self._pend(event, args)
        self._flush()

    def send_buffered(self, event: str, *args: object) -> None:
        self._pend(event, args)
        if not self._timer.isActive():
            self._timer.start()
