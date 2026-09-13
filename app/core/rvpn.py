import contextlib
import getpass
import hashlib
import mmap
import os
import re
import shutil
import stat
import subprocess
import tarfile
import threading
import time
from collections.abc import Callable
from pathlib import Path

from core import log
from core.constants import (
    BIN_DIR,
    RVPN_BIN_DIR,
    RVPN_FONTS_DIR,
    RVPN_MAC_FILE,
    RVPN_PREFIX,
    RVPN_STATE_DIR,
    WINE_API_URL,
    WINE_ASSET_SUFFIX,
    WINE_BIN,
    WINE_DIR,
)
from core.github import CancelledError, RateLimitError, fetch_to, github_asset
from core.reporter import Reporter

_TAP = "radminvpn0"
_RVPN_APP = RVPN_PREFIX / "drive_c" / "Program Files (x86)" / "Radmin VPN"
_SERVICE_LOG = RVPN_PREFIX / "drive_c" / "ProgramData" / "Famatech" / "Radmin VPN" / "service.log"
_SYS32 = RVPN_PREFIX / "drive_c" / "windows" / "system32"
_SYSWOW = RVPN_PREFIX / "drive_c" / "windows" / "syswow64"
_DRIVERS = _SYS32 / "drivers"
_FONTS_DIR = RVPN_PREFIX / "drive_c" / "windows" / "Fonts"
_REG_NAME = "rvpn.reg"
_REG_FILE = RVPN_PREFIX / "drive_c" / _REG_NAME

_FONT_SUBSTITUTES = ("Segoe UI", "Segoe UI Variable", "MS Shell Dlg")
_BUNDLED_FONTS = ("OpenSans-Regular.ttf", "OpenSans-Bold.ttf")
_FONTS_KEY = r"HKLM\Software\Microsoft\Windows NT\CurrentVersion\Fonts"
_FONT_SUB_KEY = r"HKLM\Software\Microsoft\Windows NT\CurrentVersion\FontSubstitutes"
_WINE_FONTS_KEY = r"HKCU\Software\Wine\Fonts\Replacements"
_RVPN_WINDOW_KEY = r"HKCU\Software\Famatech\Radmin VPN\ui\MainWindow"

_CMD_FILE = "/tmp/rvpn_netsh_cmd"
_CMD_PROC = _CMD_FILE + ".proc"
_MAC_RAW = "/tmp/rvpn_mac"
_FIFO_B2D = "/tmp/rvpn_b2d"
_FIFO_D2B_LOW = "/tmp/rvpn_d2b_low"
_FIFOS = (_FIFO_B2D, "/tmp/rvpn_d2b_high", _FIFO_D2B_LOW)
_RUNTIME_FILES = (_CMD_FILE, _CMD_PROC, _MAC_RAW, *_FIFOS)

_SERVICE_TIMEOUT_S = 30

_VPN_IP_SCAN = re.compile(r"26\.\d+\.\d+\.\d+")
_VPN_IP = re.compile(rf"^{_VPN_IP_SCAN.pattern}$")
_VPN_NET = "26.0.0.0/8"
_SERVICE_VER = re.compile(r"Service version:\s*([\d.]+)")
_MAC_RE = re.compile(r"^([0-9a-fA-F]{2}:){5}[0-9a-fA-F]{2}$")
_USER_RE = re.compile(r"^[A-Za-z0-9_.-]+$")
_NETSH_ADDR = re.compile(r"ip addr add (\d+\.\d+\.\d+\.\d+)/(\d+)")

_NET_CLASS = r"{4d36e972-e325-11ce-bfc1-08002be10318}"
_CLASS_KEY = rf"HKLM\SYSTEM\CurrentControlSet\Control\Class\{_NET_CLASS}\0099"
_NETWORK_KEY = rf"HKLM\SYSTEM\CurrentControlSet\Control\Network\{_NET_CLASS}"
_AEDEBUG_KEY = r"HKLM\Software\Microsoft\Windows NT\CurrentVersion\AeDebug"
_AEDEBUG_WOW_KEY = r"HKLM\Software\Wow6432Node\Microsoft\Windows NT\CurrentVersion\AeDebug"
_SERVICE_KEY = r"HKLM\SYSTEM\CurrentControlSet\Services\rvpnnetmp"
_NDIS_KEY = r"HKLM\SYSTEM\CurrentControlSet\Services\RvNetMP60"
_MEMORY_KEY = r"HKLM\System\CurrentControlSet\Control\Session Manager\Memory Management"


def _single_root(tmp_dir: Path) -> Path:
    entries = list(tmp_dir.iterdir())
    if len(entries) == 1 and entries[0].is_dir():
        return entries[0]
    return tmp_dir


def ensure_wine(reporter: Reporter, cancelled: Callable[[], bool] | None = None) -> Path:
    if WINE_BIN.exists():
        return WINE_DIR

    reporter.update("Fetching Wine")
    BIN_DIR.mkdir(parents=True, exist_ok=True)
    archive = BIN_DIR / "_wine.tar.xz"
    tmp_dir = BIN_DIR / ".wine.tmp"
    try:
        _, asset_url = github_asset(WINE_API_URL, WINE_ASSET_SUFFIX)
        shutil.rmtree(tmp_dir, ignore_errors=True)
        tmp_dir.mkdir(parents=True)
        fetch_to(asset_url, archive, cancelled=cancelled)
        with tarfile.open(archive) as t:
            t.extractall(tmp_dir, filter="data")
        staged = _single_root(tmp_dir)
        if not (staged / "bin" / "wine").exists():
            raise OSError("archive is incomplete")
        if WINE_DIR.exists():
            shutil.rmtree(WINE_DIR)
        staged.replace(WINE_DIR)
        return WINE_DIR
    except CancelledError, RateLimitError:
        raise
    except Exception as e:
        raise OSError(log.fail("Wine download failed", e)) from e
    finally:
        archive.unlink(missing_ok=True)
        shutil.rmtree(tmp_dir, ignore_errors=True)


def _wine_env() -> dict[str, str]:
    env = dict(os.environ)
    env["WINEPREFIX"] = str(RVPN_PREFIX)
    env["WINEDEBUG"] = "-all"
    env["WINEDLLOVERRIDES"] = "winemenubuilder.exe=d"
    env["MALLOC_ARENA_MAX"] = "2"
    if WINE_BIN.exists():
        env["PATH"] = str(WINE_DIR / "bin") + os.pathsep + env.get("PATH", "")
        env["WINELOADER"] = str(WINE_BIN)
        env["WINESERVER"] = str(WINE_DIR / "bin" / "wineserver")
    return env


def _rm(path: str | Path) -> None:
    with contextlib.suppress(OSError):
        Path(path).unlink()


def _copy(src: Path, dst: Path) -> None:
    dst.unlink(missing_ok=True)
    shutil.copy2(src, dst)


def _terminate(proc: subprocess.Popen | None) -> None:
    if proc is not None and proc.poll() is None:
        with contextlib.suppress(OSError):
            proc.terminate()


def _stop_proc(proc: subprocess.Popen | None) -> None:
    if proc is None or proc.poll() is not None:
        return
    try:
        proc.terminate()
        proc.wait(timeout=5)
    except subprocess.TimeoutExpired:
        proc.kill()
        proc.wait()
    except OSError:
        pass


def _wine_error(proc: subprocess.CompletedProcess) -> str:
    return proc.stderr.strip() or f"exit={proc.returncode}"


def _wineserver_stop(env: dict[str, str]) -> None:
    with contextlib.suppress(OSError):
        subprocess.run(["wineserver", "-k"], env=env, capture_output=True, check=False)
        subprocess.run(["wineserver", "-w"], env=env, capture_output=True, check=False)


def _kill_helpers(env: dict[str, str]) -> None:
    subprocess.run(["pkill", "-x", "tap_bridge"], check=False)
    _wineserver_stop(env)


def _is_fifo(path: str) -> bool:
    try:
        return stat.S_ISFIFO(Path(path).stat().st_mode)
    except OSError:
        return False


def _font_family(path: Path) -> str | None:
    try:
        data = path.read_bytes()
        for i in range(int.from_bytes(data[4:6], "big")):
            record = 12 + i * 16
            if data[record : record + 4] == b"name":
                table = int.from_bytes(data[record + 8 : record + 12], "big")
                break
        else:
            return None
        strings = table + int.from_bytes(data[table + 4 : table + 6], "big")
        family = None
        for i in range(int.from_bytes(data[table + 2 : table + 4], "big")):
            record = table + 6 + i * 12
            platform = int.from_bytes(data[record : record + 2], "big")
            language = int.from_bytes(data[record + 4 : record + 6], "big")
            name_id = int.from_bytes(data[record + 6 : record + 8], "big")
            if platform != 3 or name_id not in (1, 16):
                continue
            length = int.from_bytes(data[record + 8 : record + 10], "big")
            offset = strings + int.from_bytes(data[record + 10 : record + 12], "big")
            value = data[offset : offset + length].decode("utf-16-be", errors="ignore")
            if name_id == 16 and language == 0x409:
                return value
            if family is None or language == 0x409:
                family = value
        return family
    except OSError, IndexError:
        return None


def _font_is_bold(path: Path) -> bool:
    try:
        data = path.read_bytes()
        for i in range(int.from_bytes(data[4:6], "big")):
            record = 12 + i * 16
            if data[record : record + 4] == b"head":
                table = int.from_bytes(data[record + 8 : record + 12], "big")
                return bool(int.from_bytes(data[table + 44 : table + 46], "big") & 1)
    except OSError, IndexError:
        pass
    return False


def _font_key(path: Path) -> tuple[str, bool] | None:
    family = _font_family(path)
    return None if family is None else (family, _font_is_bold(path))


def _sync_font_clones(fonts: list[Path]) -> None:
    wine_fonts = WINE_DIR / "share" / "wine" / "fonts"
    disabled = WINE_DIR / "share" / "wine" / "fonts-disabled"
    if not wine_fonts.exists():
        return
    keys = {_font_key(font) for font in fonts} - {None}
    if disabled.exists():
        for clone in sorted(disabled.glob("*.ttf")):
            if _font_key(clone) not in keys:
                shutil.move(clone, wine_fonts / clone.name)
    for clone in sorted(wine_fonts.glob("*.ttf")):
        if _font_key(clone) in keys:
            disabled.mkdir(exist_ok=True)
            shutil.move(clone, disabled / clone.name)


def _font_setup() -> tuple[str, list[Path]] | None:
    if not RVPN_FONTS_DIR.exists():
        return None
    fonts = sorted(RVPN_FONTS_DIR.glob("*.ttf"))
    if not fonts:
        return None
    counts: dict[str, int] = {}
    for font in fonts:
        family = _font_family(font)
        if family:
            counts[family] = counts.get(family, 0) + 1
    if not counts:
        return None
    return max(counts, key=lambda name: counts[name]), fonts


def _privatize_desktop() -> None:
    users = RVPN_PREFIX / "drive_c" / "users"
    if not users.exists():
        return
    for desktop in users.glob("*/Desktop"):
        if desktop.is_symlink():
            desktop.unlink()
            desktop.mkdir(exist_ok=True)


def _read_utf16(path: Path) -> str:
    try:
        return path.read_bytes().decode("utf-16-le", errors="ignore")
    except OSError:
        return ""


def _extract_ip(text: str) -> str | None:
    lines = text.strip().split("\n")
    if not any("adapter ready" in line for line in lines):
        return None
    for line in reversed(lines):
        if "Registered as" in line or ("IP:" in line and "0.0.0.0" not in line):
            match = _VPN_IP_SCAN.search(line)
            if match:
                return match.group()
    return None


def _reg_hive(key: str) -> str:
    if key.startswith("HKLM\\"):
        return "HKEY_LOCAL_MACHINE\\" + key[5:]
    if key.startswith("HKCU\\"):
        return "HKEY_CURRENT_USER\\" + key[5:]
    return key


def _reg_value(reg_type: str, data: str) -> str:
    if reg_type == "REG_DWORD":
        return f"dword:{int(data, 0):08x}"
    if reg_type == "REG_EXPAND_SZ":
        raw = data.encode("utf-16-le") + b"\x00\x00"
        return "hex(2):" + ",".join(f"{b:02x}" for b in raw)
    return '"' + data.replace("\\", "\\\\").replace('"', '\\"') + '"'


_RegEntry = tuple[str, str | None, str | None, str | None]


def _reg_file(entries: list[_RegEntry]) -> bytes:
    grouped: dict[str, list[str]] = {}
    for key, name, reg_type, data in entries:
        grouped.setdefault(key, [])
        if name is not None:
            grouped[key].append(f'"{name}"={_reg_value(reg_type, data)}')
    lines = ["Windows Registry Editor Version 5.00", ""]
    for key in grouped:
        lines.append(f"[{_reg_hive(key)}]")
        lines.extend(grouped[key])
        lines.append("")
    return b"\xff\xfe" + "\r\n".join(lines).encode("utf-16-le")


_INSTALLER_MARKERS = (b"Radmin VPN", "Radmin VPN".encode("utf-16-le"))
_INSTALLER_MAX_BYTES = 100 * 2**20


def is_radmin_installer(path: str | Path) -> bool:
    p = Path(path)
    try:
        size = p.stat().st_size
        if size == 0 or size > _INSTALLER_MAX_BYTES:
            return False
        with p.open("rb") as f, mmap.mmap(f.fileno(), 0, access=mmap.ACCESS_READ) as m:
            return any(m.find(marker) != -1 for marker in _INSTALLER_MARKERS)
    except OSError:
        return False


def is_installed() -> bool:
    return (_RVPN_APP / "RvControlSvc.exe").exists()


def service_version() -> str:
    match = _SERVICE_VER.search(_read_utf16(_SERVICE_LOG))
    return match.group(1) if match else ""


def uninstall() -> None:
    _kill_helpers(_wine_env())
    for path in _RUNTIME_FILES:
        _rm(path)
    shutil.rmtree(RVPN_PREFIX, ignore_errors=True)
    shutil.rmtree(WINE_DIR, ignore_errors=True)
    RVPN_MAC_FILE.unlink(missing_ok=True)


class _Root:
    def __init__(self) -> None:
        self._proc: subprocess.Popen | None = None
        self._lock = threading.Lock()

    def open(self) -> str | None:
        self.close()
        loop = 'while IFS= read -r c; do [ "$c" = __END__ ] && break; eval "$c"; done'
        try:
            self._proc = subprocess.Popen(
                ["pkexec", "sh", "-c", loop],
                stdin=subprocess.PIPE,
                stdout=subprocess.DEVNULL,
                stderr=subprocess.DEVNULL,
            )
        except FileNotFoundError:
            return "pkexec not found"
        return None

    def alive(self) -> bool:
        return self._proc is not None and self._proc.poll() is None

    def run(self, script: str) -> str | None:
        with self._lock:
            proc = self._proc
            if proc is None or proc.stdin is None or proc.poll() is not None:
                return "authorization was declined"
            try:
                proc.stdin.write((script.replace("\n", "; ") + "\n").encode())
                proc.stdin.flush()
            except (OSError, ValueError) as error:
                return str(error)
        return None

    def close(self) -> None:
        with self._lock:
            proc = self._proc
            self._proc = None
        if proc is None:
            return
        if proc.stdin is not None:
            try:
                proc.stdin.write(b"__END__\n")
                proc.stdin.flush()
                proc.stdin.close()
            except OSError, ValueError:
                pass
        try:
            proc.wait(timeout=5)
        except subprocess.TimeoutExpired:
            _stop_proc(proc)


class Session:
    def __init__(self) -> None:
        self._cancel = threading.Event()
        self._relay_stop = threading.Event()
        self._bridge_proc: subprocess.Popen | None = None
        self._service_proc: subprocess.Popen | None = None
        self._gui_proc: subprocess.Popen | None = None
        self._mac = ""
        self._vpn_ip = ""
        self._env = _wine_env()
        self._env_headless = dict(self._env, WAYLAND_DISPLAY="none")
        self._env_headless.pop("DISPLAY", None)
        self._root = _Root()
        self._font_setup = _font_setup()

    def run(
        self, installer: Path | None, reporter: Reporter, *, on_running: Callable[[], None]
    ) -> None:
        try:
            if not self._preflight(reporter):
                return
            _kill_helpers(self._env)
            if not self._ensure_installed(installer, reporter):
                return
            error = self._root.open()
            if error:
                reporter.fail(log.fail("Network setup failed", error))
                return
            self._stage_artifacts(reporter)
            self._load_mac()
            reporter.update("Creating network device")
            error = self._net_bringup()
            if error:
                reporter.fail(log.fail("Network setup failed", error))
                return
            if not self._start_bridge(reporter):
                return
            self._configure_registry(reporter)
            self._start_relay()
            if not self._start_service(reporter):
                return
            reporter.update("Configuring routes")
            error = self._net_ready()
            if error:
                reporter.fail(log.fail("Routing failed", error))
                return
            self._launch_gui()
            on_running()
            self._wait_exit()
        finally:
            self._teardown()

    def stop(self) -> None:
        self._cancel.set()
        _terminate(self._gui_proc)
        _terminate(self._service_proc)

    def _preflight(self, reporter: Reporter) -> bool:
        if shutil.which("pkexec") is None:
            reporter.fail(
                log.fail("pkexec not found, a polkit agent is required", "missing dependency")
            )
            return False
        if shutil.which("ip") is None:
            reporter.fail(log.fail("iproute2 (ip) is not available", "missing dependency"))
            return False
        return True

    def _ensure_installed(self, installer: Path | None, reporter: Reporter) -> bool:
        if is_installed():
            return True
        if installer is None or not installer.is_file():
            reporter.fail("Radmin VPN installer not found")
            return False
        reporter.update("Installing")
        try:
            RVPN_PREFIX.mkdir(parents=True, exist_ok=True)
            boot = self._wine(["wineboot", "--init"])
            _privatize_desktop()
        except OSError as e:
            raise OSError(log.fail("Radmin VPN install failed", e)) from e
        if boot.returncode != 0:
            log.fail("wineboot failed", _wine_error(boot))
        self._reg_import(
            [
                (_AEDEBUG_KEY, "Debugger", "REG_SZ", "false"),
                (_AEDEBUG_WOW_KEY, "Debugger", "REG_SZ", "false"),
                (r"HKCU\Software\Wine\DllOverrides", "winemenubuilder.exe", "REG_SZ", ""),
            ]
        )
        _wineserver_stop(self._env)
        try:
            proc = subprocess.Popen(
                ["wine", str(installer), "/VERYSILENT", "/NORESTART"],
                cwd=str(RVPN_STATE_DIR),
                env=self._env,
                stdout=subprocess.DEVNULL,
                stderr=subprocess.DEVNULL,
            )
        except OSError as e:
            raise OSError(log.fail("Radmin VPN install failed", e)) from e
        while proc.poll() is None:
            if self._cancel.is_set():
                _stop_proc(proc)
                _wineserver_stop(self._env)
                return False
            time.sleep(0.2)
        for _ in range(30):
            if self._cancel.is_set():
                return False
            time.sleep(0.5)
            if is_installed():
                break
        if not is_installed():
            reporter.fail(
                log.fail("Radmin VPN installation failed", f"installer exit={proc.returncode}")
            )
            return False
        _wineserver_stop(self._env)
        self._scrub_ndis()
        _wineserver_stop(self._env)
        return True

    def _stage_artifacts(self, reporter: Reporter) -> None:
        reporter.update("Installing components")
        try:
            _privatize_desktop()
            _DRIVERS.mkdir(parents=True, exist_ok=True)
            _RVPN_APP.mkdir(parents=True, exist_ok=True)
            _SYSWOW.mkdir(parents=True, exist_ok=True)
            _copy(RVPN_BIN_DIR / "rvpnnetmp.sys", _DRIVERS / "rvpnnetmp.sys")
            _copy(RVPN_BIN_DIR / "adapter_hook.dll", _RVPN_APP / "adapter_hook.dll")
            _copy(RVPN_BIN_DIR / "rvpn_launcher.exe", _RVPN_APP / "rvpn_launcher.exe")
            _copy(RVPN_BIN_DIR / "netsh.exe", _SYSWOW / "netsh.exe")
            _copy(RVPN_BIN_DIR / "netsh64.exe", _SYS32 / "netsh.exe")
            _copy(RVPN_BIN_DIR / "drvinst.exe", _RVPN_APP / "drvinst.exe")
            _FONTS_DIR.mkdir(parents=True, exist_ok=True)
            for name in _BUNDLED_FONTS:
                src = RVPN_BIN_DIR / name
                if src.exists():
                    _copy(src, _FONTS_DIR / name)
            setup = self._font_setup
            _sync_font_clones(setup[1] if setup else [])
            if setup is not None:
                for font in setup[1]:
                    _copy(font, _FONTS_DIR / font.name)
            self._scrub_ndis()
        except OSError as e:
            raise OSError(log.fail("Component install failed", e)) from e

    def _load_mac(self) -> None:
        mac = ""
        with contextlib.suppress(OSError):
            mac = RVPN_MAC_FILE.read_text().strip()
        if not _MAC_RE.match(mac):
            octets = bytes([0x02, *os.urandom(5)])
            mac = ":".join(f"{b:02x}" for b in octets)
            try:
                RVPN_STATE_DIR.mkdir(parents=True, exist_ok=True)
                RVPN_MAC_FILE.write_text(mac)
            except OSError as e:
                raise OSError(log.fail("Could not write the adapter MAC file", e)) from e
        self._mac = mac
        raw = bytes(int(p, 16) for p in mac.split(":"))
        _rm(_MAC_RAW)
        try:
            fd = os.open(_MAC_RAW, os.O_WRONLY | os.O_CREAT | os.O_EXCL | os.O_NOFOLLOW, 0o600)
        except OSError as e:
            raise OSError(log.fail("Could not write the adapter MAC file", e)) from e
        with os.fdopen(fd, "wb") as f:
            f.write(raw)

    def _net_bringup(self) -> str | None:
        if not _MAC_RE.match(self._mac):
            return "invalid adapter MAC"
        user = getpass.getuser()
        if not _USER_RE.match(user):
            return "invalid user name"
        script = "; ".join(
            [
                "modprobe tun",
                f"ip link delete {_TAP} 2>/dev/null || true",
                f"ip tuntap add dev {_TAP} mode tap user {user}",
                f"ip link set {_TAP} address {self._mac}",
                f"ip link set {_TAP} up",
                f"ip link set {_TAP} multicast on",
                f"ip link set {_TAP} allmulticast on",
                f"sysctl -w net.ipv4.conf.{_TAP}.rp_filter=0",
                f"sysctl -w net.ipv4.conf.{_TAP}.accept_local=1",
                f"ip maddr add 224.0.2.60 dev {_TAP} 2>/dev/null || true",
                f"ip route add 224.0.2.60/32 dev {_TAP} 2>/dev/null || true",
                f"nmcli device set {_TAP} managed no 2>/dev/null || true",
            ]
        )
        error = self._root.run(script)
        if error:
            return error
        for _ in range(600):
            if self._cancel.is_set():
                return "cancelled"
            if not self._root.alive():
                return "authorization was declined"
            if Path("/sys/class/net", _TAP).exists():
                return None
            time.sleep(0.1)
        return "could not create the network device"

    def _start_bridge(self, reporter: Reporter) -> bool:
        reporter.update("Preparing network bridge")
        for fifo in _FIFOS:
            _rm(fifo)
        try:
            self._bridge_proc = subprocess.Popen(
                [str(RVPN_BIN_DIR / "tap_bridge")],
                stdout=subprocess.DEVNULL,
                stderr=subprocess.DEVNULL,
            )
        except OSError as e:
            reporter.fail(log.fail("Network bridge failed to start", e))
            return False
        for _ in range(10):
            if _is_fifo(_FIFO_B2D) and _is_fifo(_FIFO_D2B_LOW):
                return True
            time.sleep(0.1)
        reporter.fail(log.fail("Network bridge failed to start", "fifo timeout"))
        return False

    def _configure_registry(self, reporter: Reporter) -> None:
        reporter.update("Configuring adapter")
        guid = self._detect_guid()
        connection = _NETWORK_KEY + "\\" + guid + r"\Connection"
        app = r"HKLM\Software\Wow6432Node\Famatech\RadminVPN\1.0"
        firewall = app + r"\Firewall"
        registration = r"HKLM\SOFTWARE\Famatech\RadminVPN\1.0\Registration"
        image = r"C:\windows\system32\drivers\rvpnnetmp.sys"
        desktop = r"HKCU\Control Panel\Desktop"
        entries = [
            (_CLASS_KEY, "NetCfgInstanceId", "REG_SZ", guid),
            (_CLASS_KEY, "MatchingDeviceId", "REG_SZ", guid + r"\RvNetMP60"),
            (connection, "Name", "REG_SZ", "Radmin VPN"),
            (connection, "PnpInstanceID", "REG_SZ", r"ROOT\NET\0099"),
            (firewall, "AdapterId", "REG_SZ", guid),
            (app, "PowerOn", "REG_DWORD", "1"),
            (app + r"\update", "Mode", "REG_DWORD", "3"),
            (registration, None, None, None),
            (_SERVICE_KEY, "DisplayName", "REG_SZ", "Radmin VPN TAP Bridge"),
            (_SERVICE_KEY, "ImagePath", "REG_EXPAND_SZ", image),
            (_SERVICE_KEY, "Start", "REG_DWORD", "2"),
            (r"HKLM\SYSTEM\CurrentControlSet\Services\RvControlSvc", "Start", "REG_DWORD", "4"),
            (_SERVICE_KEY, "Type", "REG_DWORD", "1"),
            (_SERVICE_KEY, "Group", "REG_SZ", "NDIS"),
            (_SERVICE_KEY, "ErrorControl", "REG_DWORD", "0"),
            (_MEMORY_KEY, "SystemPages", "REG_DWORD", "0xFFFFFFFF"),
            (_MEMORY_KEY, "ClearPageFileAtShutdown", "REG_DWORD", "0"),
            (_MEMORY_KEY, "LargeSystemCache", "REG_DWORD", "1"),
            (r"HKCU\Software\Wine\X11 Driver", "Decorated", "REG_SZ", "N"),
            (desktop, "FontSmoothing", "REG_SZ", "2"),
            (desktop, "FontSmoothingType", "REG_DWORD", "1"),
            (_RVPN_WINDOW_KEY, "IsHidden", "REG_SZ", "false"),
        ]
        entries += [
            (_FONTS_KEY, f"{Path(name).stem} (TrueType)", "REG_SZ", name)
            for name in _BUNDLED_FONTS
            if (RVPN_BIN_DIR / name).exists()
        ]
        entries += [
            (_FONT_SUB_KEY, "MS Shell Dlg 2", "REG_SZ", "Tahoma"),
            (_WINE_FONTS_KEY, "MS Shell Dlg 2", "REG_SZ", "Tahoma"),
        ]
        setup = self._font_setup
        if setup is not None:
            family, fonts = setup
            entries += [
                (_FONTS_KEY, f"{font.stem} (TrueType)", "REG_SZ", font.name) for font in fonts
            ]
            entries += [(_FONT_SUB_KEY, face, "REG_SZ", family) for face in _FONT_SUBSTITUTES]
            entries += [(_WINE_FONTS_KEY, face, "REG_SZ", family) for face in _FONT_SUBSTITUTES]
        self._reg_import(entries)
        _wineserver_stop(self._env)

    def _detect_guid(self) -> str:
        digest = hashlib.md5((self._mac.replace(":", "") + "\n").encode()).hexdigest()
        return (
            "{" + f"{digest[0:8]}-{digest[8:12]}-{digest[12:16]}"
            f"-{digest[16:20]}-{digest[20:32]}" + "}"
        )

    def _start_relay(self) -> None:
        _rm(_CMD_FILE)
        _rm(_CMD_PROC)
        self._relay_stop.clear()
        threading.Thread(target=self._relay_loop, daemon=True).start()

    def _relay_loop(self) -> None:
        processing = _CMD_PROC
        while not self._relay_stop.is_set():
            try:
                if Path(_CMD_FILE).is_file():
                    Path(_CMD_FILE).rename(processing)
                    for line in Path(processing).read_text().splitlines():
                        self._apply_netsh(line.strip())
                    _rm(processing)
            except OSError:
                pass
            self._relay_stop.wait(0.1)

    def _apply_netsh(self, line: str) -> None:
        match = _NETSH_ADDR.search(line)
        if not match:
            return
        ip, cidr = match.group(1), match.group(2)
        if _VPN_IP.match(ip):
            error = self._root.run(
                f"ip addr add {ip}/{cidr} dev {_TAP} 2>/dev/null || true; "
                f"ip link set {_TAP} up 2>/dev/null || true"
            )
            if error:
                log.fail("Applying the VPN address failed", error)

    def _start_service(self, reporter: Reporter) -> bool:
        reporter.update("Running daemon")
        _rm(_SERVICE_LOG)
        env = dict(self._env)
        env["WINEDLLOVERRIDES"] = "mscoree=;mshtml=;netsh.exe=n"
        env["WINE_LARGE_ADDRESS_AWARE"] = "1"
        shim = RVPN_BIN_DIR / "dns_shim.so"
        if shim.exists():
            preload = env.get("LD_PRELOAD", "")
            env["LD_PRELOAD"] = f"{shim}:{preload}" if preload else str(shim)
        self._service_proc = subprocess.Popen(
            ["wine", "rvpn_launcher.exe", "/run"],
            cwd=str(_RVPN_APP),
            env=env,
            stdout=subprocess.DEVNULL,
            stderr=subprocess.DEVNULL,
        )
        for _ in range(_SERVICE_TIMEOUT_S * 5):
            if self._cancel.is_set():
                return False
            time.sleep(0.2)
            text = _read_utf16(_SERVICE_LOG)
            if text:
                match = _SERVICE_VER.search(text)
                if match is not None and match.group(1).startswith("1.4"):
                    reporter.fail(
                        log.fail(
                            "This Radmin VPN installer is outdated", f"daemon {match.group(1)}"
                        )
                    )
                    return False
                ip = _extract_ip(text)
                if ip:
                    self._vpn_ip = ip
                    return True
            if self._service_proc.poll() is not None:
                reporter.fail(
                    log.fail("Radmin VPN daemon exited", f"exit={self._service_proc.returncode}")
                )
                return False
        reporter.fail(
            log.fail("Radmin VPN daemon never became ready", f"{_SERVICE_TIMEOUT_S}s timeout")
        )
        return False

    def _net_ready(self) -> str | None:
        if not _VPN_IP.match(self._vpn_ip):
            return "invalid VPN IP"
        script = "; ".join(
            [
                f"ip addr add {self._vpn_ip}/8 dev {_TAP} 2>/dev/null || true",
                f"ip link set {_TAP} up",
                f"ip route replace {_VPN_NET} dev {_TAP}",
                f"ip route append 255.255.255.255/32 dev {_TAP} metric 0 2>/dev/null || true",
                f"ip route append 224.0.0.0/4 dev {_TAP} metric 0 2>/dev/null || true",
            ]
        )
        return self._root.run(script)

    def _launch_gui(self) -> None:
        env = dict(self._env)
        env["LIBGL_ALWAYS_SOFTWARE"] = "1"
        env["GALLIUM_DRIVER"] = "llvmpipe"
        try:
            self._gui_proc = subprocess.Popen(
                ["wine", "RvRvpnGui.exe"],
                cwd=str(_RVPN_APP),
                env=env,
                stdout=subprocess.DEVNULL,
                stderr=subprocess.DEVNULL,
            )
        except OSError as e:
            raise OSError(log.fail("Radmin VPN GUI failed to start", e)) from e

    def _wait_exit(self) -> None:
        proc = self._gui_proc
        while proc.poll() is None:
            if self._cancel.is_set():
                _terminate(proc)
                break
            time.sleep(0.3)

    def _teardown(self) -> None:
        self._relay_stop.set()
        _wineserver_stop(self._env)
        _stop_proc(self._gui_proc)
        _stop_proc(self._service_proc)
        _stop_proc(self._bridge_proc)
        self._gui_proc = self._service_proc = self._bridge_proc = None
        self._root.run(f"ip link delete {_TAP} 2>/dev/null || true")
        self._root.close()
        for path in _RUNTIME_FILES:
            _rm(path)

    def _wine(self, args: list[str]) -> subprocess.CompletedProcess:
        return subprocess.run(
            ["wine", *args],
            cwd=str(RVPN_STATE_DIR),
            env=self._env_headless,
            capture_output=True,
            text=True,
            errors="replace",
            check=False,
        )

    def _reg_import(self, entries: list[_RegEntry]) -> None:
        try:
            _REG_FILE.write_bytes(_reg_file(entries))
            proc = self._wine(["reg", "import", "C:\\" + _REG_NAME])
        except OSError as e:
            raise OSError(log.fail("Registry import failed", e)) from e
        if proc.returncode != 0:
            log.fail("Registry import failed", _wine_error(proc))
        _rm(_REG_FILE)

    def _scrub_ndis(self) -> None:
        if not (_DRIVERS / "RvNetMP60.sys").exists():
            return
        self._wine(["reg", "delete", _NDIS_KEY, "/f"])
        _rm(_DRIVERS / "RvNetMP60.sys")
