import contextlib
import json
import os
import socket
import subprocess
import threading
import time
from pathlib import Path

from PySide6.QtCore import QObject, Signal, Slot

from core import log
from core.constants import (
    IS_WINDOWS,
    LIBERATOR_BIN,
    NO_PROTON,
    STEAM_DIR,
)
from core.steam import resolve_proton, running_game_env

_ATTACH_DELAY = 1.5
_ATTACH_DELAY_MAX = 30.0


class LiberatorController(QObject):
    state_changed = Signal(object)
    tree_changed = Signal(object)
    error = Signal(str)
    _state_in = Signal(object, object)
    _tree_in = Signal(object, object)
    _kill_in = Signal(object)
    _error_in = Signal(str)

    def __init__(self, settings: dict, watchdog: QObject) -> None:
        super().__init__()
        self._settings = settings
        self._watchdog = watchdog
        self._proc: subprocess.Popen | None = None
        self._sock: socket.socket | None = None
        self._send_lock = threading.Lock()
        self._busy = False
        self._active = False
        self._attach_keys: set[str] = set()
        self._attach_delay = _ATTACH_DELAY
        self._next_attach = 0.0
        self._last_state: dict = self._default_state()
        self._last_tree: object = None
        self._state_in.connect(self._on_state)
        self._tree_in.connect(self._on_tree)
        self._kill_in.connect(self._on_kill)
        self._error_in.connect(self.error)
        watchdog.tick.connect(self._on_tick)

    def _default_state(self) -> dict:
        return {
            "attached": False,
            "applied": False,
            "status": "",
            "available": LIBERATOR_BIN.exists(),
            "capabilities": {},
        }

    @Slot(result="QVariantMap")
    def snapshot(self) -> dict:
        return {**self._last_state, "available": LIBERATOR_BIN.exists()}

    @Slot(result="QVariant")
    def tree_snapshot(self) -> object:
        return self._last_tree

    def start(self) -> None:
        self._active = True
        self._attach_delay = _ATTACH_DELAY
        self._next_attach = 0.0
        self._on_tick(self._watchdog.folders())

    def stop(self) -> None:
        self._active = False
        self._kill()

    @Slot(str, bool)
    def set_mod(self, mod: str, enabled: bool) -> None:
        self._send(cmd="setMod", mod=mod, enabled=enabled)

    @Slot(str)
    def set_playlist(self, playlist_id: str) -> None:
        self._send(cmd="setPlaylist", playlistId=playlist_id)

    @Slot()
    def end_round(self) -> None:
        self._send(cmd="endRound")

    @Slot()
    def end_match(self) -> None:
        self._send(cmd="endMatch")

    def _on_tick(self, folders: dict) -> None:
        if not self._active:
            return
        keys = set(folders)
        if not keys:
            self._attach_delay = _ATTACH_DELAY
            self._next_attach = 0.0
            if self._sock is not None or self._proc is not None:
                self._kill()
            return
        if not (keys & self._attach_keys) and (self._sock is not None or self._proc is not None):
            self._attach_delay = _ATTACH_DELAY
            self._next_attach = 0.0
            self._kill()
        if self._sock is None and not self._busy:
            if time.monotonic() < self._next_attach:
                return
            self._busy = True
            self._attach_keys = keys
            threading.Thread(target=self._attach, daemon=True).start()

    def _attach(self) -> None:
        listener: socket.socket | None = None
        try:
            if not LIBERATOR_BIN.exists():
                return

            listener = socket.socket()
            listener.bind(("127.0.0.1", 0))
            listener.listen(1)
            port = str(listener.getsockname()[1])

            if IS_WINDOWS:
                self._proc = subprocess.Popen(
                    [str(LIBERATOR_BIN), "--runtime", port],
                    stdout=subprocess.DEVNULL,
                    stderr=subprocess.DEVNULL,
                )
            else:
                env_game = running_game_env()
                if env_game is None:
                    return
                prefix = env_game["STEAM_COMPAT_DATA_PATH"]
                proton = resolve_proton(self._settings, key=Path(prefix).name)
                if proton is None:
                    self._error_in.emit(NO_PROTON)
                    return
                env = dict(os.environ)
                env["STEAM_COMPAT_DATA_PATH"] = prefix
                client_install = env_game.get("STEAM_COMPAT_CLIENT_INSTALL_PATH")
                env["STEAM_COMPAT_CLIENT_INSTALL_PATH"] = client_install or str(STEAM_DIR)
                self._proc = subprocess.Popen(
                    [
                        str(proton["binary"]),
                        "run",
                        "Z:" + str(LIBERATOR_BIN).replace("/", "\\"),
                        "--runtime",
                        port,
                    ],
                    env=env,
                    stdout=subprocess.DEVNULL,
                    stderr=subprocess.DEVNULL,
                )

            sock = self._await_client(listener)
            if sock is None:
                self._kill_in.emit(None)
                return
            sock.settimeout(None)
            self._sock = sock
            threading.Thread(target=self._reader, args=(sock,), daemon=True).start()
        except Exception as e:
            self._error_in.emit(log.fail("Attach failed", e))
            self._kill_in.emit(None)
        finally:
            if listener is not None:
                listener.close()
            if not self._active:
                self._kill_in.emit(None)
            if self._sock is None:
                self._bump_backoff()
            else:
                self._attach_delay = _ATTACH_DELAY
            self._busy = False

    def _bump_backoff(self) -> None:
        self._next_attach = time.monotonic() + self._attach_delay
        self._attach_delay = min(self._attach_delay * 2, _ATTACH_DELAY_MAX)

    def _await_client(self, listener: socket.socket) -> socket.socket | None:
        listener.settimeout(0.2)
        while self._active:
            proc = self._proc
            if proc is None:
                return None
            if proc.poll() is not None:
                log.fail("Liberator Runtime exited", f"exit={proc.returncode}")
                return None
            try:
                return listener.accept()[0]
            except TimeoutError:
                pass
        return None

    def _reader(self, sock: socket.socket) -> None:
        buffer = b""
        try:
            while True:
                data = sock.recv(4096)
                if not data:
                    break
                buffer += data
                while b"\n" in buffer:
                    line, buffer = buffer.split(b"\n", 1)
                    if not line.strip():
                        continue
                    try:
                        msg = json.loads(line.decode("utf-8"))
                    except ValueError:
                        continue
                    if msg.get("event") == "state":
                        del msg["event"]
                        msg["available"] = True
                        self._state_in.emit(msg, sock)
                    elif msg.get("event") == "tree":
                        self._tree_in.emit(msg.get("tree"), sock)
        except OSError:
            pass
        if sock is self._sock:
            self._bump_backoff()
            self._kill_in.emit(sock)

    def _send(self, **payload: object) -> None:
        sock = self._sock
        if sock is None:
            return
        data = (json.dumps(payload) + "\n").encode("utf-8")
        try:
            with self._send_lock:
                sock.sendall(data)
        except OSError:
            pass

    def _on_state(self, msg: dict, sock: object) -> None:
        if sock is not self._sock:
            return
        self._last_state = msg
        self.state_changed.emit(msg)

    def _on_tree(self, tree: object, sock: object) -> None:
        if sock is not self._sock:
            return
        self._last_tree = tree
        self.tree_changed.emit(tree)

    def _on_kill(self, stale: object) -> None:
        if stale is not None and stale is not self._sock:
            return
        self._kill()

    def _kill(self) -> None:
        sock = self._sock
        self._sock = None
        if sock is not None:
            with contextlib.suppress(OSError):
                sock.shutdown(socket.SHUT_RDWR)
            with contextlib.suppress(OSError):
                sock.close()
        proc = self._proc
        self._proc = None
        if proc is not None:
            with contextlib.suppress(OSError):
                proc.terminate()
            threading.Thread(target=proc.wait, daemon=True).start()
        self._last_state = self._default_state()
        self._last_tree = None
        self.state_changed.emit(self._last_state)
        self.tree_changed.emit(None)
