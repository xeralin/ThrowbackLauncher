from __future__ import annotations

import contextlib
import json
import os
import select
import socket
import struct
import uuid
from collections.abc import Iterator
from pathlib import Path

from core import log
from core.constants import IS_WINDOWS
from core.manifest import hm_display_name
from layout import OWNER

OP_HANDSHAKE = 0
OP_FRAME = 1
OP_CLOSE = 2
OP_PING = 3
OP_PONG = 4

IPC_TIMEOUT = 10

TB_CLIENT_ID = "1544119665394192525"
HM_CLIENT_ID = "1544119720662278215"
FAQ_BUTTON = {
    "label": "Download",
    "url": f"https://{OWNER}.github.io/ThrowbackFAQ/getting-started/",
}
STATUS_DISPLAY_DETAILS = 2

if IS_WINDOWS:
    import ctypes
    from ctypes import wintypes

    _GENERIC_READ = 0x80000000
    _GENERIC_WRITE = 0x40000000
    _OPEN_EXISTING = 3
    _FILE_FLAG_OVERLAPPED = 0x40000000
    _ERROR_IO_PENDING = 997
    _WAIT_TIMEOUT = 258
    _INVALID_HANDLE = wintypes.HANDLE(-1).value

    class _Overlapped(ctypes.Structure):
        _fields_ = (
            ("Internal", ctypes.c_void_p),
            ("InternalHigh", ctypes.c_void_p),
            ("Offset", wintypes.DWORD),
            ("OffsetHigh", wintypes.DWORD),
            ("hEvent", wintypes.HANDLE),
        )

    _kernel32 = ctypes.WinDLL("kernel32", use_last_error=True)
    _kernel32.CreateFileW.argtypes = (
        wintypes.LPCWSTR,
        wintypes.DWORD,
        wintypes.DWORD,
        wintypes.LPVOID,
        wintypes.DWORD,
        wintypes.DWORD,
        wintypes.HANDLE,
    )
    _kernel32.CreateFileW.restype = wintypes.HANDLE
    _kernel32.CreateEventW.argtypes = (
        wintypes.LPVOID,
        wintypes.BOOL,
        wintypes.BOOL,
        wintypes.LPCWSTR,
    )
    _kernel32.CreateEventW.restype = wintypes.HANDLE
    _kernel32.ReadFile.argtypes = (
        wintypes.HANDLE,
        wintypes.LPVOID,
        wintypes.DWORD,
        wintypes.LPDWORD,
        ctypes.POINTER(_Overlapped),
    )
    _kernel32.WriteFile.argtypes = (
        wintypes.HANDLE,
        wintypes.LPVOID,
        wintypes.DWORD,
        wintypes.LPDWORD,
        ctypes.POINTER(_Overlapped),
    )
    _kernel32.GetOverlappedResultEx.argtypes = (
        wintypes.HANDLE,
        ctypes.POINTER(_Overlapped),
        wintypes.LPDWORD,
        wintypes.DWORD,
        wintypes.BOOL,
    )
    _kernel32.GetOverlappedResult.argtypes = (
        wintypes.HANDLE,
        ctypes.POINTER(_Overlapped),
        wintypes.LPDWORD,
        wintypes.BOOL,
    )
    _kernel32.CancelIoEx.argtypes = (wintypes.HANDLE, ctypes.POINTER(_Overlapped))
    _kernel32.PeekNamedPipe.argtypes = (
        wintypes.HANDLE,
        wintypes.LPVOID,
        wintypes.DWORD,
        wintypes.LPDWORD,
        wintypes.LPDWORD,
        wintypes.LPDWORD,
    )
    _kernel32.CloseHandle.argtypes = (wintypes.HANDLE,)


def _unix_paths() -> Iterator[str]:
    bases = [
        v for env in ("XDG_RUNTIME_DIR", "TMPDIR", "TMP", "TEMP") if (v := os.environ.get(env))
    ] + ["/tmp"]
    seen: set[str] = set()
    for base in bases:
        root = Path(base)
        dirs = (
            root,
            *sorted(root.glob("app/*")),
            *sorted(root.glob(".flatpak/*/xdg-run")),
        )
        for d in dirs:
            for i in range(10):
                p = d / f"discord-ipc-{i}"
                if p.exists():
                    s = str(p)
                    if s not in seen:
                        seen.add(s)
                        yield s


class _PipeConn:
    def __init__(self, handle: int) -> None:
        self._handle = handle
        self._event = _kernel32.CreateEventW(None, True, False, None)

    @classmethod
    def open(cls, path: str) -> _PipeConn | None:
        handle = _kernel32.CreateFileW(
            path,
            _GENERIC_READ | _GENERIC_WRITE,
            0,
            None,
            _OPEN_EXISTING,
            _FILE_FLAG_OVERLAPPED,
            None,
        )
        if handle is None or handle == _INVALID_HANDLE:
            return None
        return cls(handle)

    def _await_result(self, ov: _Overlapped) -> int:
        count = wintypes.DWORD()
        if _kernel32.GetOverlappedResultEx(
            self._handle, ctypes.byref(ov), ctypes.byref(count), IPC_TIMEOUT * 1000, False
        ):
            return count.value
        err = ctypes.get_last_error()
        _kernel32.CancelIoEx(self._handle, ctypes.byref(ov))
        _kernel32.GetOverlappedResult(self._handle, ctypes.byref(ov), ctypes.byref(count), True)
        if err == _WAIT_TIMEOUT:
            raise TimeoutError("pipe timed out")
        raise ctypes.WinError(err)

    def sendall(self, data: bytes) -> None:
        ov = _Overlapped(hEvent=self._event)
        if (
            not _kernel32.WriteFile(self._handle, data, len(data), None, ctypes.byref(ov))
            and ctypes.get_last_error() != _ERROR_IO_PENDING
        ):
            raise ctypes.WinError(ctypes.get_last_error())
        self._await_result(ov)

    def recv(self, n: int) -> bytes:
        buf = ctypes.create_string_buffer(n)
        ov = _Overlapped(hEvent=self._event)
        if (
            not _kernel32.ReadFile(self._handle, buf, n, None, ctypes.byref(ov))
            and ctypes.get_last_error() != _ERROR_IO_PENDING
        ):
            raise ctypes.WinError(ctypes.get_last_error())
        count = self._await_result(ov)
        return buf.raw[:count]

    def pending(self) -> bool:
        available = wintypes.DWORD()
        if not _kernel32.PeekNamedPipe(self._handle, None, 0, None, ctypes.byref(available), None):
            raise ctypes.WinError(ctypes.get_last_error())
        return bool(available.value)

    def close(self) -> None:
        _kernel32.CloseHandle(self._handle)
        _kernel32.CloseHandle(self._event)


def _readable(conn: _PipeConn | socket.socket) -> bool:
    if isinstance(conn, _PipeConn):
        return conn.pending()
    return bool(select.select([conn], [], [], 0)[0])


def _open_candidates() -> Iterator[_PipeConn | socket.socket]:
    if IS_WINDOWS:
        for i in range(10):
            conn = _PipeConn.open(rf"\\.\pipe\discord-ipc-{i}")
            if conn is not None:
                yield conn
    else:
        for path in _unix_paths():
            s = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
            s.settimeout(IPC_TIMEOUT)
            try:
                s.connect(path)
            except OSError:
                s.close()
                continue
            yield s


class Presence:
    def __init__(self, client_id: str) -> None:
        self.client_id = client_id
        self.sock: _PipeConn | socket.socket | None = None

    @property
    def connected(self) -> bool:
        return self.sock is not None

    def connect(self) -> bool:
        for conn in _open_candidates():
            self.sock = conn
            with contextlib.suppress(OSError):
                self._send(OP_HANDSHAKE, {"v": 1, "client_id": self.client_id})
                op, _ = self._recv()
                if op == OP_FRAME:
                    return True
            self.close()
        return False

    def _send(self, op: int, payload: dict) -> None:
        sock = self.sock
        if sock is None:
            raise ConnectionError("not connected")
        data = json.dumps(payload).encode()
        sock.sendall(struct.pack("<II", op, len(data)) + data)

    def _recv(self) -> tuple[int, dict]:
        sock = self.sock
        if sock is None:
            raise ConnectionError("not connected")
        header = b""
        while len(header) < 8:
            chunk = sock.recv(8 - len(header))
            if not chunk:
                raise ConnectionError("socket closed")
            header += chunk
        op, length = struct.unpack("<II", header)
        body = b""
        while len(body) < length:
            chunk = sock.recv(length - len(body))
            if not chunk:
                raise ConnectionError("socket closed")
            body += chunk
        try:
            payload = json.loads(body.decode())
        except ValueError as e:
            raise ConnectionError("malformed frame") from e
        if not isinstance(payload, dict):
            raise ConnectionError("malformed frame")
        return op, payload

    def _await_response(self, nonce: str) -> bool:
        while True:
            op, payload = self._recv()
            if op == OP_PING:
                self._send(OP_PONG, payload)
                continue
            if op == OP_FRAME and payload.get("nonce") == nonce:
                if payload.get("evt") != "ERROR":
                    return True
                log.fail("Discord rejected the activity", payload.get("data"))
                return False

    def set(self, activity: dict) -> bool:
        try:
            nonce = str(uuid.uuid4())
            self._send(
                OP_FRAME,
                {
                    "cmd": "SET_ACTIVITY",
                    "args": {"pid": os.getpid(), "activity": activity},
                    "nonce": nonce,
                },
            )
            return self._await_response(nonce)
        except OSError:
            self.close()
            return False

    def drain(self) -> None:
        if not self.sock:
            return
        try:
            while _readable(self.sock):
                op, payload = self._recv()
                if op == OP_PING:
                    self._send(OP_PONG, payload)
        except OSError:
            self.close()

    def close(self) -> None:
        if self.sock:
            with contextlib.suppress(OSError):
                self._send(OP_CLOSE, {})
            with contextlib.suppress(OSError):
                self.sock.close()
            self.sock = None


def build_activity(download: dict, is_hm: bool, start: int) -> tuple[str, dict]:
    activity = {
        "status_display_type": STATUS_DISPLAY_DETAILS,
        "details": hm_display_name(download) if is_hm else download["label"],
        "timestamps": {"start": start},
        "buttons": [FAQ_BUTTON],
    }
    return (HM_CLIENT_ID if is_hm else TB_CLIENT_ID), activity
