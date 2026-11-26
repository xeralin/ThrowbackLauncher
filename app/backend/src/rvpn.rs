use std::fs;
use std::path::{Path, PathBuf};
use std::sync::LazyLock;

use regex::Regex;

use crate::env::Paths;

pub use host::{Session, ensure_wine, preflight, uninstall};

pub struct Reporter {
    pub update: Box<dyn Fn(&str) + Send + Sync>,
    pub fail: Box<dyn Fn(&str) + Send + Sync>,
}

const INSTALLER_LIMIT: u64 = 100 << 20;

static INSTALLER_MARK: LazyLock<regex::bytes::Regex> = LazyLock::new(|| {
    regex::bytes::Regex::new(r"Radmin VPN|R\x00a\x00d\x00m\x00i\x00n\x00 \x00V\x00P\x00N\x00")
        .expect("valid pattern")
});
static SERVICE_VER: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"Service version:\s*([\d.]+)").expect("valid pattern"));

fn prefix(paths: &Paths) -> PathBuf {
    paths.rvpn_state_dir().join("wineprefix")
}

fn app_dir(paths: &Paths) -> PathBuf {
    prefix(paths)
        .join("drive_c")
        .join("Program Files (x86)")
        .join("Radmin VPN")
}

fn service_log(paths: &Paths) -> PathBuf {
    prefix(paths)
        .join("drive_c")
        .join("ProgramData")
        .join("Famatech")
        .join("Radmin VPN")
        .join("service.log")
}

fn read_utf16(file: &Path) -> String {
    let bytes = fs::read(file).unwrap_or_default();
    let units = bytes.as_chunks().0.iter().copied().map(u16::from_le_bytes);
    char::decode_utf16(units).filter_map(Result::ok).collect()
}

pub fn is_radmin_installer(file: &Path) -> bool {
    fs::metadata(file).is_ok_and(|metadata| (1..=INSTALLER_LIMIT).contains(&metadata.len()))
        && fs::read(file).is_ok_and(|data| INSTALLER_MARK.is_match(&data))
}

pub fn is_installed(paths: &Paths) -> bool {
    app_dir(paths).join("RvControlSvc.exe").exists()
}

pub fn service_version(paths: &Paths) -> String {
    SERVICE_VER
        .captures(&read_utf16(&service_log(paths)))
        .map(|found| found[1].to_string())
        .unwrap_or_default()
}

#[cfg(target_os = "linux")]
mod host {
    use std::collections::HashMap;
    use std::ffi::{CString, OsStr, OsString};
    use std::fs::{self, File, OpenOptions};
    use std::io::{self, BufReader, Read, Write};
    use std::net::Ipv6Addr;
    use std::os::unix::ffi::OsStrExt;
    use std::os::unix::fs::{FileTypeExt, OpenOptionsExt};
    use std::path::{Path, PathBuf};
    use std::process::{Child, Command, ExitStatus, Output, Stdio};
    use std::sync::{Condvar, LazyLock, Mutex, MutexGuard, PoisonError};
    use std::thread::{self, Scope};
    use std::time::{Duration, Instant};

    use anyhow::{Result, anyhow, bail};
    use lzma_rust2::XzReader;
    use md5::{Digest, Md5};
    use regex::Regex;
    use tar::Archive;

    use super::Reporter;
    use super::{SERVICE_VER, app_dir, is_installed, prefix, read_utf16, service_log};
    use crate::env::{Paths, copy_with_times, remove_if_present};
    use crate::github::{CancelledError, download_error, fetch_to, github_asset};
    use crate::log;

    const TAP: &str = "radminvpn0";
    const REG_NAME: &str = "rvpn.reg";
    const BUNDLED_FONTS: [&str; 2] = ["OpenSans-Regular.ttf", "OpenSans-Bold.ttf"];

    const CMD_FILE: &str = "/tmp/rvpn_netsh_cmd";
    const CMD_PROC: &str = "/tmp/rvpn_netsh_cmd.proc";
    const MAC_RAW: &str = "/tmp/rvpn_mac";
    const FIFO_B2D: &str = "/tmp/rvpn_b2d";
    const FIFO_D2B_HIGH: &str = "/tmp/rvpn_d2b_high";
    const FIFO_D2B_LOW: &str = "/tmp/rvpn_d2b_low";
    const FIFOS: [&str; 3] = [FIFO_B2D, FIFO_D2B_HIGH, FIFO_D2B_LOW];
    const RUNTIME_FILES: [&str; 6] = [
        CMD_FILE,
        CMD_PROC,
        MAC_RAW,
        FIFO_B2D,
        FIFO_D2B_HIGH,
        FIFO_D2B_LOW,
    ];

    const WINE_TAG: &str = "11.21";
    const SERVICE_TIMEOUT_S: u32 = 30;
    const SERVICE_POLL: Duration = Duration::from_millis(200);
    const INSTALLER_POLL: Duration = Duration::from_millis(200);
    const INSTALLED_CHECKS: u32 = 30;
    const INSTALLED_POLL: Duration = Duration::from_millis(500);
    const DEVICE_CHECKS: u32 = 600;
    const DEVICE_POLL: Duration = Duration::from_millis(100);
    const FIFO_CHECKS: u32 = 10;
    const FIFO_POLL: Duration = Duration::from_millis(100);
    const RELAY_POLL: Duration = Duration::from_millis(100);
    const EXIT_POLL: Duration = Duration::from_millis(300);
    const STOP_TIMEOUT: Duration = Duration::from_secs(5);
    const WAIT_STEP_MIN: Duration = Duration::from_micros(500);
    const WAIT_STEP_MAX: Duration = Duration::from_millis(50);

    static VPN_IP_SCAN: LazyLock<Regex> =
        LazyLock::new(|| Regex::new(r"26\.\d+\.\d+\.\d+").expect("valid pattern"));
    static VPN_IP: LazyLock<Regex> =
        LazyLock::new(|| Regex::new(r"^26\.\d+\.\d+\.\d+$").expect("valid pattern"));
    static MAC_RE: LazyLock<Regex> = LazyLock::new(|| {
        Regex::new(r"^([0-9a-fA-F]{2}:){5}[0-9a-fA-F]{2}$").expect("valid pattern")
    });
    static NETSH_ADDR: LazyLock<Regex> = LazyLock::new(|| {
        Regex::new(r"ip addr add (\d+\.\d+\.\d+\.\d+)/(\d+)").expect("valid pattern")
    });
    static NETSH_ADDR6: LazyLock<Regex> = LazyLock::new(|| {
        Regex::new(r"ip -6 addr add ([0-9a-fA-F:]+)/64 dev radminvpn0").expect("valid pattern")
    });

    const NET_CLASS: &str = "{4d36e972-e325-11ce-bfc1-08002be10318}";
    const DRIVER_KEY: &str = r"HKLM\SYSTEM\CurrentControlSet\Services\rvpnnetmp";
    const MEMORY_KEY: &str =
        r"HKLM\System\CurrentControlSet\Control\Session Manager\Memory Management";

    const QT_NAME_SITE: usize = 0x5E90D;
    const QT_NAME_READ: [u8; 11] = [
        0x8b, 0x83, 0xd4, 0x00, 0x00, 0x00, 0x03, 0xc3, 0x6a, 0xff, 0x50,
    ];
    const QT_NAME_JUMP: [u8; 11] = [
        0xe9, 0x77, 0x29, 0xfb, 0xff, 0x90, 0x90, 0x90, 0x90, 0x90, 0x90,
    ];
    const QT_NAME_CAVE: usize = 0x11289;
    const QT_NAME_CHECK: [u8; 34] = [
        0x8b, 0x83, 0xd4, 0x00, 0x00, 0x00, 0x3d, 0x00, 0x00, 0x01, 0x00, 0x73, 0x0d, 0x03, 0xc3,
        0x68, 0xff, 0xff, 0xff, 0xff, 0x50, 0xe9, 0x75, 0xd6, 0x04, 0x00, 0x6a, 0x00, 0x53, 0xe9,
        0x6d, 0xd6, 0x04, 0x00,
    ];

    const NFT_INPUT_CHAINS: &str = concat!(
        r#"nft list chains | awk '$1 == "table" { f = $2; t = $3 } $1 == "chain" { c = $2 } "#,
        r#"/type filter hook input/ && f ~ /^(ip|ip6|inet)$/ { print f, t, c }'"#,
    );
    const ROOT_LOOP: &str =
        r#"while IFS= read -r c; do [ "$c" = __END__ ] && break; eval "$c"; done"#;

    type Env = HashMap<OsString, OsString>;

    fn firewall_close() -> String {
        format!(
            "firewall-cmd --zone=trusted --remove-interface={TAP}; \
             {NFT_INPUT_CHAINS} | while read -r f t c; do nft -a list chain $f $t $c | \
             sed -n 's/.*\"radminvpn\" # handle //p' | while read -r h; do \
             nft delete rule $f $t $c handle $h; done; done; \
             iptables -D INPUT -i {TAP} -j ACCEPT; ip6tables -D INPUT -i {TAP} -j ACCEPT"
        )
    }

    fn firewall_open() -> String {
        format!(
            "if firewall-cmd --state; then firewall-cmd --zone=trusted --change-interface={TAP}; \
             else {NFT_INPUT_CHAINS} | while read -r f t c; do \
             nft insert rule $f $t $c iifname {TAP} accept comment radminvpn; done; \
             command -v nft && iptables --version | grep -q nf_tables || \
             {{ iptables -I INPUT -i {TAP} -j ACCEPT; ip6tables -I INPUT -i {TAP} -j ACCEPT; }}; fi"
        )
    }

    fn bringup_script(uid: u32, mac: &str) -> String {
        [
            "modprobe tun".to_string(),
            format!("ip link delete {TAP}"),
            format!("ip tuntap add dev {TAP} mode tap user {uid}"),
            format!("ip link set {TAP} address {mac}"),
            format!("ip link set {TAP} up"),
            format!("ip link set {TAP} multicast on"),
            format!("ip link set {TAP} allmulticast on"),
            format!("sysctl -w net.ipv4.conf.{TAP}.rp_filter=0"),
            format!("sysctl -w net.ipv4.conf.{TAP}.accept_local=1"),
            format!("ip route add 224.0.2.60/32 dev {TAP}"),
            format!("nmcli device set {TAP} managed no"),
            firewall_close(),
            firewall_open(),
        ]
        .join("; ")
    }

    fn routes_script(vpn_ip: &str) -> String {
        [
            format!("ip addr add {vpn_ip}/8 dev {TAP}"),
            format!("ip link set {TAP} up"),
            format!("ip route replace 26.0.0.0/8 dev {TAP}"),
            format!("ip route append 255.255.255.255/32 dev {TAP} metric 0"),
            format!("ip route append 224.0.0.0/4 dev {TAP} metric 0"),
        ]
        .join("; ")
    }

    fn teardown_script() -> String {
        format!("ip link delete {TAP}; {}", firewall_close())
    }

    fn netsh_script(line: &str) -> Option<String> {
        if let Some(found) = NETSH_ADDR.captures(line) {
            let (address, prefix) = (&found[1], &found[2]);
            return VPN_IP.is_match(address).then(|| {
                format!("ip addr add {address}/{prefix} dev {TAP}; ip link set {TAP} up")
            });
        }
        let found = NETSH_ADDR6.captures(line)?;
        let address = found[1].parse::<Ipv6Addr>().ok()?;
        (address.segments()[0] == 0xfdfd).then(|| format!("ip -6 addr add {address}/64 dev {TAP}"))
    }

    fn netsh_scripts(text: &str) -> Vec<String> {
        text.lines().filter_map(netsh_script).collect()
    }

    fn extract_ip(text: &str) -> Option<&str> {
        if !text.contains("adapter ready") {
            return None;
        }
        text.split('\n')
            .rev()
            .filter(|line| {
                line.contains("Registered as") || line.contains("IP:") && !line.contains("0.0.0.0")
            })
            .find_map(|line| VPN_IP_SCAN.find(line))
            .map(|found| found.as_str())
    }

    #[derive(Debug, PartialEq)]
    enum Daemon<'a> {
        Outdated(&'a str),
        Ready(&'a str),
        Starting,
    }

    fn daemon_state(text: &str) -> Daemon<'_> {
        match SERVICE_VER.captures(text).and_then(|found| found.get(1)) {
            Some(version) if version.as_str().starts_with("1.4") => {
                Daemon::Outdated(version.as_str())
            }
            _ => extract_ip(text).map_or(Daemon::Starting, Daemon::Ready),
        }
    }

    enum RegData {
        Text(String),
        Number(u32),
        Expand(String),
    }

    struct RegEntry {
        key: String,
        value: Option<(String, RegData)>,
    }

    fn reg_text(key: &str, name: &str, data: &str) -> RegEntry {
        RegEntry {
            key: key.to_string(),
            value: Some((name.to_string(), RegData::Text(data.to_string()))),
        }
    }

    fn reg_number(key: &str, name: &str, data: u32) -> RegEntry {
        RegEntry {
            key: key.to_string(),
            value: Some((name.to_string(), RegData::Number(data))),
        }
    }

    fn reg_hive(key: &str) -> String {
        let (hive, path) = key.split_once('\\').unwrap_or((key, ""));
        let hive = match hive {
            "HKLM" => "HKEY_LOCAL_MACHINE",
            "HKCU" => "HKEY_CURRENT_USER",
            other => other,
        };
        format!("{hive}\\{path}")
    }

    fn reg_value(data: &RegData) -> String {
        match data {
            RegData::Number(number) => format!("dword:{number:08x}"),
            RegData::Expand(text) => {
                let bytes: Vec<String> = text
                    .encode_utf16()
                    .chain([0])
                    .flat_map(u16::to_le_bytes)
                    .map(|byte| format!("{byte:02x}"))
                    .collect();
                format!("hex(2):{}", bytes.join(","))
            }
            RegData::Text(text) => {
                format!("\"{}\"", text.replace('\\', "\\\\").replace('"', "\\\""))
            }
        }
    }

    fn reg_file(entries: &[RegEntry]) -> Vec<u8> {
        let mut lines = vec!["Windows Registry Editor Version 5.00".to_string()];
        let mut key = None;
        for entry in entries {
            if key != Some(&entry.key) {
                key = Some(&entry.key);
                lines.push(String::new());
                lines.push(format!("[{}]", reg_hive(&entry.key)));
            }
            if let Some((name, data)) = &entry.value {
                lines.push(format!("\"{name}\"={}", reg_value(data)));
            }
        }
        lines.push(String::new());
        let text = lines.join("\r\n");
        [0xff, 0xfe]
            .into_iter()
            .chain(text.encode_utf16().flat_map(u16::to_le_bytes))
            .collect()
    }

    fn install_entries() -> [RegEntry; 3] {
        [
            reg_text(
                r"HKLM\Software\Microsoft\Windows NT\CurrentVersion\AeDebug",
                "Debugger",
                "false",
            ),
            reg_text(
                r"HKLM\Software\Wow6432Node\Microsoft\Windows NT\CurrentVersion\AeDebug",
                "Debugger",
                "false",
            ),
            reg_text(
                r"HKCU\Software\Wine\DllOverrides",
                "winemenubuilder.exe",
                "",
            ),
        ]
    }

    fn adapter_guid(mac: &str) -> String {
        let digest = Md5::digest(format!("{}\n", mac.replace(':', "")));
        let hex: String = digest.iter().map(|byte| format!("{byte:02x}")).collect();
        format!(
            "{{{}-{}-{}-{}-{}}}",
            &hex[..8],
            &hex[8..12],
            &hex[12..16],
            &hex[16..20],
            &hex[20..]
        )
    }

    fn adapter_entries(mac: &str) -> Vec<RegEntry> {
        let guid = adapter_guid(mac);
        let class = format!(r"HKLM\SYSTEM\CurrentControlSet\Control\Class\{NET_CLASS}\0099");
        let connection =
            format!(r"HKLM\SYSTEM\CurrentControlSet\Control\Network\{NET_CLASS}\{guid}\Connection");
        let app = r"HKLM\Software\Wow6432Node\Famatech\RadminVPN\1.0";
        let desktop = r"HKCU\Control Panel\Desktop";
        let nt = r"HKLM\Software\Microsoft\Windows NT\CurrentVersion";
        let mut entries = vec![
            reg_text(&class, "NetCfgInstanceId", &guid),
            reg_text(&class, "MatchingDeviceId", &format!(r"{guid}\RvNetMP60")),
            reg_text(&connection, "Name", "Radmin VPN"),
            reg_text(&connection, "PnpInstanceID", r"ROOT\NET\0099"),
            reg_text(&format!(r"{app}\Firewall"), "AdapterId", &guid),
            reg_number(app, "PowerOn", 1),
            reg_number(&format!(r"{app}\update"), "Mode", 3),
            RegEntry {
                key: r"HKLM\SOFTWARE\Famatech\RadminVPN\1.0\Registration".to_string(),
                value: None,
            },
            reg_text(DRIVER_KEY, "DisplayName", "Radmin VPN TAP Bridge"),
            RegEntry {
                key: DRIVER_KEY.to_string(),
                value: Some((
                    "ImagePath".to_string(),
                    RegData::Expand(r"C:\windows\system32\drivers\rvpnnetmp.sys".to_string()),
                )),
            },
            reg_number(DRIVER_KEY, "Start", 2),
            reg_number(DRIVER_KEY, "Type", 1),
            reg_text(DRIVER_KEY, "Group", "NDIS"),
            reg_number(DRIVER_KEY, "ErrorControl", 0),
            reg_number(
                r"HKLM\SYSTEM\CurrentControlSet\Services\RvControlSvc",
                "Start",
                4,
            ),
            reg_number(MEMORY_KEY, "SystemPages", 0xFFFF_FFFF),
            reg_number(MEMORY_KEY, "ClearPageFileAtShutdown", 0),
            reg_number(MEMORY_KEY, "LargeSystemCache", 1),
            reg_text(r"HKCU\Software\Wine\X11 Driver", "Decorated", "N"),
            reg_text(desktop, "FontSmoothing", "2"),
            reg_number(desktop, "FontSmoothingType", 1),
            reg_text(
                r"HKCU\Software\Famatech\Radmin VPN\ui\MainWindow",
                "IsHidden",
                "false",
            ),
        ];
        entries.extend(BUNDLED_FONTS.map(|name| {
            let stem = Path::new(name).file_stem().unwrap_or_default();
            reg_text(
                &format!(r"{nt}\Fonts"),
                &format!("{} (TrueType)", stem.to_string_lossy()),
                name,
            )
        }));
        entries.push(reg_text(
            &format!(r"{nt}\FontSubstitutes"),
            "MS Shell Dlg 2",
            "Tahoma",
        ));
        entries.push(reg_text(
            r"HKCU\Software\Wine\Fonts\Replacements",
            "MS Shell Dlg 2",
            "Tahoma",
        ));
        entries
    }

    fn mac_file(paths: &Paths) -> PathBuf {
        paths.rvpn_state_dir().join("mac")
    }

    fn windows_dir(paths: &Paths) -> PathBuf {
        prefix(paths).join("drive_c").join("windows")
    }

    fn drivers_dir(paths: &Paths) -> PathBuf {
        windows_dir(paths).join("system32").join("drivers")
    }

    fn wine_env(paths: &Paths, base: impl Iterator<Item = (OsString, OsString)>) -> Env {
        let mut env: Env = base.collect();
        let bin = paths.wine_dir().join("bin");
        let mut search = bin.clone().into_os_string();
        search.push(":");
        if let Some(inherited) = env.get(OsStr::new("PATH")) {
            search.push(inherited);
        }
        env.extend(
            [
                ("WINEPREFIX", prefix(paths).into_os_string()),
                ("WINEDEBUG", "-all".into()),
                ("WINEDLLOVERRIDES", "winemenubuilder.exe=d".into()),
                ("MALLOC_ARENA_MAX", "2".into()),
                ("PATH", search),
                ("WINELOADER", bin.join("wine").into_os_string()),
                ("WINESERVER", bin.join("wineserver").into_os_string()),
            ]
            .map(|(name, value)| (name.into(), value)),
        );
        env
    }

    fn headless_env(env: &Env) -> Env {
        let mut env = env.clone();
        env.insert("WAYLAND_DISPLAY".into(), "none".into());
        env.remove(OsStr::new("DISPLAY"));
        env
    }

    fn service_env(env: &Env, shim: &Path) -> Env {
        let mut preload = shim.as_os_str().to_os_string();
        if let Some(inherited) = env.get(OsStr::new("LD_PRELOAD"))
            && !inherited.is_empty()
        {
            preload.push(":");
            preload.push(inherited);
        }
        let mut env = env.clone();
        env.insert(
            "WINEDLLOVERRIDES".into(),
            "mscoree=;mshtml=;netsh.exe=n".into(),
        );
        env.insert("WINE_LARGE_ADDRESS_AWARE".into(), "1".into());
        env.insert("LD_PRELOAD".into(), preload);
        env
    }

    fn gui_env(env: &Env) -> Env {
        let mut env = env.clone();
        env.insert("LIBGL_ALWAYS_SOFTWARE".into(), "1".into());
        env.insert("GALLIUM_DRIVER".into(), "llvmpipe".into());
        env
    }

    fn command(program: impl AsRef<OsStr>, env: &Env) -> Command {
        let mut command = Command::new(program);
        command.env_clear().envs(env);
        command
    }

    fn named(command: &Command, error: io::Error) -> io::Error {
        let program = command.get_program().to_string_lossy();
        io::Error::new(error.kind(), format!("{error}: '{program}'"))
    }

    fn spawn(command: &mut Command) -> io::Result<Child> {
        command.spawn().map_err(|error| named(command, error))
    }

    fn output(command: &mut Command) -> io::Result<Output> {
        command
            .stdin(Stdio::inherit())
            .output()
            .map_err(|error| named(command, error))
    }

    fn exit_status(child: &mut Child) -> Option<ExitStatus> {
        child.try_wait().ok().flatten()
    }

    fn live(slot: &mut Option<Child>) -> Option<&mut Child> {
        let child = slot.as_mut()?;
        exit_status(child).is_none().then_some(child)
    }

    fn wine_error(output: &Output) -> String {
        let stderr = String::from_utf8_lossy(&output.stderr)
            .replace("\r\n", "\n")
            .replace('\r', "\n");
        match stderr.trim() {
            "" => log::exit_text(output.status),
            text => text.to_string(),
        }
    }

    fn wineserver_stop(env: &Env) {
        for flag in ["-k", "-w"] {
            if output(command("wineserver", env).arg(flag)).is_err() {
                return;
            }
        }
    }

    fn kill_helpers(env: &Env) -> io::Result<()> {
        let mut pkill = Command::new("pkill");
        pkill
            .args(["-x", "tap_bridge"])
            .status()
            .map_err(|error| named(&pkill, error))?;
        wineserver_stop(env);
        Ok(())
    }

    fn rm(file: impl AsRef<Path>) {
        let _ = fs::remove_file(file);
    }

    fn copy(source: &Path, target: &Path) -> io::Result<()> {
        remove_if_present(target)?;
        copy_with_times(source, target)
    }

    fn is_fifo(file: &str) -> bool {
        fs::metadata(file).is_ok_and(|metadata| metadata.file_type().is_fifo())
    }

    fn patch_font_names(data: &mut [u8]) -> bool {
        let site = QT_NAME_SITE..QT_NAME_SITE + QT_NAME_READ.len();
        let cave = QT_NAME_CAVE..QT_NAME_CAVE + QT_NAME_CHECK.len();
        let pristine = data.get(site.clone()) == Some(&QT_NAME_READ[..])
            && data
                .get(cave.clone())
                .is_some_and(|bytes| bytes.iter().all(|byte| *byte == 0xcc));
        if pristine {
            data[site].copy_from_slice(&QT_NAME_JUMP);
            data[cave].copy_from_slice(&QT_NAME_CHECK);
        }
        pristine
    }

    fn patch_qt_font_names(file: &Path) -> io::Result<()> {
        let mut data = fs::read(file)?;
        if patch_font_names(&mut data) {
            fs::write(file, data)?;
        }
        Ok(())
    }

    fn privatize_desktop(paths: &Paths) -> io::Result<()> {
        let Ok(users) = fs::read_dir(prefix(paths).join("drive_c").join("users")) else {
            return Ok(());
        };
        for user in users.flatten() {
            let desktop = user.path().join("Desktop");
            if desktop.is_symlink() {
                fs::remove_file(&desktop)?;
                if let Err(error) = fs::create_dir(&desktop)
                    && !desktop.is_dir()
                {
                    return Err(error);
                }
            }
        }
        Ok(())
    }

    fn random_mac() -> io::Result<String> {
        let mut octets = [0x02; 6];
        File::open("/dev/urandom")?.read_exact(&mut octets[1..])?;
        Ok(octets.map(|octet| format!("{octet:02x}")).join(":"))
    }

    fn write_raw_mac(file: &Path, raw: &[u8]) -> io::Result<()> {
        OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .custom_flags(libc::O_NOFOLLOW)
            .open(file)?
            .write_all(raw)
    }

    fn setup_failed(error: io::Error) -> anyhow::Error {
        anyhow!(log::fail("Radmin VPN setup failed", error))
    }

    fn load_mac(file: &Path, raw_file: &Path) -> Result<String> {
        let mut mac = fs::read_to_string(file)
            .unwrap_or_default()
            .trim()
            .to_string();
        if !MAC_RE.is_match(&mac) {
            mac = random_mac().map_err(setup_failed)?;
            fs::write(file, &mac)
                .map_err(|error| anyhow!(log::fail("Adapter MAC file write failed", error)))?;
        }
        let raw: Vec<u8> = mac
            .split(':')
            .filter_map(|octet| u8::from_str_radix(octet, 16).ok())
            .collect();
        rm(raw_file);
        write_raw_mac(raw_file, &raw)
            .map_err(|error| anyhow!(log::fail("Adapter MAC file write failed", error)))?;
        Ok(mac)
    }

    fn relay_pickup(cmd_file: &Path, cmd_proc: &Path) -> Option<Vec<String>> {
        if fs::metadata(cmd_file).ok()?.len() == 0 {
            return None;
        }
        fs::rename(cmd_file, cmd_proc).ok()?;
        let text = fs::read(cmd_proc).ok()?;
        Some(netsh_scripts(&String::from_utf8_lossy(&text)))
    }

    fn runnable(file: &Path) -> bool {
        CString::new(file.as_os_str().as_bytes())
            .is_ok_and(|name| unsafe { libc::access(name.as_ptr(), libc::X_OK) } == 0)
            && !file.is_dir()
    }

    fn on_path(program: &str, search: Option<OsString>) -> bool {
        search.is_some_and(|search| {
            std::env::split_paths(&search).any(|dir| runnable(&dir.join(program)))
        })
    }

    fn single_root(folder: &Path) -> io::Result<PathBuf> {
        let mut entries = fs::read_dir(folder)?;
        if let (Some(only), None) = (entries.next(), entries.next()) {
            let only = only?.path();
            if only.is_dir() {
                return Ok(only);
            }
        }
        Ok(folder.to_path_buf())
    }

    fn fetch_wine(
        paths: &Paths,
        archive: &Path,
        tmp_dir: &Path,
        cancelled: &dyn Fn() -> bool,
    ) -> Result<()> {
        fs::create_dir_all(paths.bin_dir())?;
        let (_, asset_url) = github_asset(
            paths,
            &format!("https://api.github.com/repos/Kron4ek/Wine-Builds/releases/tags/{WINE_TAG}"),
            "staging-amd64-wow64.tar.xz",
        )?;
        let _ = fs::remove_dir_all(tmp_dir);
        fs::create_dir(tmp_dir)?;
        fetch_to(&asset_url, archive, None, Some(cancelled))?;
        let packed = XzReader::new(BufReader::new(File::open(archive)?), true);
        Archive::new(packed).unpack(tmp_dir)?;
        let staged = single_root(tmp_dir)?;
        if !staged.join("bin").join("wine").exists() {
            bail!("archive is incomplete");
        }
        let wine_dir = paths.wine_dir();
        if wine_dir.exists() {
            fs::remove_dir_all(&wine_dir)?;
        }
        Ok(fs::rename(staged, wine_dir)?)
    }

    pub fn preflight() -> Result<()> {
        if !on_path("pkexec", std::env::var_os("PATH")) {
            bail!(log::fail(
                "pkexec not found, a polkit agent is required",
                "missing dependency"
            ));
        }
        if !on_path("ip", std::env::var_os("PATH")) {
            bail!(log::fail("iproute2 not found", "missing dependency"));
        }
        Ok(())
    }

    pub fn ensure_wine(
        paths: &Paths,
        reporter: &Reporter,
        cancelled: &dyn Fn() -> bool,
    ) -> Result<()> {
        let wine = paths.wine_dir().join("bin").join("wine");
        let pinned = format!("wine-{WINE_TAG}");
        if let Ok(output) = Command::new(wine).arg("--version").output()
            && String::from_utf8_lossy(&output.stdout)
                .split_whitespace()
                .next()
                == Some(pinned.as_str())
        {
            return Ok(());
        }
        (reporter.update)("Fetching Wine");
        let archive = paths.bin_dir().join("_wine.tar.xz");
        let tmp_dir = paths.bin_dir().join(".wine.tmp");
        let outcome = fetch_wine(paths, &archive, &tmp_dir, cancelled)
            .map_err(|error| download_error("Wine download failed", error));
        rm(&archive);
        let _ = fs::remove_dir_all(&tmp_dir);
        outcome
    }

    pub fn uninstall(paths: &Paths) -> Result<()> {
        kill_helpers(&wine_env(paths, std::env::vars_os()))?;
        for file in RUNTIME_FILES {
            rm(file);
        }
        let _ = fs::remove_dir_all(prefix(paths));
        let _ = fs::remove_dir_all(paths.wine_dir());
        Ok(remove_if_present(&mac_file(paths))?)
    }

    #[derive(Default)]
    struct Event {
        flag: Mutex<bool>,
        changed: Condvar,
    }

    impl Event {
        fn set(&self) {
            *self.flag.lock().unwrap_or_else(PoisonError::into_inner) = true;
            self.changed.notify_all();
        }

        fn is_set(&self) -> bool {
            *self.flag.lock().unwrap_or_else(PoisonError::into_inner)
        }

        fn wait(&self, timeout: Duration) -> bool {
            let flag = self.flag.lock().unwrap_or_else(PoisonError::into_inner);
            let (flag, _) = self
                .changed
                .wait_timeout_while(flag, timeout, |flag| !*flag)
                .unwrap_or_else(PoisonError::into_inner);
            *flag
        }
    }

    #[derive(Default)]
    struct Proc(Mutex<Option<Child>>);

    impl Proc {
        fn holding(child: Child) -> Self {
            Self(Mutex::new(Some(child)))
        }

        fn slot(&self) -> MutexGuard<'_, Option<Child>> {
            self.0.lock().unwrap_or_else(PoisonError::into_inner)
        }

        fn hold(&self, child: Child) {
            *self.slot() = Some(child);
        }

        fn poll(&self) -> Option<ExitStatus> {
            self.slot().as_mut().and_then(exit_status)
        }

        fn running(&self) -> bool {
            live(&mut self.slot()).is_some()
        }

        fn signal(&self, signal: libc::c_int) -> io::Result<()> {
            let mut slot = self.slot();
            let Some(child) = live(&mut slot) else {
                return Ok(());
            };
            if unsafe { libc::kill(child.id() as libc::pid_t, signal) } == 0 {
                return Ok(());
            }
            match io::Error::last_os_error() {
                error if error.raw_os_error() == Some(libc::ESRCH) => Ok(()),
                error => Err(error),
            }
        }

        fn terminate(&self) {
            let _ = self.signal(libc::SIGTERM);
        }

        fn wait(&self, limit: Option<Duration>) -> bool {
            let deadline = limit.map(|limit| Instant::now() + limit);
            let mut step = WAIT_STEP_MIN;
            while self.running() {
                let left =
                    deadline.map(|deadline| deadline.saturating_duration_since(Instant::now()));
                if left.is_some_and(|left| left.is_zero()) {
                    return false;
                }
                step = (step * 2)
                    .min(left.unwrap_or(WAIT_STEP_MAX))
                    .min(WAIT_STEP_MAX);
                thread::sleep(step);
            }
            true
        }

        fn stop(&self) {
            if !self.running()
                || self.signal(libc::SIGTERM).is_err()
                || self.wait(Some(STOP_TIMEOUT))
            {
                return;
            }
            if self.signal(libc::SIGKILL).is_ok() {
                self.wait(None);
            }
        }
    }

    #[derive(Default)]
    struct Root {
        shell: Proc,
    }

    impl Root {
        fn open(&self) -> io::Result<()> {
            self.start(Command::new("pkexec").args(["sh", "-c", ROOT_LOOP]))
        }

        fn start(&self, command: &mut Command) -> io::Result<()> {
            let shell = spawn(
                command
                    .stdin(Stdio::piped())
                    .stdout(Stdio::null())
                    .stderr(Stdio::null()),
            )?;
            self.shell.hold(shell);
            Ok(())
        }

        fn alive(&self) -> bool {
            self.shell.running()
        }

        fn run(&self, script: &str) -> Result<(), String> {
            let mut slot = self.shell.slot();
            let stdin = live(&mut slot)
                .and_then(|shell| shell.stdin.as_mut())
                .ok_or("authorization was declined")?;
            stdin
                .write_all(format!("{script}\n").as_bytes())
                .map_err(|error| error.to_string())
        }

        fn close(&self) {
            let Some(mut shell) = self.shell.slot().take() else {
                return;
            };
            if let Some(mut stdin) = shell.stdin.take() {
                let _ = stdin.write_all(b"__END__\n");
            }
            let shell = Proc::holding(shell);
            if !shell.wait(Some(STOP_TIMEOUT)) {
                shell.stop();
            }
        }
    }

    struct Teardown<F: FnMut()>(F);

    impl<F: FnMut()> Drop for Teardown<F> {
        fn drop(&mut self) {
            (self.0)();
        }
    }

    fn guarded(
        reporter: &Reporter,
        steps: impl FnOnce() -> Result<()>,
        teardown: impl FnMut(),
    ) -> Result<()> {
        let _teardown = Teardown(teardown);
        report(steps(), reporter)
    }

    fn report(outcome: Result<()>, reporter: &Reporter) -> Result<()> {
        match outcome {
            Err(error) if !error.is::<CancelledError>() => {
                (reporter.fail)(&error.to_string());
                Ok(())
            }
            outcome => outcome,
        }
    }

    pub struct Session {
        paths: Paths,
        cancel: Event,
        relay_stop: Event,
        bridge: Proc,
        service: Proc,
        gui: Proc,
        env: Env,
        env_headless: Env,
        root: Root,
    }

    impl Session {
        pub fn new(paths: &Paths) -> Self {
            let env = wine_env(paths, std::env::vars_os());
            Self {
                paths: paths.clone(),
                cancel: Event::default(),
                relay_stop: Event::default(),
                bridge: Proc::default(),
                service: Proc::default(),
                gui: Proc::default(),
                env_headless: headless_env(&env),
                env,
                root: Root::default(),
            }
        }

        pub fn run(
            &self,
            installer: Option<&Path>,
            reporter: &Reporter,
            on_running: &dyn Fn(),
        ) -> Result<()> {
            thread::scope(|scope| {
                guarded(
                    reporter,
                    || self.sequence(scope, installer, reporter, on_running),
                    || self.teardown(),
                )
            })
        }

        pub fn stop(&self) {
            self.cancel.set();
            self.gui.terminate();
            self.service.terminate();
        }

        fn sequence<'scope>(
            &'scope self,
            scope: &'scope Scope<'scope, '_>,
            installer: Option<&Path>,
            reporter: &Reporter,
            on_running: &dyn Fn(),
        ) -> Result<()> {
            kill_helpers(&self.env).map_err(setup_failed)?;
            self.ensure_installed(installer, reporter)?;
            if self.cancel.is_set() {
                return Ok(());
            }
            self.root.open().map_err(setup_failed)?;
            self.stage_artifacts(reporter)?;
            let mac = load_mac(&mac_file(&self.paths), Path::new(MAC_RAW))?;
            if !self.create_device(
                &mac,
                Path::new(&format!("/sys/class/net/{TAP}/ifindex")),
                reporter,
            )? {
                return Ok(());
            }
            self.start_bridge(reporter)?;
            self.configure_registry(reporter, &mac)?;
            self.start_relay(scope).map_err(setup_failed)?;
            let vpn_ip = self.start_service(reporter)?;
            (reporter.update)("Configuring routes");
            self.configure_routes(&vpn_ip)
                .map_err(|error| anyhow!(log::fail("Routing failed", error)))?;
            self.launch_gui()?;
            on_running();
            self.wait_exit()
                .map_err(|error| anyhow!(log::fail("Radmin VPN stopped unexpectedly", error)))
        }

        fn init_prefix(&self) -> io::Result<Output> {
            fs::create_dir_all(prefix(&self.paths))?;
            let boot = self.wine(&["wineboot", "--init"])?;
            privatize_desktop(&self.paths)?;
            Ok(boot)
        }

        fn ensure_installed(&self, installer: Option<&Path>, reporter: &Reporter) -> Result<()> {
            if is_installed(&self.paths) {
                return Ok(());
            }
            let Some(installer) = installer.filter(|file| file.is_file()) else {
                bail!("Radmin VPN installer not found");
            };
            (reporter.update)("Installing");
            let boot = self
                .init_prefix()
                .map_err(|error| anyhow!(log::fail("Radmin VPN install failed", error)))?;
            if !boot.status.success() {
                log::fail("wineboot failed", wine_error(&boot));
                if !prefix(&self.paths).join("drive_c").is_dir() {
                    bail!("Radmin VPN install failed");
                }
            }
            self.reg_import(&install_entries())?;
            wineserver_stop(&self.env);
            let setup = spawn(
                command("wine", &self.env)
                    .arg(installer)
                    .args(["/VERYSILENT", "/NORESTART"])
                    .current_dir(self.paths.rvpn_state_dir())
                    .stdout(Stdio::null())
                    .stderr(Stdio::null()),
            )
            .map_err(|error| anyhow!(log::fail("Radmin VPN install failed", error)))?;
            let setup = Proc::holding(setup);
            let status = loop {
                if let Some(status) = setup.poll() {
                    break status;
                }
                if self.cancel.wait(INSTALLER_POLL) {
                    setup.stop();
                    wineserver_stop(&self.env);
                    return Ok(());
                }
            };
            for _ in 0..INSTALLED_CHECKS {
                if self.cancel.wait(INSTALLED_POLL) {
                    return Ok(());
                }
                if is_installed(&self.paths) {
                    wineserver_stop(&self.env);
                    self.scrub_ndis().map_err(setup_failed)?;
                    wineserver_stop(&self.env);
                    return Ok(());
                }
            }
            bail!(log::fail(
                "Radmin VPN install failed",
                format!("installer {}", log::exit_text(status))
            ))
        }

        fn stage(&self) -> io::Result<()> {
            privatize_desktop(&self.paths)?;
            let helpers = self.paths.rvpn_bin_dir();
            let app = app_dir(&self.paths);
            let windows = windows_dir(&self.paths);
            let drivers = drivers_dir(&self.paths);
            let syswow = windows.join("syswow64");
            let fonts = windows.join("Fonts");
            fs::create_dir_all(&drivers)?;
            fs::create_dir_all(&syswow)?;
            for (name, folder, staged) in [
                ("rvpnnetmp.sys", &drivers, "rvpnnetmp.sys"),
                ("adapter_hook.dll", &app, "adapter_hook.dll"),
                ("rvpn_launcher.exe", &app, "rvpn_launcher.exe"),
                ("netsh.exe", &syswow, "netsh.exe"),
                ("netsh64.exe", &windows.join("system32"), "netsh.exe"),
                ("drvinst.exe", &app, "drvinst.exe"),
            ] {
                copy(&helpers.join(name), &folder.join(staged))?;
            }
            fs::create_dir_all(&fonts)?;
            for name in BUNDLED_FONTS {
                copy(&helpers.join(name), &fonts.join(name))?;
            }
            patch_qt_font_names(&app.join("platforms").join("qwindows.dll"))?;
            self.scrub_ndis()
        }

        fn stage_artifacts(&self, reporter: &Reporter) -> Result<()> {
            (reporter.update)("Installing components");
            self.stage().map_err(setup_failed)
        }

        fn create_device(&self, mac: &str, ifindex: &Path, reporter: &Reporter) -> Result<bool> {
            (reporter.update)("Creating network device");
            let device = self.net_bringup(mac, ifindex);
            if self.cancel.is_set() {
                return Ok(false);
            }
            device.map_err(|error| anyhow!(log::fail("Network setup failed", error)))?;
            Ok(true)
        }

        fn net_bringup(&self, mac: &str, ifindex: &Path) -> Result<(), String> {
            if !MAC_RE.is_match(mac) {
                return Err("invalid adapter MAC".to_string());
            }
            let script = bringup_script(unsafe { libc::getuid() }, mac);
            let index = || fs::read_to_string(ifindex).unwrap_or_default();
            let stale = index();
            self.root.run(&script)?;
            for _ in 0..DEVICE_CHECKS {
                if !self.root.alive() {
                    return Err("authorization was declined".to_string());
                }
                let current = index();
                if !current.is_empty() && current != stale {
                    return Ok(());
                }
                if self.cancel.wait(DEVICE_POLL) {
                    return Ok(());
                }
            }
            Err("could not create the network device".to_string())
        }

        fn start_bridge(&self, reporter: &Reporter) -> Result<()> {
            (reporter.update)("Preparing network bridge");
            for fifo in FIFOS {
                rm(fifo);
            }
            let bridge = spawn(
                Command::new(self.paths.rvpn_bin_dir().join("tap_bridge"))
                    .stdout(Stdio::null())
                    .stderr(Stdio::null()),
            )
            .map_err(|error| anyhow!(log::fail("Network bridge failed to start", error)))?;
            self.bridge.hold(bridge);
            for _ in 0..FIFO_CHECKS {
                if is_fifo(FIFO_B2D) && is_fifo(FIFO_D2B_LOW) {
                    return Ok(());
                }
                if let Some(status) = self.bridge.poll() {
                    bail!(log::fail(
                        "Network bridge failed to start",
                        log::exit_text(status)
                    ));
                }
                thread::sleep(FIFO_POLL);
            }
            bail!(log::fail("Network bridge failed to start", "fifo timeout"))
        }

        fn configure_registry(&self, reporter: &Reporter, mac: &str) -> Result<()> {
            (reporter.update)("Configuring adapter");
            self.reg_import(&adapter_entries(mac))?;
            wineserver_stop(&self.env);
            Ok(())
        }

        fn start_relay<'scope>(&'scope self, scope: &'scope Scope<'scope, '_>) -> io::Result<()> {
            rm(CMD_FILE);
            rm(CMD_PROC);
            thread::Builder::new().spawn_scoped(scope, || self.relay_loop())?;
            Ok(())
        }

        fn relay_loop(&self) {
            while !self.relay_stop.is_set() {
                if let Some(scripts) = relay_pickup(Path::new(CMD_FILE), Path::new(CMD_PROC)) {
                    for script in scripts {
                        if let Err(error) = self.root.run(&script) {
                            log::fail("Applying the VPN address failed", error);
                        }
                    }
                    rm(CMD_PROC);
                }
                self.relay_stop.wait(RELAY_POLL);
            }
        }

        fn start_service(&self, reporter: &Reporter) -> Result<String> {
            (reporter.update)("Running daemon");
            let log_file = service_log(&self.paths);
            rm(&log_file);
            let env = service_env(&self.env, &self.paths.rvpn_bin_dir().join("net_shim.so"));
            let service = spawn(
                command("wine", &env)
                    .args(["rvpn_launcher.exe", "/run"])
                    .current_dir(app_dir(&self.paths))
                    .stdout(Stdio::null())
                    .stderr(Stdio::null()),
            )
            .map_err(|error| anyhow!(log::fail("Radmin VPN daemon failed to start", error)))?;
            self.service.hold(service);
            for _ in 0..SERVICE_TIMEOUT_S * 1000 / SERVICE_POLL.as_millis() as u32 {
                if self.cancel.wait(SERVICE_POLL) {
                    return Err(CancelledError.into());
                }
                match daemon_state(&read_utf16(&log_file)) {
                    Daemon::Outdated(version) => bail!(log::fail(
                        "This Radmin VPN installer is outdated",
                        format!("daemon {version}")
                    )),
                    Daemon::Ready(ip) => return Ok(ip.to_string()),
                    Daemon::Starting => {}
                }
                if let Some(status) = self.service.poll() {
                    bail!(log::fail(
                        "Radmin VPN daemon exited",
                        log::exit_text(status)
                    ));
                }
            }
            bail!(log::fail(
                "Radmin VPN daemon never became ready",
                format!("{SERVICE_TIMEOUT_S}s timeout")
            ))
        }

        fn configure_routes(&self, vpn_ip: &str) -> Result<(), String> {
            if !VPN_IP.is_match(vpn_ip) {
                return Err("invalid VPN IP".to_string());
            }
            self.root.run(&routes_script(vpn_ip))
        }

        fn launch_gui(&self) -> Result<()> {
            let gui = spawn(
                command("wine", &gui_env(&self.env))
                    .arg("RvRvpnGui.exe")
                    .current_dir(app_dir(&self.paths))
                    .stdout(Stdio::null())
                    .stderr(Stdio::null()),
            )
            .map_err(|error| anyhow!(log::fail("Radmin VPN GUI failed to start", error)))?;
            self.gui.hold(gui);
            Ok(())
        }

        fn wait_exit(&self) -> Result<(), String> {
            while !self.cancel.wait(EXIT_POLL) {
                if self.gui.poll().is_some() {
                    return Ok(());
                }
                for (name, helper) in [("bridge", &self.bridge), ("daemon", &self.service)] {
                    if let Some(status) = helper.poll()
                        && !self.cancel.is_set()
                    {
                        return Err(format!("{name} {}", log::exit_text(status)));
                    }
                }
            }
            Ok(())
        }

        fn teardown(&self) {
            self.relay_stop.set();
            wineserver_stop(&self.env);
            self.gui.stop();
            self.service.stop();
            self.bridge.stop();
            let _ = self.root.run(&teardown_script());
            self.root.close();
            for file in RUNTIME_FILES {
                rm(file);
            }
        }

        fn wine(&self, args: &[&str]) -> io::Result<Output> {
            output(
                command("wine", &self.env_headless)
                    .args(args)
                    .current_dir(self.paths.rvpn_state_dir()),
            )
        }

        fn reg_import(&self, entries: &[RegEntry]) -> Result<()> {
            let file = prefix(&self.paths).join("drive_c").join(REG_NAME);
            let import = fs::write(&file, reg_file(entries))
                .and_then(|()| self.wine(&["reg", "import", &format!("C:\\{REG_NAME}")]))
                .map_err(|error| anyhow!(log::fail("Registry import failed", error)))?;
            if !import.status.success() {
                log::fail("Registry import failed", wine_error(&import));
            }
            rm(file);
            Ok(())
        }

        fn scrub_ndis(&self) -> io::Result<()> {
            let driver = drivers_dir(&self.paths).join("RvNetMP60.sys");
            if !driver.exists() {
                return Ok(());
            }
            rm(driver);
            self.wine(&[
                "reg",
                "delete",
                r"HKLM\SYSTEM\CurrentControlSet\Services\RvNetMP60",
                "/f",
            ])?;
            Ok(())
        }
    }

    #[cfg(test)]
    mod tests {
        use std::fs::{FileTimes, Permissions};
        use std::io::{BufRead, ErrorKind};
        use std::os::unix::fs::{MetadataExt, PermissionsExt, symlink};
        use std::os::unix::process::ExitStatusExt;
        use std::sync::Arc;
        use std::time::{SystemTime, UNIX_EPOCH};

        use super::*;
        use crate::github::testing::{Reply, Server, seed};

        const CLOSE: &str = concat!(
            "firewall-cmd --zone=trusted --remove-interface=radminvpn0; ",
            r##"nft list chains | awk '$1 == "table" { f = $2; t = $3 } $1 == "chain" { c = $2 } "##,
            r##"/type filter hook input/ && f ~ /^(ip|ip6|inet)$/ { print f, t, c }' | "##,
            "while read -r f t c; do nft -a list chain $f $t $c | ",
            r##"sed -n 's/.*"radminvpn" # handle //p' | while read -r h; do "##,
            "nft delete rule $f $t $c handle $h; done; done; ",
            "iptables -D INPUT -i radminvpn0 -j ACCEPT; ip6tables -D INPUT -i radminvpn0 -j ACCEPT",
        );
        const OPEN: &str = concat!(
            "if firewall-cmd --state; then ",
            "firewall-cmd --zone=trusted --change-interface=radminvpn0; else ",
            r##"nft list chains | awk '$1 == "table" { f = $2; t = $3 } $1 == "chain" { c = $2 } "##,
            r##"/type filter hook input/ && f ~ /^(ip|ip6|inet)$/ { print f, t, c }' | "##,
            "while read -r f t c; do ",
            "nft insert rule $f $t $c iifname radminvpn0 accept comment radminvpn; done; ",
            "command -v nft && iptables --version | grep -q nf_tables || ",
            "{ iptables -I INPUT -i radminvpn0 -j ACCEPT; ip6tables -I INPUT -i radminvpn0 -j ACCEPT; }; fi",
        );
        const MAC: &str = "02:1a:2B:3c:4D:5e";
        const GUID: &str = "{fbff95f1-6f42-ce95-4b6b-ce1d0d445660}";
        const WINE_ROOT: &str = "wine-11.21-staging-amd64-wow64";
        const WINE_SCRIPT: &str = "#!/bin/sh\necho 'wine-11.21 (Staging)'\n";
        const WINE_TAR_XZ: &str = concat!(
            "fd377a585a000004e6d6b4460200210116000000742fe5a3e04fff02295d003b9a4a1f092f63a5fdeb039e8a9010b077",
            "328184be9dd6c2f3009228d738d2db51882ef7fd97e617c655e13bc90700478c8b9ec08d71f7213d69828d8de7c7d40b",
            "ea709a1c74376ae94a4c888bccf36896d91ae669ef9223b29bb9d82aa5c8db1aca095431f709f11a6ffeef6828b0f3fe",
            "f71f734a8268917e8cae37eeadb7e9bfc766be9a7bddefb10fc8fcb64018650ca5366d3975c98e742ff9960620fbb43d",
            "09273fe8f687dfa59cc99c8aabdd1be41f48e3816dea446bb99e811aff11bd037a6a50be0be36cf6bc658f3723da82b0",
            "5357ac3440c992a64879cfdd94c2e9715002109a45cb23a51606e51e2656ffbb69f0bfcc671cdde2645c9c3cb5cbe2f8",
            "6d9436177923292e739dc590ac1068b6a6bb1f56c5c1c3f907926654f06f3925919c8f51ac49d35177c056fb31a09230",
            "71f62128a5c43e70383f817e3a4b5c28a604c0e8059f8489229f700867b194de36eb472dc7a705eacb06a6a459e00830",
            "5a03a900dff14eddceb6925efa8d991b72940c29931b497bd5b0b0b8ead26fc2059e06e3781f107a40fa39f4af11f86b",
            "385e53684c07a80c16676ae1eabded027d405247377b0501813c59d67f1f507ec332600b42096874357983bb732837fb",
            "1cc577972dcf115657eadfb2596138b9259356c01f2b2fb5c3b72a4039747ef5c5454e554c8bec729419f6997164a52a",
            "6ae7172dbba631e896957e71026310b9a47b0f9ed24ae7e712b4d5d1e34d24bcdfb90af807b238028b6648897904a86d",
            "d8b038c80ed2000000000000bda70be94d0715100001c50480a001000027ca5ab1c467fb020000000004595a",
        );
        const INCOMPLETE_TAR_XZ: &str = concat!(
            "fd377a585a000004e6d6b44604c08001805021011c000000000000007e509a66e027ff00785d003b9a4a1f094455ee74",
            "b12ca7dce3bc0d4735fd1d84f41b50b19010b859ebfb82165aa5eff9d96627f8bee12cf66bc8f454a1a7ef655008ba01",
            "9256072f28d417da860e50eefeeb555af74f14a4bf2c3bfa2176e3b0603a62a916582a7b9d6c6c1f1c6f216d8324ea5b",
            "7836fee239609cb6f3631cc8e0b100004adc548d921dfcd300019c0180500000deb37415b1c467fb020000000004595a",
        );

        const FAKE_WINE: &str = r#"#!/bin/sh
calls="${WINEPREFIX%/rvpn/wineprefix}/calls"
if [ "${0##*/}" = wineserver ]; then
    printf '%s|%s|%s\n' "wineserver $*" "${DISPLAY-unset}" "${WAYLAND_DISPLAY-unset}" >> "$calls"
    exit 0
fi
printf '%s|%s|%s|%s\n' "wine $*" "$(pwd -P)" "${DISPLAY-unset}" "${WAYLAND_DISPLAY-unset}" >> "$calls"
"#;
        const INSTALL: &str = r#"app="$WINEPREFIX/drive_c/Program Files (x86)/Radmin VPN"
drivers="$WINEPREFIX/drive_c/windows/system32/drivers"
case "$1" in
wineboot)
    mkdir -p "$WINEPREFIX/drive_c/users/steamuser"
    ln -s .. "$WINEPREFIX/drive_c/users/steamuser/Desktop"
    echo ' boot trouble ' >&2
    exit 3
    ;;
reg)
    if [ "$2" = import ]; then cp "$WINEPREFIX/drive_c/rvpn.reg" "$WINEPREFIX/imported.reg"; fi
    if [ -e "$drivers/RvNetMP60.sys" ]; then echo 'driver still there' >> "$calls"; fi
    ;;
*)
    mkdir -p "$app" "$drivers"
    : > "$drivers/RvNetMP60.sys"
    : > "$app/RvControlSvc.exe"
    ;;
esac"#;
        const HANGING_INSTALL: &str = r#"case "$1" in
wineboot) mkdir -p "$WINEPREFIX/drive_c" ;;
reg) ;;
*)
    echo $$ > "$WINEPREFIX/installer.pid"
    exec sleep 600
    ;;
esac"#;
        const DAEMON: &str = r#"printf '%s|%s|%s\n' "$WINEDLLOVERRIDES" "$WINE_LARGE_ADDRESS_AWARE" "$LD_PRELOAD" >> "$calls"
log="$WINEPREFIX/drive_c/ProgramData/Famatech/Radmin VPN/service"
[ -e "$WINEPREFIX/next.log" ] || exit 5
cp "$WINEPREFIX/next.log" "$log.tmp"
mv "$log.tmp" "$log.log"
exec sleep 600"#;

        fn rooted(dir: &tempfile::TempDir) -> Paths {
            paths(&fs::canonicalize(dir.path()).unwrap())
        }

        fn scripted(paths: &Paths, body: &str) -> Session {
            let bin = paths.wine_dir().join("bin");
            fs::create_dir_all(&bin).unwrap();
            for name in ["wine", "wineserver"] {
                let mut writer = Command::new("sh")
                    .args(["-c", r#"cat > "$0" && chmod 755 "$0""#])
                    .arg(bin.join(name))
                    .stdin(Stdio::piped())
                    .spawn()
                    .unwrap();
                let mut script = writer.stdin.take().unwrap();
                writeln!(script, "{FAKE_WINE}{body}").unwrap();
                drop(script);
                assert!(writer.wait().unwrap().success());
            }
            let mut session = Session::new(paths);
            session.env.insert("DISPLAY".into(), ":7".into());
            session.env.remove(OsStr::new("WAYLAND_DISPLAY"));
            session.env.remove(OsStr::new("LD_PRELOAD"));
            session
        }

        fn calls(paths: &Paths) -> Vec<String> {
            let root = paths.data_root.parent().unwrap().display().to_string();
            fs::read_to_string(paths.data_root.join("calls"))
                .unwrap_or_default()
                .lines()
                .map(|line| line.replace(&root, "~"))
                .collect()
        }

        fn hex(text: &str) -> Vec<u8> {
            (0..text.len())
                .step_by(2)
                .map(|at| u8::from_str_radix(&text[at..at + 2], 16).unwrap())
                .collect()
        }

        fn paths(root: &Path) -> Paths {
            Paths::new(root.join("asset"), root.join("data"))
        }

        fn recorder() -> (Reporter, Arc<Mutex<Vec<String>>>) {
            let seen = Arc::new(Mutex::new(Vec::new()));
            let (updates, fails) = (seen.clone(), seen.clone());
            let reporter = Reporter {
                update: Box::new(move |text| {
                    updates.lock().unwrap().push(format!("update {text}"))
                }),
                fail: Box::new(move |text| fails.lock().unwrap().push(format!("fail {text}"))),
            };
            (reporter, seen)
        }

        fn utf16(text: &str) -> Vec<u8> {
            text.encode_utf16().flat_map(u16::to_le_bytes).collect()
        }

        fn signal(process: &Proc) -> Option<i32> {
            process.poll().and_then(|status| status.signal())
        }

        fn base_env(pairs: &[(&str, &str)]) -> impl Iterator<Item = (OsString, OsString)> {
            pairs
                .iter()
                .map(|(name, value)| (OsString::from(name), OsString::from(value)))
                .collect::<Vec<_>>()
                .into_iter()
        }

        fn value<'a>(env: &'a Env, name: &str) -> Option<&'a str> {
            env.get(OsStr::new(name)).and_then(|value| value.to_str())
        }

        fn shell(script: &str) -> Child {
            Command::new("sh")
                .args(["-c", script])
                .stdin(Stdio::piped())
                .spawn()
                .unwrap()
        }

        fn moment(seconds: u64) -> SystemTime {
            UNIX_EPOCH + Duration::from_secs(seconds)
        }

        fn seconds(time: SystemTime) -> u64 {
            time.duration_since(UNIX_EPOCH).unwrap().as_secs()
        }

        fn serve(body: Vec<u8>) -> String {
            Server::start(move |_| Reply::new(200, body.clone()))
                .url(&format!("/{WINE_ROOT}.tar.xz"))
        }

        fn seed_release(paths: &Paths, asset_url: &str) {
            assert_eq!(WINE_TAG, "11.21");
            seed(
                paths,
                "https://api.github.com/repos/Kron4ek/Wine-Builds/releases/tags/11.21",
                &[
                    ("wine-11.21-amd64.tar.xz", "http://127.0.0.1:9/other"),
                    (&format!("{WINE_ROOT}.tar.xz"), asset_url),
                ],
            );
        }

        #[test]
        fn firewall_scripts_open_and_close_the_tap() {
            assert_eq!(firewall_close(), CLOSE);
            assert_eq!(firewall_open(), OPEN);
            assert_eq!(
                teardown_script(),
                format!("ip link delete radminvpn0; {CLOSE}")
            );
        }

        #[test]
        fn bringup_script_is_one_line() {
            let script = bringup_script(1000, MAC);
            assert_eq!(
                script,
                format!(
                    "modprobe tun; ip link delete radminvpn0; \
                     ip tuntap add dev radminvpn0 mode tap user 1000; \
                     ip link set radminvpn0 address 02:1a:2B:3c:4D:5e; \
                     ip link set radminvpn0 up; ip link set radminvpn0 multicast on; \
                     ip link set radminvpn0 allmulticast on; \
                     sysctl -w net.ipv4.conf.radminvpn0.rp_filter=0; \
                     sysctl -w net.ipv4.conf.radminvpn0.accept_local=1; \
                     ip route add 224.0.2.60/32 dev radminvpn0; \
                     nmcli device set radminvpn0 managed no; {CLOSE}; {OPEN}"
                )
            );
            assert!(!script.contains('\n'));
        }

        #[test]
        fn routes_script_sets_address_and_routes() {
            assert_eq!(
                routes_script("26.12.34.56"),
                "ip addr add 26.12.34.56/8 dev radminvpn0; ip link set radminvpn0 up; \
                 ip route replace 26.0.0.0/8 dev radminvpn0; \
                 ip route append 255.255.255.255/32 dev radminvpn0 metric 0; \
                 ip route append 224.0.0.0/4 dev radminvpn0 metric 0"
            );
        }

        #[test]
        fn netsh_lines_become_address_scripts() {
            let script = |ip: &str, cidr: &str| {
                format!("ip addr add {ip}/{cidr} dev radminvpn0; ip link set radminvpn0 up")
            };
            assert_eq!(
                netsh_scripts("ip addr add 26.221.171.186/8\n"),
                [script("26.221.171.186", "8")]
            );
            assert_eq!(
                netsh_scripts("ip addr add 26.1.2.3/8\nip addr add 26.4.5.6/16\n"),
                [script("26.1.2.3", "8"), script("26.4.5.6", "16")]
            );
            assert_eq!(
                netsh_scripts("ip addr add 26.1.2.3/8 ip addr add 26.9.9.9/8\n"),
                [script("26.1.2.3", "8")]
            );
            assert_eq!(
                netsh_scripts("ip addr add 26.1.1.1/8\r\nip addr add 26.2.2.2/8"),
                [script("26.1.1.1", "8"), script("26.2.2.2", "8")]
            );
            assert_eq!(
                netsh_scripts("xx ip addr add 26.1.2.3/8 dev whatever; rm -rf /\n"),
                [script("26.1.2.3", "8")]
            );
            assert_eq!(
                netsh_scripts("ip addr add 26.1.2.3/123456\n"),
                [script("26.1.2.3", "123456")]
            );
            let six = |ip: &str| format!("ip -6 addr add {ip}/64 dev radminvpn0");
            assert_eq!(
                netsh_scripts(
                    "ip addr add 26.31.211.43/8\nip -6 addr add fdfd::1a1f:d32b/64 dev radminvpn0\n"
                ),
                [script("26.31.211.43", "8"), six("fdfd::1a1f:d32b")]
            );
            assert_eq!(
                netsh_scripts("ip -6 addr add FDFD:0:0:0:0:0:1A1F:D32B/64 dev radminvpn0\n"),
                [six("fdfd::1a1f:d32b")]
            );
            for ignored in [
                "ip -6 addr add fe80::1/64 dev radminvpn0\n",
                "ip -6 addr add fd00::1/64 dev radminvpn0\n",
                "ip -6 addr add fdfd::1a1f:d32b/128 dev radminvpn0\n",
                "ip -6 addr add fdfd:::1/64 dev radminvpn0\n",
                "ip -6 addr add fdfd::1/64 dev eth0\n",
                "ip addr add 10.1.2.3/8\nip addr add 126.1.2.3/8\n",
                "ip addr add 10.0.0.1/8 ip addr add 26.9.9.9/8\n",
                "interface ip add address name=Radmin VPN addr=26.1.2.3 mask=255.0.0.0\n\n",
                "ip addr add 26.1.2.3\n",
                "ip addr add 26.1.2.3.4/8\n",
                "",
            ] {
                assert!(netsh_scripts(ignored).is_empty(), "{ignored:?}");
            }
        }

        #[test]
        fn vpn_ip_comes_from_the_last_matching_log_line() {
            let log = "2026-11-10 21:08:01 Service version: 2.1.4951.1\n\
                       2026-11-10 21:08:02 IP: 0.0.0.0\n\
                       2026-11-10 21:08:03 adapter opened\n\
                       2026-11-10 21:08:04 IP: 26.221.171.186 mask 255.0.0.0\n\
                       2026-11-10 21:08:04 adapter ready\n";
            assert_eq!(extract_ip(log), Some("26.221.171.186"));
            assert_eq!(
                extract_ip(&log.replace("adapter ready", "adapter opened")),
                None
            );
            for (text, ip) in [
                (
                    "adapter ready\nRegistered as 26.1.2.3 (id 5)\nIP: 26.9.9.9\n",
                    Some("26.9.9.9"),
                ),
                (
                    "adapter ready\nIP: 26.9.9.9\nRegistered as 26.1.2.3\n",
                    Some("26.1.2.3"),
                ),
                ("adapter ready\nIP: 0.0.0.0\n", None),
                ("adapter ready\nIP: 0.0.0.0 26.1.1.1\n", None),
                (
                    "adapter ready\nRegistered as 0.0.0.0 then 26.4.4.4\n",
                    Some("26.4.4.4"),
                ),
                ("adapter ready\nIP: 10.0.0.5\n", None),
                (
                    "adapter ready\nIP: 26.7.7.7\nRegistered as nothing\n",
                    Some("26.7.7.7"),
                ),
                ("adapter ready\nIP: 126.5.6.7\n", Some("26.5.6.7")),
                ("adapter ready\r\nIP: 26.2.3.4\r\n", Some("26.2.3.4")),
                ("", None),
            ] {
                assert_eq!(extract_ip(text), ip, "{text:?}");
            }
        }

        #[test]
        fn guards_accept_only_plain_macs_and_radmin_addresses() {
            for (mac, valid) in [
                (MAC, true),
                ("02:1a:2b:3c:4d:5", false),
                ("02-1a-2b-3c-4d-5e", false),
                (" 02:1a:2b:3c:4d:5e", false),
                ("02:1a:2b:3c:4d:5e:6f", false),
                ("0g:1a:2b:3c:4d:5e", false),
                ("02:1a:2b:3c:4d:5e; reboot", false),
                ("", false),
            ] {
                assert_eq!(MAC_RE.is_match(mac), valid, "{mac:?}");
            }
            for (ip, valid) in [
                ("26.1.2.3", true),
                ("26.999.999.999", true),
                ("26.1.2", false),
                ("126.1.2.3", false),
                ("26.1.2.3 ", false),
                ("26.1.2.3;x", false),
                ("27.1.2.3", false),
                ("26.1.2.3.4", false),
            ] {
                assert_eq!(VPN_IP.is_match(ip), valid, "{ip:?}");
            }
        }

        #[test]
        fn registry_values_use_the_regedit_notation() {
            assert_eq!(reg_value(&RegData::Number(0xFFFF_FFFF)), "dword:ffffffff");
            assert_eq!(reg_value(&RegData::Number(3)), "dword:00000003");
            assert_eq!(
                reg_value(&RegData::Text("a\\b\"c".to_string())),
                "\"a\\\\b\\\"c\""
            );
            assert_eq!(reg_value(&RegData::Text(String::new())), "\"\"");
            assert_eq!(
                reg_value(&RegData::Expand("C:\\é".to_string())),
                "hex(2):43,00,3a,00,5c,00,e9,00,00,00"
            );
            assert_eq!(
                reg_hive(r"HKLM\Software\Wine"),
                r"HKEY_LOCAL_MACHINE\Software\Wine"
            );
            assert_eq!(
                reg_hive(r"HKCU\Control Panel"),
                r"HKEY_CURRENT_USER\Control Panel"
            );
        }

        #[test]
        fn adapter_guid_is_the_md5_of_the_mac_digits() {
            assert_eq!(adapter_guid(MAC), GUID);
            assert_eq!(
                adapter_guid("02:00:00:00:00:00"),
                "{44b53441-24e7-3123-df3a-421c2eac2979}"
            );
        }

        #[test]
        fn registry_file_for_the_install() {
            let text = [
                "Windows Registry Editor Version 5.00",
                "",
                r#"[HKEY_LOCAL_MACHINE\Software\Microsoft\Windows NT\CurrentVersion\AeDebug]"#,
                r#""Debugger"="false""#,
                "",
                r#"[HKEY_LOCAL_MACHINE\Software\Wow6432Node\Microsoft\Windows NT\CurrentVersion\AeDebug]"#,
                r#""Debugger"="false""#,
                "",
                r#"[HKEY_CURRENT_USER\Software\Wine\DllOverrides]"#,
                r#""winemenubuilder.exe"="""#,
                "",
            ]
            .join("\r\n");
            let file = reg_file(&install_entries());
            assert_eq!(file[..2], [0xff, 0xfe]);
            assert_eq!(file[2..], utf16(&text));
        }

        #[test]
        fn registry_file_for_the_adapter() {
            let text = [
                "Windows Registry Editor Version 5.00",
                "",
                r#"[HKEY_LOCAL_MACHINE\SYSTEM\CurrentControlSet\Control\Class\{4d36e972-e325-11ce-bfc1-08002be10318}\0099]"#,
                r#""NetCfgInstanceId"="{fbff95f1-6f42-ce95-4b6b-ce1d0d445660}""#,
                r#""MatchingDeviceId"="{fbff95f1-6f42-ce95-4b6b-ce1d0d445660}\\RvNetMP60""#,
                "",
                r#"[HKEY_LOCAL_MACHINE\SYSTEM\CurrentControlSet\Control\Network\{4d36e972-e325-11ce-bfc1-08002be10318}\{fbff95f1-6f42-ce95-4b6b-ce1d0d445660}\Connection]"#,
                r#""Name"="Radmin VPN""#,
                r#""PnpInstanceID"="ROOT\\NET\\0099""#,
                "",
                r#"[HKEY_LOCAL_MACHINE\Software\Wow6432Node\Famatech\RadminVPN\1.0\Firewall]"#,
                r#""AdapterId"="{fbff95f1-6f42-ce95-4b6b-ce1d0d445660}""#,
                "",
                r#"[HKEY_LOCAL_MACHINE\Software\Wow6432Node\Famatech\RadminVPN\1.0]"#,
                r#""PowerOn"=dword:00000001"#,
                "",
                r#"[HKEY_LOCAL_MACHINE\Software\Wow6432Node\Famatech\RadminVPN\1.0\update]"#,
                r#""Mode"=dword:00000003"#,
                "",
                r#"[HKEY_LOCAL_MACHINE\SOFTWARE\Famatech\RadminVPN\1.0\Registration]"#,
                "",
                r#"[HKEY_LOCAL_MACHINE\SYSTEM\CurrentControlSet\Services\rvpnnetmp]"#,
                r#""DisplayName"="Radmin VPN TAP Bridge""#,
                concat!(
                    r#""ImagePath"=hex(2):43,00,3a,00,5c,00,77,00,69,00,6e,00,64,00,6f,00,77,00,73,00,"#,
                    "5c,00,73,00,79,00,73,00,74,00,65,00,6d,00,33,00,32,00,5c,00,64,00,72,00,69,00,",
                    "76,00,65,00,72,00,73,00,5c,00,72,00,76,00,70,00,6e,00,6e,00,65,00,74,00,6d,00,",
                    "70,00,2e,00,73,00,79,00,73,00,00,00",
                ),
                r#""Start"=dword:00000002"#,
                r#""Type"=dword:00000001"#,
                r#""Group"="NDIS""#,
                r#""ErrorControl"=dword:00000000"#,
                "",
                r#"[HKEY_LOCAL_MACHINE\SYSTEM\CurrentControlSet\Services\RvControlSvc]"#,
                r#""Start"=dword:00000004"#,
                "",
                r#"[HKEY_LOCAL_MACHINE\System\CurrentControlSet\Control\Session Manager\Memory Management]"#,
                r#""SystemPages"=dword:ffffffff"#,
                r#""ClearPageFileAtShutdown"=dword:00000000"#,
                r#""LargeSystemCache"=dword:00000001"#,
                "",
                r#"[HKEY_CURRENT_USER\Software\Wine\X11 Driver]"#,
                r#""Decorated"="N""#,
                "",
                r#"[HKEY_CURRENT_USER\Control Panel\Desktop]"#,
                r#""FontSmoothing"="2""#,
                r#""FontSmoothingType"=dword:00000001"#,
                "",
                r#"[HKEY_CURRENT_USER\Software\Famatech\Radmin VPN\ui\MainWindow]"#,
                r#""IsHidden"="false""#,
                "",
                r#"[HKEY_LOCAL_MACHINE\Software\Microsoft\Windows NT\CurrentVersion\Fonts]"#,
                r#""OpenSans-Regular (TrueType)"="OpenSans-Regular.ttf""#,
                r#""OpenSans-Bold (TrueType)"="OpenSans-Bold.ttf""#,
                "",
                r#"[HKEY_LOCAL_MACHINE\Software\Microsoft\Windows NT\CurrentVersion\FontSubstitutes]"#,
                r#""MS Shell Dlg 2"="Tahoma""#,
                "",
                r#"[HKEY_CURRENT_USER\Software\Wine\Fonts\Replacements]"#,
                r#""MS Shell Dlg 2"="Tahoma""#,
                "",
            ]
            .join("\r\n");
            let file = reg_file(&adapter_entries(MAC));
            assert_eq!(file[..2], [0xff, 0xfe]);
            assert_eq!(
                String::from_utf16(
                    &file[2..]
                        .as_chunks()
                        .0
                        .iter()
                        .copied()
                        .map(u16::from_le_bytes)
                        .collect::<Vec<_>>()
                )
                .unwrap(),
                text
            );
        }

        #[test]
        fn font_patch_jumps_land_in_the_cave() {
            let jump = |base: usize, at: usize, code: &[u8]| {
                let offset = i32::from_le_bytes(code[at + 1..at + 5].try_into().unwrap());
                (base + at + 5).wrapping_add_signed(offset as isize)
            };
            assert_eq!(jump(QT_NAME_SITE, 0, &QT_NAME_JUMP), QT_NAME_CAVE);
            let back = QT_NAME_SITE + QT_NAME_READ.len();
            assert_eq!(jump(QT_NAME_CAVE, 21, &QT_NAME_CHECK), back);
            assert_eq!(jump(QT_NAME_CAVE, 29, &QT_NAME_CHECK), back);
        }

        #[test]
        fn font_patch_applies_once_and_only_to_the_known_build() {
            let mut pristine = vec![0u8; QT_NAME_SITE + QT_NAME_READ.len() + 7];
            pristine[QT_NAME_SITE..QT_NAME_SITE + 11].copy_from_slice(&QT_NAME_READ);
            pristine[QT_NAME_CAVE..QT_NAME_CAVE + 34].fill(0xcc);

            let mut data = pristine.clone();
            assert!(patch_font_names(&mut data));
            assert_eq!(data[QT_NAME_SITE..QT_NAME_SITE + 11], QT_NAME_JUMP);
            assert_eq!(data[QT_NAME_CAVE..QT_NAME_CAVE + 34], QT_NAME_CHECK);
            let changed = data
                .iter()
                .zip(&pristine)
                .filter(|(now, before)| now != before);
            assert_eq!(changed.count(), 11 + 34);
            let patched = data.clone();
            assert!(!patch_font_names(&mut data));
            assert_eq!(data, patched);

            let mut used_cave = pristine.clone();
            used_cave[QT_NAME_CAVE + 33] = 0x90;
            let mut other_site = pristine.clone();
            other_site[QT_NAME_SITE] = 0x8a;
            let mut short = pristine.clone();
            short.truncate(QT_NAME_SITE + 10);
            for mut data in [used_cave, other_site, short, Vec::new()] {
                let before = data.clone();
                assert!(!patch_font_names(&mut data));
                assert_eq!(data, before);
            }
        }

        #[test]
        fn font_patch_rewrites_the_file_only_when_it_applies() {
            let dir = tempfile::tempdir().unwrap();
            let file = dir.path().join("qwindows.dll");
            let mut data = vec![0u8; QT_NAME_SITE + 32];
            data[QT_NAME_SITE..QT_NAME_SITE + 11].copy_from_slice(&QT_NAME_READ);
            data[QT_NAME_CAVE..QT_NAME_CAVE + 34].fill(0xcc);
            fs::write(&file, &data).unwrap();
            patch_qt_font_names(&file).unwrap();
            let patched = fs::read(&file).unwrap();
            assert_eq!(patched[QT_NAME_SITE..QT_NAME_SITE + 11], QT_NAME_JUMP);
            File::open(&file)
                .unwrap()
                .set_times(FileTimes::new().set_modified(moment(1_700_000_000)))
                .unwrap();
            patch_qt_font_names(&file).unwrap();
            assert_eq!(fs::read(&file).unwrap(), patched);
            let untouched = fs::metadata(&file).unwrap().modified().unwrap();
            assert_eq!(seconds(untouched), 1_700_000_000);
            assert!(patch_qt_font_names(&dir.path().join("missing.dll")).is_err());
        }

        #[test]
        fn wine_environment_extends_the_launcher_environment() {
            let paths = Paths::new(PathBuf::from("/opt/app"), PathBuf::from("/data"));
            let env = wine_env(
                &paths,
                base_env(&[
                    ("PATH", "/usr/bin:/bin"),
                    ("HOME", "/home/user"),
                    ("DISPLAY", ":0"),
                    ("WINEDEBUG", "+all"),
                ]),
            );
            assert_eq!(value(&env, "WINEPREFIX"), Some("/data/rvpn/wineprefix"));
            assert_eq!(value(&env, "WINEDEBUG"), Some("-all"));
            assert_eq!(
                value(&env, "WINEDLLOVERRIDES"),
                Some("winemenubuilder.exe=d")
            );
            assert_eq!(value(&env, "MALLOC_ARENA_MAX"), Some("2"));
            assert_eq!(
                value(&env, "PATH"),
                Some("/data/bin/wine/bin:/usr/bin:/bin")
            );
            assert_eq!(value(&env, "WINELOADER"), Some("/data/bin/wine/bin/wine"));
            assert_eq!(
                value(&env, "WINESERVER"),
                Some("/data/bin/wine/bin/wineserver")
            );
            assert_eq!(value(&env, "HOME"), Some("/home/user"));
            assert_eq!(value(&env, "DISPLAY"), Some(":0"));
            assert_eq!(env.len(), 9);

            let bare = wine_env(&paths, base_env(&[]));
            assert_eq!(value(&bare, "PATH"), Some("/data/bin/wine/bin:"));
            assert_eq!(bare.len(), 7);
        }

        #[test]
        fn environments_for_headless_daemon_and_window() {
            let paths = Paths::new(PathBuf::from("/opt/app"), PathBuf::from("/data"));
            let env = wine_env(
                &paths,
                base_env(&[("DISPLAY", ":0"), ("LD_PRELOAD", "/x.so")]),
            );
            let shim = Path::new("/opt/app/rvpn/net_shim.so");

            let headless = headless_env(&env);
            assert_eq!(value(&headless, "WAYLAND_DISPLAY"), Some("none"));
            assert_eq!(value(&headless, "DISPLAY"), None);
            assert_eq!(headless.len(), env.len());
            assert_eq!(value(&env, "DISPLAY"), Some(":0"));
            assert_eq!(value(&env, "WAYLAND_DISPLAY"), None);

            let service = service_env(&env, shim);
            assert_eq!(
                value(&service, "WINEDLLOVERRIDES"),
                Some("mscoree=;mshtml=;netsh.exe=n")
            );
            assert_eq!(value(&service, "WINE_LARGE_ADDRESS_AWARE"), Some("1"));
            assert_eq!(
                value(&service, "LD_PRELOAD"),
                Some("/opt/app/rvpn/net_shim.so:/x.so")
            );
            assert_eq!(service.len(), env.len() + 1);
            assert_eq!(
                value(&env, "WINEDLLOVERRIDES"),
                Some("winemenubuilder.exe=d")
            );
            for base in [base_env(&[]), base_env(&[("LD_PRELOAD", "")])] {
                let service = service_env(&wine_env(&paths, base), shim);
                assert_eq!(
                    value(&service, "LD_PRELOAD"),
                    Some("/opt/app/rvpn/net_shim.so")
                );
            }

            let gui = gui_env(&env);
            assert_eq!(value(&gui, "LIBGL_ALWAYS_SOFTWARE"), Some("1"));
            assert_eq!(value(&gui, "GALLIUM_DRIVER"), Some("llvmpipe"));
            assert_eq!(value(&gui, "DISPLAY"), Some(":0"));
            assert_eq!(gui.len(), env.len() + 2);
        }

        #[test]
        fn commands_get_exactly_the_session_environment_and_its_path() {
            let dir = tempfile::tempdir().unwrap();
            let fake = dir.path().join("wine");
            fs::write(
                &fake,
                "#!/bin/sh\necho fake \"$@\" \"${HOME-unset}\" \"$WINEDEBUG\"\n",
            )
            .unwrap();
            fs::set_permissions(&fake, Permissions::from_mode(0o755)).unwrap();
            let mut search = dir.path().as_os_str().to_os_string();
            search.push(":");
            search.push(std::env::var_os("PATH").unwrap());
            let env: Env = [
                (OsString::from("PATH"), search),
                (OsString::from("WINEDEBUG"), OsString::from("-all")),
            ]
            .into();
            let done = output(command("wine", &env).args(["reg", "import"])).unwrap();
            assert_eq!(
                String::from_utf8_lossy(&done.stdout),
                "fake reg import unset -all\n"
            );
            assert!(done.status.success());
        }

        #[test]
        fn start_errors_name_the_program() {
            let env = Env::new();
            let error = output(&mut command("rvpn-missing-program", &env)).unwrap_err();
            assert_eq!(error.kind(), ErrorKind::NotFound);
            assert!(
                error.to_string().ends_with(": 'rvpn-missing-program'"),
                "{error}"
            );
            let error = spawn(&mut Command::new("/nonexistent/tap_bridge")).unwrap_err();
            assert!(
                error.to_string().ends_with(": '/nonexistent/tap_bridge'"),
                "{error}"
            );
        }

        #[test]
        fn wine_errors_prefer_stderr_over_the_exit_code() {
            let failed = |stderr: &[u8]| Output {
                status: ExitStatus::from_raw(3 << 8),
                stdout: b"ignored".to_vec(),
                stderr: stderr.to_vec(),
            };
            assert_eq!(
                wine_error(&failed(b"  bad key\r\nsecond line \r\n")),
                "bad key\nsecond line"
            );
            assert_eq!(wine_error(&failed(b"a\rb")), "a\nb");
            assert_eq!(wine_error(&failed(b"")), "exit=3");
            assert_eq!(wine_error(&failed(b" \n\t")), "exit=3");
            assert_eq!(wine_error(&failed(b"caf\xff")), "caf\u{fffd}");
            let killed = Output {
                status: ExitStatus::from_raw(libc::SIGKILL),
                stdout: Vec::new(),
                stderr: Vec::new(),
            };
            assert_eq!(wine_error(&killed), "signal: 9 (SIGKILL)");
        }

        #[test]
        fn copy_replaces_the_target_and_keeps_mode_and_times() {
            let dir = tempfile::tempdir().unwrap();
            let source = dir.path().join("netsh64.exe");
            fs::write(&source, "new helper").unwrap();
            fs::set_permissions(&source, Permissions::from_mode(0o755)).unwrap();
            let times = FileTimes::new()
                .set_accessed(moment(1_600_000_000))
                .set_modified(moment(1_700_000_000));
            File::open(&source).unwrap().set_times(times).unwrap();

            let victim = dir.path().join("victim");
            fs::write(&victim, "old").unwrap();
            let target = dir.path().join("netsh.exe");
            symlink(&victim, &target).unwrap();
            copy(&source, &target).unwrap();
            assert!(!target.is_symlink());
            let metadata = fs::metadata(&target).unwrap();
            assert_eq!(metadata.mode() & 0o7777, 0o755);
            assert_eq!(seconds(metadata.modified().unwrap()), 1_700_000_000);
            assert_eq!(
                metadata.accessed().unwrap(),
                fs::metadata(&source).unwrap().accessed().unwrap()
            );
            assert_eq!(fs::read_to_string(&victim).unwrap(), "old");
            assert_eq!(fs::read_to_string(&target).unwrap(), "new helper");

            fs::set_permissions(&source, Permissions::from_mode(0o444)).unwrap();
            copy(&source, &target).unwrap();
            assert_eq!(fs::metadata(&target).unwrap().mode() & 0o7777, 0o444);
            copy(&source, &target).unwrap();
            assert!(copy(&dir.path().join("missing"), &target).is_err());
            assert!(!target.exists());
        }

        #[test]
        fn desktop_links_become_private_folders() {
            let dir = tempfile::tempdir().unwrap();
            let paths = paths(dir.path());
            privatize_desktop(&paths).unwrap();
            let users = prefix(&paths).join("drive_c").join("users");
            let home = dir.path().join("home-desktop");
            fs::create_dir_all(&home).unwrap();
            fs::write(home.join("note"), "x").unwrap();
            fs::create_dir_all(users.join("alice")).unwrap();
            fs::create_dir_all(users.join("bob").join("Desktop")).unwrap();
            fs::write(users.join("bob").join("Desktop").join("own"), "x").unwrap();
            fs::create_dir_all(users.join("carol")).unwrap();
            fs::write(users.join("plain"), "x").unwrap();
            symlink(&home, users.join("alice").join("Desktop")).unwrap();
            symlink("gone", users.join("carol").join("Desktop")).unwrap();
            privatize_desktop(&paths).unwrap();
            for user in ["alice", "carol"] {
                let desktop = users.join(user).join("Desktop");
                assert!(desktop.is_dir() && !desktop.is_symlink());
                assert_eq!(fs::read_dir(&desktop).unwrap().count(), 0);
            }
            assert!(home.join("note").exists());
            assert!(users.join("bob").join("Desktop").join("own").exists());
        }

        #[test]
        fn stored_mac_is_reused_and_written_raw() {
            let dir = tempfile::tempdir().unwrap();
            let file = dir.path().join("mac");
            let raw = dir.path().join("rvpn_mac");
            fs::write(&file, " 02:1A:2b:3c:4d:5e\n").unwrap();
            let victim = dir.path().join("victim");
            fs::write(&victim, "keep").unwrap();
            symlink(&victim, &raw).unwrap();
            assert_eq!(load_mac(&file, &raw).unwrap(), "02:1A:2b:3c:4d:5e");
            assert_eq!(fs::read_to_string(&file).unwrap(), " 02:1A:2b:3c:4d:5e\n");
            assert!(!raw.is_symlink());
            assert_eq!(
                fs::read(&raw).unwrap(),
                [0x02, 0x1a, 0x2b, 0x3c, 0x4d, 0x5e]
            );
            assert_eq!(fs::metadata(&raw).unwrap().mode() & 0o7777 & !0o600, 0);
            assert_eq!(fs::read_to_string(&victim).unwrap(), "keep");
            assert_eq!(load_mac(&file, &raw).unwrap(), "02:1A:2b:3c:4d:5e");
        }

        #[test]
        fn missing_or_invalid_mac_is_replaced_by_a_local_one() {
            let dir = tempfile::tempdir().unwrap();
            let raw = dir.path().join("rvpn_mac");
            let invalid: [Option<&[u8]>; 5] = [
                None,
                Some(b"02:1a:2b:3c:4d"),
                Some(b"\0\0\0\0"),
                Some(b""),
                Some(b"02:1a:2b:\xff"),
            ];
            for stored in invalid {
                let file = dir.path().join("mac");
                rm(&file);
                if let Some(stored) = stored {
                    fs::write(&file, stored).unwrap();
                }
                let mac = load_mac(&file, &raw).unwrap();
                assert!(MAC_RE.is_match(&mac) && mac.starts_with("02:"), "{mac}");
                assert_eq!(mac, mac.to_lowercase());
                assert_eq!(fs::read_to_string(&file).unwrap(), mac);
                let octets = fs::read(&raw).unwrap();
                let shown: Vec<String> =
                    octets.iter().map(|octet| format!("{octet:02x}")).collect();
                assert_eq!(shown.join(":"), mac);
            }
            assert_ne!(random_mac().unwrap(), random_mac().unwrap());
        }

        #[test]
        fn unwritable_mac_files_are_reported() {
            let dir = tempfile::tempdir().unwrap();
            let raw = dir.path().join("rvpn_mac");
            let error = load_mac(&dir.path().join("no").join("mac"), &raw).unwrap_err();
            assert_eq!(error.to_string(), "Adapter MAC file write failed");

            let file = dir.path().join("mac");
            fs::write(&file, MAC).unwrap();
            let error = load_mac(&file, &dir.path().join("no").join("raw")).unwrap_err();
            assert_eq!(error.to_string(), "Adapter MAC file write failed");

            fs::write(&raw, "taken").unwrap();
            assert!(write_raw_mac(&raw, b"x").is_err());
            assert_eq!(fs::read_to_string(&raw).unwrap(), "taken");
        }

        #[test]
        fn relay_takes_the_command_file_only_with_content() {
            let dir = tempfile::tempdir().unwrap();
            let cmd = dir.path().join("rvpn_netsh_cmd");
            let taken = dir.path().join("rvpn_netsh_cmd.proc");
            assert_eq!(relay_pickup(&cmd, &taken), None);
            fs::write(&cmd, "").unwrap();
            assert_eq!(relay_pickup(&cmd, &taken), None);
            assert!(cmd.exists() && !taken.exists());

            fs::write(&cmd, "ip addr add 26.1.2.3/8\nnoise\n").unwrap();
            fs::write(&taken, "stale").unwrap();
            assert_eq!(
                relay_pickup(&cmd, &taken),
                Some(vec![
                    "ip addr add 26.1.2.3/8 dev radminvpn0; ip link set radminvpn0 up".to_string()
                ])
            );
            assert!(!cmd.exists());
            assert_eq!(
                fs::read_to_string(&taken).unwrap(),
                "ip addr add 26.1.2.3/8\nnoise\n"
            );

            fs::write(&cmd, b"\xff\xfe\nip addr add 26.4.5.6/8\n").unwrap();
            assert_eq!(
                relay_pickup(&cmd, &taken).map(|scripts| scripts.len()),
                Some(1)
            );
            fs::write(&cmd, "noise\n").unwrap();
            assert_eq!(relay_pickup(&cmd, &taken), Some(Vec::new()));
            fs::write(&cmd, "x").unwrap();
            assert_eq!(
                relay_pickup(&cmd, &dir.path().join("no").join("proc")),
                None
            );
            assert!(cmd.exists());
        }

        #[test]
        fn path_lookup_needs_an_executable_file() {
            let dir = tempfile::tempdir().unwrap();
            let bin = dir.path().join("bin");
            fs::create_dir_all(bin.join("folder")).unwrap();
            fs::write(bin.join("tool"), "").unwrap();
            fs::set_permissions(bin.join("tool"), Permissions::from_mode(0o755)).unwrap();
            fs::write(bin.join("plain"), "").unwrap();
            fs::set_permissions(bin.join("plain"), Permissions::from_mode(0o644)).unwrap();
            symlink("tool", bin.join("alias")).unwrap();
            symlink("gone", bin.join("broken")).unwrap();
            let search = |dirs: &[&Path]| Some(std::env::join_paths(dirs).unwrap());
            let other = dir.path().join("other");
            for (program, found) in [
                ("tool", true),
                ("alias", true),
                ("plain", false),
                ("folder", false),
                ("broken", false),
                ("missing", false),
            ] {
                assert_eq!(
                    on_path(program, search(&[&other, &bin])),
                    found,
                    "{program}"
                );
            }
            assert!(!on_path("tool", search(&[&other])));
            assert!(!on_path("tool", None));
        }

        #[test]
        fn single_root_unwraps_one_top_folder_only() {
            let dir = tempfile::tempdir().unwrap();
            let root = dir.path();
            assert_eq!(single_root(root).unwrap(), root);
            fs::create_dir(root.join("wine")).unwrap();
            assert_eq!(single_root(root).unwrap(), root.join("wine"));
            fs::write(root.join("extra"), "x").unwrap();
            assert_eq!(single_root(root).unwrap(), root);
            fs::remove_dir(root.join("wine")).unwrap();
            assert_eq!(single_root(root).unwrap(), root);
            assert!(single_root(&root.join("missing")).is_err());
        }

        #[test]
        fn wine_is_fetched_unpacked_and_moved_into_place() {
            let dir = tempfile::tempdir().unwrap();
            let paths = paths(dir.path());
            seed_release(&paths, &serve(hex(WINE_TAR_XZ)));
            fs::create_dir_all(paths.bin_dir().join(".wine.tmp").join("stale")).unwrap();
            fs::create_dir_all(paths.wine_dir().join("broken")).unwrap();
            let (reporter, seen) = recorder();
            ensure_wine(&paths, &reporter, &|| false).unwrap();
            assert_eq!(*seen.lock().unwrap(), ["update Fetching Wine"]);
            let wine = paths.wine_dir();
            assert_eq!(
                fs::read_to_string(wine.join("bin").join("wine")).unwrap(),
                WINE_SCRIPT
            );
            assert!(!wine.join("broken").exists());
            let inf = fs::metadata(wine.join("share").join("wine").join("wine.inf")).unwrap();
            assert_eq!(seconds(inf.modified().unwrap()), 1760698800);
            let mut left: Vec<_> = fs::read_dir(paths.bin_dir())
                .unwrap()
                .map(|entry| entry.unwrap().file_name())
                .collect();
            left.sort();
            assert_eq!(left, ["api_cache.json", "wine"]);

            ensure_wine(&paths, &reporter, &|| panic!("nothing to fetch")).unwrap();
            assert_eq!(seen.lock().unwrap().len(), 1);
        }

        #[test]
        fn wine_of_another_version_is_replaced() {
            for (old, mode) in [
                ("#!/bin/sh\necho 'wine-11.20 (Staging)'\n", 0o755),
                ("old", 0o644),
            ] {
                let dir = tempfile::tempdir().unwrap();
                let paths = paths(dir.path());
                seed_release(&paths, &serve(hex(WINE_TAR_XZ)));
                let loader = paths.wine_dir().join("bin").join("wine");
                fs::create_dir_all(loader.parent().unwrap()).unwrap();
                fs::write(&loader, old).unwrap();
                fs::set_permissions(&loader, Permissions::from_mode(mode)).unwrap();
                let (reporter, seen) = recorder();
                ensure_wine(&paths, &reporter, &|| false).unwrap();
                assert_eq!(*seen.lock().unwrap(), ["update Fetching Wine"]);
                assert_eq!(fs::read_to_string(&loader).unwrap(), WINE_SCRIPT);
            }
        }

        #[test]
        fn failed_wine_downloads_leave_nothing_behind() {
            for (body, message) in [
                (b"not an archive".to_vec(), "Wine download failed"),
                (hex(INCOMPLETE_TAR_XZ), "Wine download failed"),
            ] {
                let dir = tempfile::tempdir().unwrap();
                let paths = paths(dir.path());
                seed_release(&paths, &serve(body));
                let (reporter, seen) = recorder();
                let error = ensure_wine(&paths, &reporter, &|| false).unwrap_err();
                assert_eq!(error.to_string(), message);
                assert_eq!(*seen.lock().unwrap(), ["update Fetching Wine"]);
                assert_eq!(fs::read_dir(paths.bin_dir()).unwrap().count(), 1);
            }
        }

        #[test]
        fn cancelled_wine_downloads_stay_silent() {
            let dir = tempfile::tempdir().unwrap();
            let paths = paths(dir.path());
            seed_release(&paths, &serve(hex(WINE_TAR_XZ)));
            let error = ensure_wine(&paths, &recorder().0, &|| true).unwrap_err();
            assert!(error.is::<CancelledError>());
            assert_eq!(fs::read_dir(paths.bin_dir()).unwrap().count(), 1);
        }

        #[test]
        fn events_wake_waiters_and_stay_set() {
            let event = Event::default();
            assert!(!event.is_set());
            let started = Instant::now();
            assert!(!event.wait(Duration::from_millis(30)));
            assert!(started.elapsed() >= Duration::from_millis(30));
            thread::scope(|scope| {
                scope.spawn(|| {
                    thread::sleep(Duration::from_millis(30));
                    event.set();
                });
                let started = Instant::now();
                assert!(event.wait(Duration::from_secs(20)));
                assert!(started.elapsed() < Duration::from_secs(10));
            });
            assert!(event.is_set());
            assert!(event.wait(Duration::ZERO));
        }

        #[test]
        fn processes_report_how_they_ended() {
            let absent = Proc::default();
            assert_eq!(absent.poll(), None);
            assert!(!absent.running());
            absent.terminate();
            absent.stop();
            assert!(absent.wait(None));

            let exited = Proc::holding(shell("exit 7"));
            assert!(exited.wait(Some(Duration::from_secs(20))));
            let code = || exited.poll().and_then(|status| status.code());
            assert_eq!(code(), Some(7));
            assert_eq!(code(), Some(7));
            exited.terminate();
            exited.stop();

            let sleeper = Proc::default();
            sleeper.hold(Command::new("sleep").arg("600").spawn().unwrap());
            assert!(sleeper.running());
            assert_eq!(sleeper.poll(), None);
            assert!(!sleeper.wait(Some(Duration::from_millis(30))));
            sleeper.terminate();
            assert!(sleeper.wait(Some(Duration::from_secs(20))));
            assert_eq!(signal(&sleeper), Some(libc::SIGTERM));
        }

        #[test]
        fn stop_ends_with_sigterm_first_and_sigkill_after_the_grace_period() {
            let polite = Proc::holding(Command::new("sleep").arg("600").spawn().unwrap());
            let started = Instant::now();
            polite.stop();
            assert_eq!(signal(&polite), Some(libc::SIGTERM));
            assert!(started.elapsed() < STOP_TIMEOUT);

            let mut deaf = Command::new("sh")
                .args(["-c", "trap '' TERM; echo ready; read line"])
                .stdin(Stdio::piped())
                .stdout(Stdio::piped())
                .spawn()
                .unwrap();
            let mut ready = String::new();
            BufReader::new(deaf.stdout.take().unwrap())
                .read_line(&mut ready)
                .unwrap();
            assert_eq!(ready, "ready\n");
            let stubborn = Proc::holding(deaf);
            let started = Instant::now();
            stubborn.stop();
            assert!(started.elapsed() >= STOP_TIMEOUT);
            assert_eq!(signal(&stubborn), Some(libc::SIGKILL));
        }

        #[test]
        fn root_shell_runs_lines_until_the_end_marker() {
            let dir = tempfile::tempdir().unwrap();
            let out = dir.path().join("out");
            let root = Root::default();
            assert!(!root.alive());
            assert_eq!(
                root.run("true"),
                Err("authorization was declined".to_string())
            );
            root.close();

            root.start(Command::new("sh").args(["-c", ROOT_LOOP]))
                .unwrap();
            assert!(root.alive());
            let line = |text: &str| format!("printf '%s\\n' {text} >> '{}'", out.display());
            root.run(&line("one")).unwrap();
            root.run(&format!(
                "false; missing-command; {}",
                line("'two  spaced'")
            ))
            .unwrap();
            root.run(&format!(
                "  {} | cat; {}",
                line("three"),
                line(r#""a\\b" '$HOME'"#)
            ))
            .unwrap();
            root.close();
            assert_eq!(
                fs::read_to_string(&out).unwrap(),
                "one\ntwo  spaced\nthree\na\\b\n$HOME\n"
            );
            assert!(!root.alive());
            assert_eq!(
                root.run("true"),
                Err("authorization was declined".to_string())
            );
            root.close();
        }

        #[test]
        fn root_shell_reports_a_declined_authorization() {
            let root = Root::default();
            root.start(&mut Command::new("false")).unwrap();
            assert!(root.shell.wait(Some(Duration::from_secs(20))));
            assert!(!root.alive());
            assert_eq!(
                root.run("true"),
                Err("authorization was declined".to_string())
            );
            let started = Instant::now();
            root.close();
            assert!(started.elapsed() < STOP_TIMEOUT);
            assert!(
                root.start(&mut Command::new("rvpn-missing-program"))
                    .is_err()
            );
        }

        #[test]
        fn root_shell_ends_on_its_own_when_the_launcher_goes_away() {
            let root = Root::default();
            root.start(Command::new("sh").args(["-c", ROOT_LOOP]))
                .unwrap();
            let mut shell = root.shell.slot().take().unwrap();
            drop(shell.stdin.take());
            assert!(shell.wait().unwrap().success());
        }

        #[test]
        fn only_expected_failures_reach_the_reporter() {
            let (reporter, seen) = recorder();
            assert!(report(Ok(()), &reporter).is_ok());
            assert!(report(Err(anyhow!("Network setup failed")), &reporter).is_ok());
            let missing = io::Error::from_raw_os_error(libc::ENOENT);
            assert!(report(Err(missing.into()), &reporter).is_ok());
            let cancelled = report(Err(CancelledError.into()), &reporter).unwrap_err();
            assert!(cancelled.is::<CancelledError>());
            assert_eq!(
                *seen.lock().unwrap(),
                [
                    "fail Network setup failed",
                    "fail No such file or directory (os error 2)"
                ]
            );
        }

        #[test]
        fn failures_are_reported_before_the_teardown() {
            let (reporter, seen) = recorder();
            let teardown = || seen.lock().unwrap().push("teardown".to_string());
            assert!(guarded(&reporter, || bail!("Network setup failed"), teardown).is_ok());
            assert_eq!(
                *seen.lock().unwrap(),
                ["fail Network setup failed", "teardown"]
            );
            assert!(guarded(&reporter, || Ok(()), teardown).is_ok());
            let cancelled = guarded(&reporter, || Err(CancelledError.into()), teardown);
            assert!(cancelled.unwrap_err().is::<CancelledError>());
            let crash = std::panic::AssertUnwindSafe(|| {
                guarded(&reporter, || panic!("unexpected"), teardown)
            });
            assert!(std::panic::catch_unwind(crash).is_err());
            assert_eq!(seen.lock().unwrap()[1..], ["teardown"; 4]);
        }

        #[test]
        fn install_needs_an_installer_file() {
            let dir = tempfile::tempdir().unwrap();
            let paths = paths(dir.path());
            let session = Session::new(&paths);
            let (reporter, seen) = recorder();
            for installer in [
                None,
                Some(dir.path()),
                Some(&*dir.path().join("missing.exe")),
            ] {
                let error = session.ensure_installed(installer, &reporter).unwrap_err();
                assert_eq!(error.to_string(), "Radmin VPN installer not found");
            }
            fs::create_dir_all(app_dir(&paths)).unwrap();
            fs::write(app_dir(&paths).join("RvControlSvc.exe"), "MZ").unwrap();
            session.ensure_installed(None, &reporter).unwrap();
            assert!(seen.lock().unwrap().is_empty());
            assert!(log::take_recorded().is_empty());
        }

        #[test]
        fn first_install_order() {
            let dir = tempfile::tempdir().unwrap();
            let paths = rooted(&dir);
            let session = scripted(&paths, INSTALL);
            let installer = paths.data_root.join("Radmin_VPN_2.1.exe");
            fs::write(&installer, "MZ").unwrap();
            let (reporter, seen) = recorder();
            session
                .ensure_installed(Some(&installer), &reporter)
                .unwrap();
            assert_eq!(*seen.lock().unwrap(), ["update Installing"]);
            assert_eq!(log::take_recorded(), ["wineboot failed - boot trouble"]);
            assert_eq!(
                calls(&paths),
                [
                    "wine wineboot --init|~/data/rvpn|unset|none",
                    r"wine reg import C:\rvpn.reg|~/data/rvpn|unset|none",
                    "wineserver -k|:7|unset",
                    "wineserver -w|:7|unset",
                    "wine ~/data/Radmin_VPN_2.1.exe /VERYSILENT /NORESTART|~/data/rvpn|:7|unset",
                    "wineserver -k|:7|unset",
                    "wineserver -w|:7|unset",
                    concat!(
                        r"wine reg delete HKLM\SYSTEM\CurrentControlSet\Services\RvNetMP60 /f",
                        "|~/data/rvpn|unset|none"
                    ),
                    "wineserver -k|:7|unset",
                    "wineserver -w|:7|unset",
                ]
            );
            assert_eq!(
                fs::read(prefix(&paths).join("imported.reg")).unwrap(),
                reg_file(&install_entries())
            );
            let drive = prefix(&paths).join("drive_c");
            assert!(!drive.join(REG_NAME).exists());
            let desktop = drive.join("users").join("steamuser").join("Desktop");
            assert!(desktop.is_dir() && !desktop.is_symlink());
            assert!(!drivers_dir(&paths).join("RvNetMP60.sys").exists());
            assert!(is_installed(&paths));
        }

        #[test]
        fn a_wineboot_that_leaves_no_prefix_ends_the_install() {
            let dir = tempfile::tempdir().unwrap();
            let paths = rooted(&dir);
            let session = scripted(&paths, "echo ' boot crash ' >&2\nexit 11");
            let installer = paths.data_root.join("Radmin_VPN_2.1.exe");
            fs::write(&installer, "MZ").unwrap();
            let (reporter, _seen) = recorder();
            let error = session
                .ensure_installed(Some(&installer), &reporter)
                .unwrap_err();
            assert_eq!(error.to_string(), "Radmin VPN install failed");
            assert_eq!(log::take_recorded(), ["wineboot failed - boot crash"]);
            assert_eq!(
                calls(&paths),
                ["wine wineboot --init|~/data/rvpn|unset|none"]
            );
        }

        #[test]
        fn stopping_during_the_install_ends_the_installer() {
            let dir = tempfile::tempdir().unwrap();
            let paths = rooted(&dir);
            let session = scripted(&paths, HANGING_INSTALL);
            let installer = paths.data_root.join("setup.exe");
            fs::write(&installer, "MZ").unwrap();
            let pid_file = prefix(&paths).join("installer.pid");
            let (reporter, seen) = recorder();
            thread::scope(|scope| {
                scope.spawn(|| {
                    for _ in 0..2000 {
                        if fs::read_to_string(&pid_file).is_ok_and(|pid| pid.ends_with('\n')) {
                            break;
                        }
                        thread::sleep(Duration::from_millis(10));
                    }
                    session.stop();
                });
                session
                    .ensure_installed(Some(&installer), &reporter)
                    .unwrap();
            });
            let pid = fs::read_to_string(&pid_file).unwrap();
            let pid: libc::pid_t = pid.trim().parse().unwrap();
            assert_eq!(unsafe { libc::kill(pid, 0) }, -1);
            assert_eq!(*seen.lock().unwrap(), ["update Installing"]);
            assert!(log::take_recorded().is_empty());
            assert_eq!(
                calls(&paths)[4..],
                [
                    "wine ~/data/setup.exe /VERYSILENT /NORESTART|~/data/rvpn|:7|unset",
                    "wineserver -k|:7|unset",
                    "wineserver -w|:7|unset",
                ]
            );
            assert!(!is_installed(&paths));
        }

        #[test]
        fn registry_imports_log_failures_and_clean_up() {
            let dir = tempfile::tempdir().unwrap();
            let paths = rooted(&dir);
            let mut session = scripted(&paths, "echo ' key refused ' >&2\nexit 2");
            let file = prefix(&paths).join("drive_c").join(REG_NAME);
            let error = session.reg_import(&install_entries()).unwrap_err();
            assert_eq!(error.to_string(), "Registry import failed");
            assert_eq!(
                log::take_recorded(),
                ["Registry import failed - No such file or directory (os error 2)"]
            );
            session.scrub_ndis().unwrap();
            assert!(calls(&paths).is_empty());

            fs::create_dir_all(prefix(&paths).join("drive_c")).unwrap();
            session.reg_import(&install_entries()).unwrap();
            assert_eq!(
                log::take_recorded(),
                ["Registry import failed - key refused"]
            );
            assert!(!file.exists());
            assert_eq!(
                calls(&paths),
                [r"wine reg import C:\rvpn.reg|~/data/rvpn|unset|none"]
            );

            session
                .env_headless
                .insert("PATH".into(), paths.data_root.clone().into());
            let error = session.reg_import(&install_entries()).unwrap_err();
            assert_eq!(error.to_string(), "Registry import failed");
            let logged = log::take_recorded();
            assert!(
                logged.len() == 1 && logged[0].ends_with(": 'wine'"),
                "{logged:?}"
            );
            assert_eq!(fs::read(&file).unwrap(), reg_file(&install_entries()));
            assert_eq!(calls(&paths).len(), 1);
        }

        #[test]
        fn daemon_log_is_judged_version_first() {
            let ready = "Service version: 2.1.4951.1\r\nadapter ready\r\nIP: 26.221.171.186\r\n";
            assert_eq!(daemon_state(ready), Daemon::Ready("26.221.171.186"));
            assert_eq!(
                daemon_state(&ready.replace("2.1.4951.1", "1.4.4642.1")),
                Daemon::Outdated("1.4.4642.1")
            );
            assert_eq!(
                daemon_state("Service version: 1.4.4642.1\n"),
                Daemon::Outdated("1.4.4642.1")
            );
            for current in [
                "adapter ready\nIP: 26.1.2.3\n",
                "Service version: 2.0.4899.9\nadapter ready\nRegistered as 26.1.2.3\n",
                "Service version: 11.4\nadapter ready\nIP: 26.1.2.3\n",
                "Service version: 2.1\nService version: 1.4\nadapter ready\nIP: 26.1.2.3\n",
            ] {
                assert_eq!(
                    daemon_state(current),
                    Daemon::Ready("26.1.2.3"),
                    "{current:?}"
                );
            }
            for starting in [
                "",
                "Service version: 2.1.4951.1\r\n",
                "Service version: 2.1.4951.1\nIP: 26.1.2.3\n",
                "Service version: 2.1.4951.1\nadapter ready\nIP: 0.0.0.0\n",
            ] {
                assert_eq!(daemon_state(starting), Daemon::Starting, "{starting:?}");
            }
        }

        #[test]
        fn daemon_start_follows_the_service_log() {
            let ready = "Service version: 2.1.4951.1\r\nadapter ready\r\nIP: 26.221.171.186\r\n";
            let outdated = ready.replace("2.1.4951.1", "1.4.4642.1");
            let start = |next: Option<&str>, stopped: bool| {
                let dir = tempfile::tempdir().unwrap();
                let paths = rooted(&dir);
                let session = scripted(&paths, DAEMON);
                let log_file = service_log(&paths);
                fs::create_dir_all(log_file.parent().unwrap()).unwrap();
                fs::create_dir_all(app_dir(&paths)).unwrap();
                fs::write(&log_file, utf16("adapter ready\nIP: 26.9.9.9\n")).unwrap();
                if let Some(next) = next {
                    fs::write(prefix(&paths).join("next.log"), utf16(next)).unwrap();
                }
                if stopped {
                    session.stop();
                }
                let (reporter, seen) = recorder();
                let outcome = session.start_service(&reporter);
                session.service.stop();
                assert_eq!(*seen.lock().unwrap(), ["update Running daemon"]);
                (outcome, log::take_recorded(), calls(&paths))
            };

            let (outcome, logged, seen) = start(Some(ready), false);
            assert_eq!(outcome.unwrap(), "26.221.171.186");
            assert!(logged.is_empty());
            assert_eq!(
                seen,
                [
                    concat!(
                        "wine rvpn_launcher.exe /run",
                        "|~/data/rvpn/wineprefix/drive_c/Program Files (x86)/Radmin VPN|:7|unset"
                    ),
                    "mscoree=;mshtml=;netsh.exe=n|1|~/asset/rvpn/net_shim.so",
                ]
            );

            let (outcome, logged, _) = start(Some(&outdated), false);
            assert_eq!(
                outcome.unwrap_err().to_string(),
                "This Radmin VPN installer is outdated"
            );
            assert_eq!(
                logged,
                ["This Radmin VPN installer is outdated - daemon 1.4.4642.1"]
            );

            let (outcome, logged, _) = start(None, false);
            assert_eq!(outcome.unwrap_err().to_string(), "Radmin VPN daemon exited");
            assert_eq!(logged, ["Radmin VPN daemon exited - exit=5"]);

            let (outcome, logged, _) = start(Some(ready), true);
            assert!(outcome.unwrap_err().is::<CancelledError>());
            assert!(logged.is_empty());
        }

        #[test]
        fn daemon_and_window_start_in_the_program_folder() {
            let dir = tempfile::tempdir().unwrap();
            let paths = rooted(&dir);
            let session = scripted(
                &paths,
                r#"printf '%s|%s\n' "$LIBGL_ALWAYS_SOFTWARE" "$GALLIUM_DRIVER" >> "$calls""#,
            );
            let (reporter, seen) = recorder();
            let error = session.start_service(&reporter).unwrap_err();
            assert_eq!(error.to_string(), "Radmin VPN daemon failed to start");
            let error = session.launch_gui().unwrap_err();
            assert_eq!(error.to_string(), "Radmin VPN GUI failed to start");
            let logged = log::take_recorded();
            assert_eq!(logged.len(), 2, "{logged:?}");
            assert!(logged[0].starts_with("Radmin VPN daemon failed to start - "));
            assert!(logged[1].starts_with("Radmin VPN GUI failed to start - "));
            assert!(calls(&paths).is_empty());

            fs::create_dir_all(app_dir(&paths)).unwrap();
            session.launch_gui().unwrap();
            assert!(session.gui.wait(Some(Duration::from_secs(20))));
            let code = session.gui.poll().and_then(|status| status.code());
            assert_eq!(code, Some(0));
            assert_eq!(
                calls(&paths),
                [
                    concat!(
                        "wine RvRvpnGui.exe",
                        "|~/data/rvpn/wineprefix/drive_c/Program Files (x86)/Radmin VPN|:7|unset"
                    ),
                    "1|llvmpipe",
                ]
            );
            assert_eq!(*seen.lock().unwrap(), ["update Running daemon"]);
        }

        #[test]
        fn staging_copies_helpers_fonts_and_patches_qt() {
            let dir = tempfile::tempdir().unwrap();
            let paths = paths(dir.path());
            let helpers = paths.rvpn_bin_dir();
            fs::create_dir_all(&helpers).unwrap();
            let names = [
                "rvpnnetmp.sys",
                "adapter_hook.dll",
                "rvpn_launcher.exe",
                "netsh.exe",
                "netsh64.exe",
                "drvinst.exe",
                "OpenSans-Regular.ttf",
                "OpenSans-Bold.ttf",
            ];
            for name in names {
                fs::write(helpers.join(name), format!("content of {name}")).unwrap();
            }
            let app = app_dir(&paths);
            let qt = app.join("platforms").join("qwindows.dll");
            fs::create_dir_all(qt.parent().unwrap()).unwrap();
            let users = prefix(&paths).join("drive_c").join("users");
            fs::create_dir_all(users.join("steamuser")).unwrap();
            symlink(dir.path(), users.join("steamuser").join("Desktop")).unwrap();
            let session = Session::new(&paths);
            let (reporter, seen) = recorder();

            let error = session.stage_artifacts(&reporter).unwrap_err();
            assert_eq!(error.to_string(), "Radmin VPN setup failed");

            let mut data = vec![0u8; QT_NAME_SITE + 32];
            data[QT_NAME_SITE..QT_NAME_SITE + 11].copy_from_slice(&QT_NAME_READ);
            data[QT_NAME_CAVE..QT_NAME_CAVE + 34].fill(0xcc);
            fs::write(&qt, data).unwrap();
            let windows = windows_dir(&paths);
            fs::create_dir_all(windows.join("system32")).unwrap();
            fs::write(windows.join("system32").join("netsh.exe"), "wine builtin").unwrap();
            session.stage_artifacts(&reporter).unwrap();
            session.stage_artifacts(&reporter).unwrap();
            assert_eq!(*seen.lock().unwrap(), ["update Installing components"; 3]);
            for (staged, name) in [
                (drivers_dir(&paths).join("rvpnnetmp.sys"), "rvpnnetmp.sys"),
                (app.join("adapter_hook.dll"), "adapter_hook.dll"),
                (app.join("rvpn_launcher.exe"), "rvpn_launcher.exe"),
                (windows.join("syswow64").join("netsh.exe"), "netsh.exe"),
                (windows.join("system32").join("netsh.exe"), "netsh64.exe"),
                (app.join("drvinst.exe"), "drvinst.exe"),
                (
                    windows.join("Fonts").join("OpenSans-Regular.ttf"),
                    "OpenSans-Regular.ttf",
                ),
                (
                    windows.join("Fonts").join("OpenSans-Bold.ttf"),
                    "OpenSans-Bold.ttf",
                ),
            ] {
                assert_eq!(
                    fs::read_to_string(&staged).unwrap(),
                    format!("content of {name}"),
                    "{}",
                    staged.display()
                );
            }
            let patched = fs::read(&qt).unwrap();
            assert_eq!(patched[QT_NAME_SITE..QT_NAME_SITE + 11], QT_NAME_JUMP);
            assert_eq!(patched[QT_NAME_CAVE..QT_NAME_CAVE + 34], QT_NAME_CHECK);
            assert!(!users.join("steamuser").join("Desktop").is_symlink());
        }

        #[test]
        fn device_setup_checks_mac_and_authorization_first() {
            let dir = tempfile::tempdir().unwrap();
            let session = Session::new(&paths(dir.path()));
            let index = dir.path().join("ifindex");
            for mac in ["", "02:1a:2b:3c:4d", "02:1a:2b:3c:4d:5e; reboot"] {
                assert_eq!(
                    session.net_bringup(mac, &index),
                    Err("invalid adapter MAC".to_string())
                );
            }
            assert_eq!(
                session.net_bringup(MAC, &index),
                Err("authorization was declined".to_string())
            );
            session.root.start(&mut Command::new("false")).unwrap();
            assert!(session.root.shell.wait(Some(Duration::from_secs(20))));
            assert_eq!(
                session.net_bringup(MAC, &index),
                Err("authorization was declined".to_string())
            );
            for ip in ["", "10.0.0.1", "26.1.2.3; reboot", "26.1.2"] {
                assert_eq!(
                    session.configure_routes(ip),
                    Err("invalid VPN IP".to_string())
                );
            }
            assert_eq!(
                session.configure_routes("26.1.2.3"),
                Err("authorization was declined".to_string())
            );
            session.root.close();
        }

        #[test]
        fn device_wait_ends_only_on_a_new_interface_index() {
            let dir = tempfile::tempdir().unwrap();
            let index = dir.path().join("ifindex");
            let received = dir.path().join("received");
            let sink = format!("exec cat > '{}'", received.display());
            let (reporter, seen) = recorder();

            let session = Session::new(&paths(dir.path()));
            session
                .root
                .start(Command::new("sh").args(["-c", &sink]))
                .unwrap();
            fs::write(&index, "7\n").unwrap();
            let created = Event::default();
            thread::scope(|scope| {
                let wait = scope.spawn(|| {
                    let device = session.create_device(MAC, &index, &reporter);
                    created.set();
                    device
                });
                assert!(!created.wait(Duration::from_millis(400)));
                fs::remove_file(&index).unwrap();
                assert!(!created.wait(Duration::from_millis(300)));
                fs::write(&index, "8\n").unwrap();
                assert!(wait.join().unwrap().unwrap());
            });
            session.root.close();
            assert_eq!(
                fs::read_to_string(&received).unwrap(),
                format!(
                    "{}\n__END__\n",
                    bringup_script(unsafe { libc::getuid() }, MAC)
                )
            );

            let session = Session::new(&paths(dir.path()));
            session
                .root
                .start(Command::new("sh").args(["-c", &sink]))
                .unwrap();
            thread::scope(|scope| {
                scope.spawn(|| {
                    thread::sleep(Duration::from_millis(100));
                    session.stop();
                });
                assert!(!session.create_device(MAC, &index, &reporter).unwrap());
            });
            session.root.close();

            let session = Session::new(&paths(dir.path()));
            session
                .root
                .start(Command::new("sh").args(["-c", "read line"]))
                .unwrap();
            let error = session.create_device(MAC, &index, &reporter).unwrap_err();
            assert_eq!(error.to_string(), "Network setup failed");
            assert_eq!(
                log::take_recorded(),
                ["Network setup failed - authorization was declined"]
            );
            session.root.close();

            let session = Session::new(&paths(dir.path()));
            session.stop();
            assert!(!session.create_device(MAC, &index, &reporter).unwrap());
            assert!(log::take_recorded().is_empty());
            assert_eq!(*seen.lock().unwrap(), ["update Creating network device"; 4]);
        }

        #[test]
        fn closing_the_window_ends_the_session_quietly() {
            let dir = tempfile::tempdir().unwrap();
            let session = Session::new(&paths(dir.path()));
            session.gui.hold(shell("exit 0"));
            session.bridge.hold(shell("read line"));
            session.service.hold(shell("exit 1"));
            assert!(session.service.wait(Some(Duration::from_secs(20))));
            assert!(session.gui.wait(Some(Duration::from_secs(20))));
            assert_eq!(session.wait_exit(), Ok(()));
            session.bridge.stop();
        }

        #[test]
        fn a_dead_helper_ends_the_session_with_its_exit_code() {
            let dir = tempfile::tempdir().unwrap();
            for (bridge, service, error) in [
                ("exit 3", "read line", "bridge exit=3"),
                ("read line", "kill -9 $$", "daemon signal: 9 (SIGKILL)"),
                ("exit 1", "exit 2", "bridge exit=1"),
            ] {
                let session = Session::new(&paths(dir.path()));
                session.gui.hold(shell("read line"));
                session.bridge.hold(shell(bridge));
                session.service.hold(shell(service));
                for (helper, script) in [(&session.bridge, bridge), (&session.service, service)] {
                    assert!(script == "read line" || helper.wait(Some(Duration::from_secs(20))));
                }
                assert_eq!(session.wait_exit(), Err(error.to_string()));
                assert!(session.gui.running());
                for helper in [&session.gui, &session.bridge, &session.service] {
                    helper.stop();
                }
            }
        }

        #[test]
        fn stop_cancels_the_wait_and_terminates_window_and_daemon() {
            let dir = tempfile::tempdir().unwrap();
            let session = Session::new(&paths(dir.path()));
            session
                .gui
                .hold(Command::new("sleep").arg("600").spawn().unwrap());
            session
                .bridge
                .hold(Command::new("sleep").arg("600").spawn().unwrap());
            session
                .service
                .hold(Command::new("sleep").arg("600").spawn().unwrap());
            thread::scope(|scope| {
                scope.spawn(|| {
                    thread::sleep(Duration::from_millis(100));
                    session.stop();
                });
                assert_eq!(session.wait_exit(), Ok(()));
            });
            assert!(session.cancel.is_set());
            assert!(session.gui.wait(Some(Duration::from_secs(20))));
            assert!(session.service.wait(Some(Duration::from_secs(20))));
            assert_eq!(signal(&session.gui), Some(libc::SIGTERM));
            assert_eq!(signal(&session.service), Some(libc::SIGTERM));
            assert!(session.bridge.running());
            session.bridge.stop();
            assert_eq!(session.wait_exit(), Ok(()));
        }
    }
}

#[cfg(not(target_os = "linux"))]
mod host {
    use std::io;
    use std::path::Path;

    use anyhow::Result;

    use super::Reporter;
    use crate::env::Paths;

    pub struct Session;

    impl Session {
        pub fn new(_paths: &Paths) -> Self {
            Self
        }

        pub fn run(
            &self,
            _installer: Option<&Path>,
            _reporter: &Reporter,
            _on_running: &dyn Fn(),
        ) -> Result<()> {
            unsupported()
        }

        pub fn stop(&self) {}
    }

    fn unsupported() -> Result<()> {
        Err(io::Error::from(io::ErrorKind::Unsupported).into())
    }

    pub fn preflight() -> Result<()> {
        unsupported()
    }

    pub fn ensure_wine(
        _paths: &Paths,
        _reporter: &Reporter,
        _cancelled: &dyn Fn() -> bool,
    ) -> Result<()> {
        unsupported()
    }

    pub fn uninstall(_paths: &Paths) -> Result<()> {
        unsupported()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn utf16(text: &str) -> Vec<u8> {
        text.encode_utf16().flat_map(u16::to_le_bytes).collect()
    }

    #[test]
    fn installer_needs_the_marker_and_a_sane_size() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("setup.exe");
        for (content, expected) in [
            (b"".to_vec(), false),
            (b"MZ...Radmin VPN...".to_vec(), true),
            (
                [b"MZ".to_vec(), utf16("Radmin VPN"), b"tail".to_vec()].concat(),
                true,
            ),
            ([b"MZ\0".to_vec(), utf16("xRadmin VPN")].concat(), true),
            (b"MZ...Radmin  VPN...radmin vpn".to_vec(), false),
            (utf16("Radmin VP"), false),
            (b"R\0a\0d\0m\0i\0n\0 \0V\0P\0N".to_vec(), false),
        ] {
            fs::write(&file, &content).unwrap();
            assert_eq!(is_radmin_installer(&file), expected, "{content:?}");
        }
        assert!(!is_radmin_installer(&dir.path().join("missing.exe")));
        assert!(!is_radmin_installer(dir.path()));

        fs::write(&file, b"Radmin VPN").unwrap();
        let handle = fs::File::options().write(true).open(&file).unwrap();
        handle.set_len(1 << 20).unwrap();
        assert!(is_radmin_installer(&file));
        handle.set_len(INSTALLER_LIMIT + 1).unwrap();
        assert!(!is_radmin_installer(&file));
    }

    #[test]
    fn installed_state_follows_the_service_binary() {
        let dir = tempfile::tempdir().unwrap();
        let paths = Paths::new(dir.path().join("asset"), dir.path().join("data"));
        assert!(!is_installed(&paths));
        let app = dir
            .path()
            .join("data")
            .join("rvpn")
            .join("wineprefix")
            .join("drive_c")
            .join("Program Files (x86)")
            .join("Radmin VPN");
        fs::create_dir_all(&app).unwrap();
        assert!(!is_installed(&paths));
        fs::write(app.join("RvControlSvc.exe"), "MZ").unwrap();
        assert!(is_installed(&paths));
    }

    #[test]
    fn service_version_comes_from_the_utf16_log() {
        let dir = tempfile::tempdir().unwrap();
        let paths = Paths::new(dir.path().join("asset"), dir.path().join("data"));
        assert_eq!(service_version(&paths), "");
        let log = dir
            .path()
            .join("data")
            .join("rvpn")
            .join("wineprefix")
            .join("drive_c")
            .join("ProgramData")
            .join("Famatech")
            .join("Radmin VPN")
            .join("service.log");
        fs::create_dir_all(log.parent().unwrap()).unwrap();
        for (text, version) in [
            (
                "\u{feff}start\r\nService version: 2.1.4951.1\r\nIP: 0.0.0.0\r\n",
                "2.1.4951.1",
            ),
            ("Service version:2.0.4899.9 x", "2.0.4899.9"),
            ("Service version:\t 1.4.4642.1\n", "1.4.4642.1"),
            ("Service version: 2.0\nService version: 2.1\n", "2.0"),
            ("x Service version: ...", "..."),
            ("service version: 2.1", ""),
            ("Service version: v2", ""),
            ("", ""),
        ] {
            fs::write(&log, utf16(text)).unwrap();
            assert_eq!(service_version(&paths), version, "{text:?}");
        }
        fs::write(&log, "Service version: 2.1.4951.1").unwrap();
        assert_eq!(service_version(&paths), "");
    }

    #[test]
    fn utf16_decoding_drops_broken_units() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("service.log");
        for (bytes, text) in [
            (&b"\x00\xd8\x41\x00"[..], "A"),
            (&b"\x00\xdc\x41\x00"[..], "A"),
            (&b"\x41\x00\x00\xd8"[..], "A"),
            (&b"\x41\x00\x42"[..], "A"),
            (
                &b"\xff\xfe\x41\x00\x3d\xd8\x00\xde\x42\x00"[..],
                "\u{feff}A\u{1f600}B",
            ),
            (&b"\x3d\xd8\x3d\xd8\x00\xde"[..], "\u{1f600}"),
            (&b""[..], ""),
        ] {
            fs::write(&file, bytes).unwrap();
            assert_eq!(read_utf16(&file), text, "{bytes:?}");
        }
        assert_eq!(read_utf16(&dir.path().join("missing.log")), "");
        assert_eq!(read_utf16(dir.path()), "");
    }
}
