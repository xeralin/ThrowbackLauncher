import threading
from pathlib import Path

from PySide6.QtCore import QObject, Signal, Slot

from bridge.dialogs import EXE_FILTER, pick_file
from bridge.reporter import SignalReporter
from bridge.slots import deferred_slot
from core import log
from core.github import CancelledError, RateLimitError
from core.rvpn import (
    Session,
    ensure_wine,
    is_installed,
    is_radmin_installer,
    service_version,
    uninstall,
)

_SHUTDOWN_TIMEOUT_S = 10


class RvpnController(QObject):
    state_changed = Signal("QVariantMap")
    error = Signal(str)
    _state_in = Signal("QVariantMap")
    _step_in = Signal(str)
    _error_in = Signal(str)
    _idle_in = Signal()

    def __init__(self) -> None:
        super().__init__()
        self._session: Session | None = None
        self._thread: threading.Thread | None = None
        self._installer: Path | None = None
        self._busy = False
        self._cancel = threading.Event()
        self._pending_error = ""
        self._attached = False
        self._state = self._make_state("idle")
        self._state_in.connect(self._store_and_emit)
        self._step_in.connect(self._store_step)
        self._error_in.connect(self._on_error_in)
        self._idle_in.connect(self._on_idle_in)

    def _make_state(self, state: str, step: str = "") -> dict:
        return {
            "state": state,
            "installed": is_installed(),
            "hasInstaller": bool(self._installer and self._installer.is_file()),
            "step": step,
            "version": service_version(),
        }

    def _store_step(self, text: str) -> None:
        self._state["step"] = text
        self.state_changed.emit(self._state)

    def _store_and_emit(self, state: dict) -> None:
        self._state = state
        self.state_changed.emit(state)

    @Slot(result="QVariantMap")
    def snapshot(self) -> dict:
        return self._state

    @Slot()
    def take_startup_error(self) -> None:
        self._attached = True
        if self._pending_error:
            text, self._pending_error = self._pending_error, ""
            self.error.emit(text)

    @deferred_slot()
    def select_installer(self) -> None:
        picked = pick_file("Choose the Radmin VPN installer", EXE_FILTER)
        if not picked:
            return
        if not is_radmin_installer(picked):
            self.error.emit(log.fail("This is not a Radmin VPN installer", picked))
            return
        self._installer = Path(picked)
        self.run()

    @Slot()
    def run(self) -> None:
        if self._busy:
            return
        self._cancel.clear()
        self._busy = True
        self._store_and_emit(self._make_state("building"))
        self._thread = threading.Thread(target=self._run, daemon=True)
        self._thread.start()

    @Slot()
    def stop(self) -> None:
        self._cancel.set()
        session = self._session
        if session is not None:
            session.stop()

    def shutdown(self) -> None:
        self.stop()
        thread = self._thread
        if thread is not None:
            thread.join(timeout=_SHUTDOWN_TIMEOUT_S)

    @Slot()
    def uninstall(self) -> None:
        if self._busy:
            return
        self._cancel.clear()
        self._busy = True
        self._store_and_emit(self._make_state("uninstalling"))
        self._thread = threading.Thread(target=self._uninstall, daemon=True)
        self._thread.start()

    def _uninstall(self) -> None:
        try:
            uninstall()
            if not is_installed():
                self._installer = None
        except OSError as e:
            self._error_in.emit(log.fail("Radmin VPN removal failed", e))
        finally:
            self._idle_in.emit()

    def _run(self) -> None:
        reporter = SignalReporter(step_emit=self._step_in.emit, fail_emit=self._error_in.emit)
        try:
            ensure_wine(reporter, self._cancel.is_set)
            if self._cancel.is_set():
                return
            self._session = Session()
            if self._cancel.is_set():
                return
            self._session.run(self._installer, reporter, on_running=self._on_running)
        except CancelledError:
            pass
        except RateLimitError as e:
            self._error_in.emit(e.message())
        except OSError as e:
            self._error_in.emit(str(e))
        except Exception as e:
            self._error_in.emit(log.fail("Radmin VPN failed", e))
        finally:
            self._session = None
            self._idle_in.emit()

    def _on_error_in(self, text: str) -> None:
        if self._cancel.is_set():
            return
        if self._attached:
            self.error.emit(text)
        else:
            self._pending_error = text

    def _on_idle_in(self) -> None:
        self._busy = False
        self._store_and_emit(self._make_state("idle"))

    def _on_running(self) -> None:
        self._state_in.emit(self._make_state("running"))
