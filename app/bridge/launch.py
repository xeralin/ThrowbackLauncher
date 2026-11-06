import subprocess
import threading
import time
from collections.abc import Callable
from functools import partial
from pathlib import Path
from queue import Queue

from PySide6.QtCore import Property, QObject, Signal, Slot

from core import log
from core.constants import (
    BUSY_MESSAGE,
    IS_WINDOWS,
    NO_PROTON,
    NOT_INSTALLED,
    PREFIX_DIR,
    TL_LAUNCHER,
    UPDATE_RUNNING,
)
from core.heatedmetal import hm_build_time, hm_crash_log, hm_files_present
from core.manifest import (
    hm_folder_name,
    installed_path,
    partial_path,
    resolve_install,
)
from core.playtime import read_playtime, write_playtime
from core.steam import (
    expand_launch_args,
    game_started,
    is_season_running,
    prefix_pids,
    proton_env,
    resolve_proton,
    running_game_folders,
    stop_game,
)
from core.throwbackloader import read_tl_args, tl_present, write_tl_args

_SUSPEND_GAP_S = 30.0
_CHECKPOINT_S = 300.0


class LaunchController(QObject):
    error = Signal(str)
    running_changed = Signal("QVariantList")
    launching_changed = Signal(object)
    files_missing = Signal(str, bool, str)
    playtime_changed = Signal()
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
        self._sessions: dict[tuple[str, bool], float] = {}
        self._last_tick = 0.0
        self._playtime_table = read_playtime()
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

    def _on_tick(self, folders: dict[str, list[int]]) -> None:
        pids: dict[tuple[str, bool], list[int]] = {}
        for folder, folder_pids in folders.items():
            resolved = resolve_install(folder, self._downloads)
            if resolved is not None:
                pids[(resolved[0]["key"], resolved[1])] = folder_pids
        pairs = set(pids)
        self._count_playtime(pids)
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
                self._on_fail(
                    log.fail("Launch failed", f"exit={proc.returncode}"), self._launch_gen
                )
        if self._launching is not None:
            self._watchdog.keep_fast()

    def _count_playtime(self, pids: dict[tuple[str, bool], list[int]]) -> None:
        now = time.time()
        if now - self._last_tick > _SUSPEND_GAP_S:
            for pair in self._sessions:
                self._sessions[pair] += now - self._last_tick
        self._last_tick = now
        for pair in self._sessions.keys() - pids.keys():
            self._book(pair, self._sessions.pop(pair), now)
        for pair, start in self._sessions.items():
            if now - start >= _CHECKPOINT_S:
                self._sessions[pair] = self._book(pair, start, now)
        for pair in pids.keys() - self._sessions.keys():
            started = game_started(pids[pair]) or now
            known = self._playtime_entry(*pair)
            until = known.get("until", 0)
            if round(started) > until:
                self._edition(*pair).update(
                    sessions=known.get("sessions", 0) + 1,
                    since=known.get("since", until or round(started)),
                    until=round(started),
                )
                self._write_playtime()
            self._sessions[pair] = max(started, until)

    def _playtime_entry(self, key: str, hm: bool) -> dict[str, int]:
        entry = self._playtime_table.get(key, {})
        entry = entry.get("hm") if hm else entry
        if not isinstance(entry, dict):
            return {}
        return {name: value for name, value in entry.items() if isinstance(value, int)}

    def _edition(self, key: str, hm: bool) -> dict:
        entry = self._playtime_table.setdefault(key, {})
        if not hm:
            return entry
        if not isinstance(entry.get("hm"), dict):
            entry["hm"] = {}
        return entry["hm"]

    def _book(self, pair: tuple[str, bool], start: float, end: float) -> float:
        booked = max(0, round((end - start) / 60))
        until = start + booked * 60
        minutes = self._playtime_entry(*pair).get("minutes", 0) + booked
        self._edition(*pair).update(minutes=minutes, until=round(until))
        self._write_playtime()
        return until

    def _write_playtime(self) -> None:
        ranked = sorted(
            self._playtime_table.items(),
            key=lambda item: self._order.get(item[0], len(self._order)),
        )
        try:
            write_playtime(dict(ranked))
        except OSError as e:
            log.fail("Play time save failed", e)
        self.playtime_changed.emit()

    def save_playtime(self) -> None:
        self._count_playtime({})

    def _running_list(self) -> list:
        order = sorted(self._running_pairs, key=lambda pair: (self._order[pair[0]], pair[1]))
        return [{"key": key, "hm": hm} for key, hm in order]

    def _launching_map(self) -> dict | None:
        if self._launching is None:
            return None
        key, hm = self._launching
        return {"key": key, "hm": hm}

    def _set_launching(self, value: tuple[str, bool] | None) -> None:
        if value != self._launching:
            self._launching = value
            self.launching_changed.emit(self._launching_map())

    @Slot(result="QVariantList")
    def running(self) -> list:
        return self._running_list()

    @Slot(result="QVariant")
    def launching(self) -> dict | None:
        return self._launching_map()

    @Property("QVariantList", notify=playtime_changed)
    def playtime(self) -> list:
        played = []
        for d in self._downloads:
            code = d["label"].partition(" ")[0]
            for hm in (False, True):
                entry = self._playtime_entry(d["key"], hm)
                if entry.get("minutes", 0) > 0:
                    until = entry.get("until", 0)
                    played.append(
                        {
                            "key": d["key"],
                            "code": code,
                            "hm": hm,
                            "minutes": entry["minutes"],
                            "sessions": entry.get("sessions", 0),
                            "lastPlayed": until,
                            "firstPlayed": entry.get("since", until),
                        }
                    )
        return played

    @Slot(str, result="QVariantMap")
    def installs(self, key: str) -> dict:
        def edition(hm: bool) -> dict:
            folder = installed_path(key, hm)
            if folder is not None:
                return {
                    "installed": True,
                    "partial": False,
                    "built": hm_build_time(folder) if hm else 0,
                    "crashLog": hm and bool(hm_crash_log(folder)),
                }
            return {
                "installed": False,
                "partial": partial_path(key, hm) is not None,
                "built": 0,
                "crashLog": False,
            }

        return {"tb": edition(False), "hm": edition(True), "prefix": (PREFIX_DIR / key).is_dir()}

    @Slot(str, bool, result="QVariantMap")
    def game_args(self, key: str, hm: bool) -> dict:
        folder = installed_path(key, hm)
        if folder is None:
            return {"args": "", "renderer": ""}
        renderer = ""
        if not hm:
            if (folder / "RainbowSix_Vulkan.exe").exists():
                renderer = "vulkan"
            elif (folder / "RainbowSix_DX11.exe").exists():
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
            return log.fail("Config.toml edit failed", e)
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
        if not tl_present(folder) or (hm and not hm_files_present(folder)):
            self.files_missing.emit(key, hm, str(folder.parent))
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
            stop_game([pid for pids in running_game_folders().values() for pid in pids])
            launcher = str(folder / ("RainbowSix.exe" if hm else TL_LAUNCHER))
            argv, env = [launcher], None
            if not IS_WINDOWS:
                proton = resolve_proton(self._settings, key=key)
                if proton is None:
                    self._fail_in.emit(NO_PROTON, gen)
                    return
                argv, args_env = expand_launch_args(
                    self._settings, key, [str(proton["binary"]), "run", launcher]
                )
                env = {**proton_env(PREFIX_DIR / key), **args_env}
            proc = subprocess.Popen(
                argv, cwd=str(folder), env=env, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL
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
        threading.Thread(target=proc.wait, daemon=True).start()
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
        if launch_proc is not None:
            self._kill_job(launch_proc)
        if IS_WINDOWS:
            folders = running_game_folders()
            stop_game([pid for name in (key, hm_folder_name(key)) for pid in folders.get(name, ())])
        else:
            self._clear_prefix(key)
        self._watchdog.poke()

    def _on_closed(self, key: str) -> None:
        if IS_WINDOWS:
            return
        if self._launching is not None and self._launching[0] == key:
            return
        if is_season_running(key):
            return
        self._clear_prefix(key)

    def _clear_prefix(self, key: str) -> None:
        stop_game(prefix_pids(PREFIX_DIR / key))
