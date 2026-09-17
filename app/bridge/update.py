import os
import subprocess
import threading
from dataclasses import asdict

from PySide6.QtCore import QCoreApplication, QObject, QTimer, Signal, Slot

from bridge.reporter import SignalReporter
from core import log
from core import update as update_backend
from core.constants import (
    CACHE_CLEARING,
    DOWNLOAD_RUNNING,
    IS_WINDOWS,
    SELF_UPDATABLE,
    UNINSTALL_RUNNING,
)
from core.github import RateLimitError
from core.self_update import take_outcome
from layout import APP_NAME

_RESTART_DELAY_MS = 800


class UpdateController(QObject):
    changed = Signal()
    progress = Signal(int)
    done = Signal(bool, str, str)
    error = Signal(str)
    _check_done_in = Signal(object, str, str)
    _apply_done_in = Signal(bool, str, str, bool)
    _progress_in = Signal(float)

    def __init__(self) -> None:
        super().__init__()
        self._busy = False
        self._checking = False
        self._pending: list[update_backend.Component] = []
        self._components: list[dict] = []
        self._check_error = ""
        self._check_detail = ""
        self._progress = 0
        self._applying = ""
        self._outcome = take_outcome()
        self._check_done_in.connect(self._on_check_done)
        self._apply_done_in.connect(self._on_apply_done)
        self._progress_in.connect(self._emit_progress)

    def set_peers(
        self, settings_bridge: QObject, downloader: QObject, uninstaller: QObject
    ) -> None:
        self._settings_bridge = settings_bridge
        self._downloader = downloader
        self._uninstaller = uninstaller

    @property
    def busy(self) -> bool:
        return self._busy

    @Slot()
    def take_startup_outcome(self) -> None:
        if self._outcome is None:
            return
        outcome, self._outcome = self._outcome, None
        message = "" if outcome else f"{APP_NAME} update failed, previous version kept"
        self.done.emit(outcome, APP_NAME, message)

    @Slot(result="QVariantMap")
    def snapshot(self) -> dict:
        return {
            "busy": self._busy,
            "checking": self._checking,
            "components": self._components,
            "checkError": self._check_error,
            "checkErrorDetail": self._check_detail,
            "progress": self._progress,
            "applying": self._applying,
            "selfUpdatable": SELF_UPDATABLE,
        }

    def _emit_progress(self, fraction: float) -> None:
        pct = min(int(fraction * 100), 100)
        if pct != self._progress:
            self._progress = pct
            self.progress.emit(pct)

    @Slot(result="QVariantList")
    def installed(self) -> list:
        return [
            {"name": c.name, "version": version}
            for c in update_backend.COMPONENTS
            if (version := c.current())
        ]

    @Slot(bool)
    def check(self, force: bool = False) -> None:
        if self._busy or self._checking:
            return
        self._checking = True
        self.changed.emit()
        threading.Thread(target=lambda: self._check_work(force), daemon=True).start()

    def _check_work(self, force: bool = False) -> None:
        try:
            pending, error, detail = update_backend.available(force)
        except Exception as e:
            log.fail("Update check failed", e)
            pending, error, detail = [], "error", ""
        self._check_done_in.emit(pending, error, detail)

    def _on_check_done(self, pending: list, error: str, detail: str) -> None:
        if not error or pending:
            self._pending = [component for component, _, _ in pending]
            self._components = [
                {
                    "name": component.name,
                    "target": latest,
                    **asdict(notes),
                }
                for component, latest, notes in pending
            ]
        self._check_error = error
        self._check_detail = detail
        self._checking = False
        self.changed.emit()

    @Slot(str)
    def apply(self, name: str) -> None:
        component = next((c for c in self._pending if c.name == name), None)
        if self._busy or self._checking or component is None:
            return
        if self._settings_bridge.clearing_cache():
            self.error.emit(CACHE_CLEARING)
            return
        if self._downloader.running:
            self.error.emit(DOWNLOAD_RUNNING)
            return
        if self._uninstaller.busy_key() is not None:
            self.error.emit(UNINSTALL_RUNNING)
            return
        self._busy = True
        self._applying = name
        self._progress = 0
        self.progress.emit(0)
        self.changed.emit()
        threading.Thread(target=lambda: self._apply_work(component), daemon=True).start()

    def _apply_work(self, component: update_backend.Component) -> None:
        message = ""
        fail_text = ""

        def on_fail(text: str) -> None:
            nonlocal fail_text
            fail_text = text

        try:
            ok = bool(
                component.apply(
                    reporter=SignalReporter(progress_emit=self._progress_in.emit, fail_emit=on_fail)
                )
            )
        except RateLimitError as e:
            ok = False
            message = e.message()
        except OSError as e:
            ok = False
            message = str(e)
        except Exception as e:
            ok = False
            message = log.fail(f"{component.name} update failed", e)
        if not ok and not message:
            message = fail_text
        self._apply_done_in.emit(ok, component.name, message, component.restart)

    def _on_apply_done(self, ok: bool, name: str, message: str, restart: bool) -> None:
        if ok and restart:
            QTimer.singleShot(_RESTART_DELAY_MS, self._restart)
            return
        self._checking = True
        self._busy = False
        self._applying = ""
        self.changed.emit()
        self.done.emit(ok, name, message)
        threading.Thread(target=self._check_work, daemon=True).start()

    def _restart(self) -> None:
        if not IS_WINDOWS:
            appimage = os.environ.get("APPIMAGE", "")
            if appimage:
                subprocess.Popen(
                    [appimage, "--relaunch"],
                    start_new_session=True,
                    stdout=subprocess.DEVNULL,
                    stderr=subprocess.DEVNULL,
                )
        QCoreApplication.quit()
