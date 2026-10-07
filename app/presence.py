import threading
import time

from PySide6.QtCore import QObject

from core.manifest import resolve_install
from core.rpc import Presence, build_activity

POLL_INTERVAL = 10
MAX_RECONNECT_DELAY = 30


class PresenceController:
    def __init__(self, downloads: list[dict], watchdog: QObject) -> None:
        self._downloads = downloads
        self._folder: str | None = None
        self._wake = threading.Event()
        self._stop = threading.Event()
        self._thread: threading.Thread | None = None
        watchdog.tick.connect(self._on_tick)

    def set_enabled(self, enabled: bool) -> None:
        if not enabled:
            self.stop()
            return
        if self._thread is not None and self._thread.is_alive():
            return
        self._stop = threading.Event()
        self._wake = threading.Event()
        self._thread = threading.Thread(
            target=self._run, args=(self._stop, self._wake), name="discord-rpc", daemon=True
        )
        self._thread.start()

    def stop(self) -> None:
        self._stop.set()
        self._wake.set()
        self._thread = None

    def _on_tick(self, folders: object) -> None:
        folder = next(iter(folders), None)
        if folder != self._folder:
            self._folder = folder
            self._wake.set()

    @staticmethod
    def _sleep(wake: threading.Event, timeout: float | None) -> None:
        wake.wait(timeout)
        wake.clear()

    @staticmethod
    def _drop(presence: Presence | None) -> None:
        if presence is not None:
            presence.close()

    def _run(self, stop: threading.Event, wake: threading.Event) -> None:
        presence: Presence | None = None
        folder: str | None = None
        client_id = ""
        activity: dict | None = None
        sent = False
        delay = 1
        try:
            while not stop.is_set():
                if self._folder != folder:
                    folder = self._folder
                    resolved = resolve_install(folder, self._downloads) if folder else None
                    new_id, activity = (
                        build_activity(*resolved, int(time.time())) if resolved else ("", None)
                    )
                    if new_id != client_id or activity is None:
                        self._drop(presence)
                        presence = None
                        client_id = new_id
                    sent = False
                    delay = 1
                if presence is not None:
                    presence.drain()
                if activity is not None:
                    if presence is None or not presence.connected:
                        presence = Presence(client_id)
                        if not presence.connect():
                            presence = None
                            wait, delay = delay, min(delay * 2, MAX_RECONNECT_DELAY)
                            self._sleep(wake, wait)
                            continue
                        delay = 1
                        sent = False
                    if not sent:
                        sent = presence.set(activity)
                self._sleep(wake, POLL_INTERVAL if presence is not None else None)
        finally:
            self._drop(presence)
