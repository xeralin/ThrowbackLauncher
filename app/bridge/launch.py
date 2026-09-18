import subprocess
import threading
from collections.abc import Callable
from functools import partial
from pathlib import Path
from queue import Queue

from PySide6.QtCore import QObject, Signal, Slot

from core import log
from core.constants import (
    BUSY_MESSAGE,
    DX11_EXE,
    HM_LAUNCHER,
    IS_WINDOWS,
    NO_PROTON,
    NOT_INSTALLED,
    PREFIX_DIR,
    TL_LAUNCHER,
    UPDATE_RUNNING,
    VULKAN_EXE,
)
from core.manifest import (
    hm_folder_name,
    installed_path,
    partial_path,
    resolve_install,
)
from core.steam import (
    expand_launch_args,
    is_game_running,
    is_season_running,
    prefix_pids,
    proton_env,
    resolve_proton,
    running_game_folders,
    running_game_pids,
    stop_game,
)
from core.throwbackloader import read_tl_args, write_tl_args


class LaunchController(QObject):
    error = Signal(str)
    running_changed = Signal("QVariantList")
    launching_changed = Signal("QVariantMap")
    _fail_in = Signal(str, int)
    _spawned_in = Signal(object, int)

    def __init__(
        self,
        settings: dict,
        downloads: list[dict],
        watchdog: QObject,
        downloader: QObject,
        updater: QObject,
    ) -> None:
        super().__init__()
        self._settings = settings
        self._downloads = downloads
        self._order = {d["key"]: i for i, d in enumerate(downloads)}
        self._watchdog = watchdog
        self._downloader = downloader
        self._updater = updater
        self._running_pairs: set[tuple[str, bool]] = set()
        self._launching: tuple[str, bool] | None = None
        self._launch_proc: subprocess.Popen | None = None
        self._launch_gen = 0
        self._jobs: Queue[Callable[[], None]] = Queue()
        self._fail_in.connect(self._on_fail)
        self._spawned_in.connect(self._on_spawned)
        watchdog.tick.connect(self._on_tick)
        threading.Thread(target=self._work, daemon=True).start()

    def _work(self) -> None:
        while True:
            job = self._jobs.get()
            try:
                job()
            except Exception as e:
                log.fail("Launch job failed", e)

    def _on_tick(self, folders: object) -> None:
        pairs = set()
        for folder in folders:
            resolved = resolve_install(folder, self._downloads)
            if resolved is not None:
                pairs.add((resolved[0]["key"], resolved[1]))
        if pairs != self._running_pairs:
            closed = {key for key, _ in self._running_pairs} - {key for key, _ in pairs}
            self._running_pairs = pairs
            self.running_changed.emit(self._running_list())
            for key in closed:
                self._jobs.put(partial(self._on_closed, key))
        if self._launching is not None:
            proc = self._launch_proc
            if self._launching in pairs and proc is not None:
                self._set_launching(None)
            elif proc is not None and proc.poll() is not None:
                failed = self._launching
                self._set_launching(None)
                self._jobs.put(partial(self._on_closed, failed[0]))
                self.error.emit(log.fail("Launch failed", f"exit={proc.returncode}"))
        if self._launching is not None:
            self._watchdog.keep_fast()

    def _running_list(self) -> list:
        order = sorted(self._running_pairs, key=lambda pair: (self._order[pair[0]], pair[1]))
        return [{"key": key, "hm": hm} for key, hm in order]

    def _launching_map(self) -> dict:
        if self._launching is None:
            return {}
        key, hm = self._launching
        return {"key": key, "hm": hm}

    def _set_launching(self, value: tuple[str, bool] | None) -> None:
        if value != self._launching:
            self._launching = value
            self.launching_changed.emit(self._launching_map())

    @Slot(result="QVariantList")
    def running(self) -> list:
        return self._running_list()

    @Slot(result="QVariantMap")
    def launching(self) -> dict:
        return self._launching_map()

    @Slot(str, result="QVariantMap")
    def installs(self, key: str) -> dict:
        def edition(hm: bool) -> dict:
            if installed_path(key, hm) is not None:
                return {"installed": True, "partial": False}
            return {"installed": False, "partial": partial_path(key, hm) is not None}

        return {"tb": edition(False), "hm": edition(True)}

    @Slot(str, bool, result="QVariantMap")
    def game_args(self, key: str, hm: bool) -> dict:
        folder = installed_path(key, hm)
        if folder is None:
            return {"args": "", "renderer": ""}
        renderer = ""
        if not hm:
            if (folder / VULKAN_EXE).exists():
                renderer = "vulkan"
            elif (folder / DX11_EXE).exists():
                renderer = "dx11"
        return {"args": read_tl_args(folder), "renderer": renderer}

    @Slot(str, bool, str, result=str)
    def set_game_args(self, key: str, hm: bool, value: str) -> str:
        folder = installed_path(key, hm)
        if folder is None:
            return NOT_INSTALLED
        try:
            write_tl_args(folder, value.strip())
        except OSError as e:
            return log.fail("Arguments save failed", e)
        return ""

    @Slot(str, bool)
    def launch(self, key: str, hm: bool) -> None:
        if self._launching is not None:
            self.error.emit("R6S is launching")
            return
        if self._updater.busy:
            self.error.emit(UPDATE_RUNNING)
            return
        if self._downloader.busy_with(key):
            self.error.emit(BUSY_MESSAGE)
            return
        folder = installed_path(key, hm)
        if folder is None:
            self.error.emit(NOT_INSTALLED)
            return
        self._launch_proc = None
        self._launch_gen += 1
        self._set_launching((key, hm))
        self._watchdog.poke()
        self._jobs.put(partial(self._launch_job, key, hm, folder, self._launch_gen))

    def _launch_job(self, key: str, hm: bool, folder: Path, gen: int) -> None:
        if gen != self._launch_gen:
            return
        try:
            if is_game_running():
                stop_game(running_game_pids())
            launcher = folder / (HM_LAUNCHER if hm else TL_LAUNCHER)
            if IS_WINDOWS:
                proc = subprocess.Popen(
                    [str(launcher)],
                    cwd=str(folder),
                    stdout=subprocess.DEVNULL,
                    stderr=subprocess.DEVNULL,
                )
            else:
                proton = resolve_proton(self._settings, key=key)
                if proton is None:
                    self._fail_in.emit(NO_PROTON, gen)
                    return
                argv, args_env = expand_launch_args(
                    self._settings, key, [str(proton["binary"]), "run", str(launcher)]
                )
                proc = subprocess.Popen(
                    argv,
                    cwd=str(folder),
                    env={**proton_env(PREFIX_DIR / key), **args_env},
                    stdout=subprocess.DEVNULL,
                    stderr=subprocess.DEVNULL,
                )
        except Exception as e:
            self._fail_in.emit(log.fail("Launch failed", e), gen)
            return
        self._spawned_in.emit(proc, gen)

    def _on_spawned(self, proc: subprocess.Popen, gen: int) -> None:
        if gen != self._launch_gen:
            self._jobs.put(partial(self._kill_job, proc))
            return
        self._launch_proc = proc
        self._watchdog.poke()

    def _kill_job(self, proc: subprocess.Popen) -> None:
        if proc.poll() is None:
            stop_game([proc.pid])

    def _on_fail(self, message: str, gen: int) -> None:
        if gen != self._launch_gen:
            return
        failed = self._launching
        self._set_launching(None)
        if failed is not None:
            self._jobs.put(partial(self._on_closed, failed[0]))
        self.error.emit(message)

    @Slot(str)
    def stop(self, key: str) -> None:
        launch_proc = None
        if self._launching is not None and self._launching[0] == key:
            self._launch_gen += 1
            self._set_launching(None)
            launch_proc = self._launch_proc
        self._jobs.put(partial(self._stop_job, key, launch_proc))

    def _stop_job(self, key: str, launch_proc: subprocess.Popen | None) -> None:
        if launch_proc is not None and launch_proc.poll() is None:
            stop_game([launch_proc.pid])
        if IS_WINDOWS:
            names = {key, hm_folder_name(key)}
            pids = [
                pid
                for folder, folder_pids in running_game_folders().items()
                if folder in names
                for pid in folder_pids
            ]
            if pids:
                stop_game(pids)
        else:
            self._clear_prefix(key)
        self._watchdog.poke()

    def _on_closed(self, key: str) -> None:
        if self._launching is not None and self._launching[0] == key:
            return
        if is_season_running(key):
            return
        self._clear_prefix(key)

    def _clear_prefix(self, key: str) -> None:
        if IS_WINDOWS:
            return
        pids = prefix_pids(PREFIX_DIR / key)
        if pids:
            stop_game(pids)
