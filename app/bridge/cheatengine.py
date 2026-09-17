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
from core.manifest import installed_tb_downloads
from core.throwbackloader import read_tl_tools


class CheatEngineController(QObject):
    done = Signal(bool, str)
    _done_in = Signal(bool, str)

    def __init__(self, settings: dict, downloads: list[dict]) -> None:
        super().__init__()
        self._settings = settings
        self._downloads = downloads
        self._installer: Path | None = None
        self._busy = False
        self._done_in.connect(self._on_done)

    def _on_done(self, ok: bool, message: str) -> None:
        self._busy = False
        self.done.emit(ok, message)

    @Slot(result=str)
    def pick_installer(self) -> str:
        picked = pick_file("Choose the Cheat Engine installer", EXE_FILTER)
        if picked:
            self._installer = Path(picked)
        return picked

    @Slot(result="QVariantList")
    def seasons(self) -> list:
        result = []
        for folder, download in installed_tb_downloads(self._downloads):
            key = download["key"]
            has_ce = any(CE_MARKER in t.lower() for t in read_tl_tools(folder))
            result.append(
                {
                    "key": key,
                    "label": download["label"],
                    "hasCe": has_ce,
                    "present": is_cheat_engine_present(key),
                }
            )
        return result

    @Slot(str, result="QVariantMap")
    def add(self, key: str) -> dict:
        try:
            add_cheat_engine(key)
        except OSError as e:
            return {"ok": False, "message": str(e)}
        except Exception as e:
            return {"ok": False, "message": log.fail("Cheat Engine setup failed", e)}
        return {"ok": True, "message": ""}

    @Slot(str, result="QVariantMap")
    def remove(self, key: str) -> dict:
        try:
            remove_cheat_engine(key)
        except OSError as e:
            return {"ok": False, "message": str(e)}
        except Exception as e:
            return {"ok": False, "message": log.fail("Cheat Engine removal failed", e)}
        return {"ok": True, "message": ""}

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
            self._done_in.emit(False, str(e))
        except Exception as e:
            self._done_in.emit(False, log.fail("Cheat Engine setup failed", e))
        else:
            self._done_in.emit(True, "")
