use std::hash::{BuildHasher, RandomState};
use std::io::{Read, Write};
use std::time::Duration;

use anyhow::{Context, Result, bail};
use serde::Serialize;
use serde_json::{Map, Value};

use crate::log;

use ipc::{Conn, open_candidates, readable};

const OP_HANDSHAKE: u32 = 0;
const OP_FRAME: u32 = 1;
const OP_CLOSE: u32 = 2;
const OP_PING: u32 = 3;
const OP_PONG: u32 = 4;

const IPC_TIMEOUT: Duration = Duration::from_secs(10);

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Activity {
    status_display_type: u8,
    details: String,
    timestamps: Timestamps,
    buttons: [Button; 1],
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
struct Timestamps {
    start: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
struct Button {
    label: &'static str,
    url: &'static str,
}

#[derive(Serialize)]
struct Handshake<'a> {
    v: u8,
    client_id: &'a str,
}

#[derive(Serialize)]
struct Command<'a> {
    cmd: &'static str,
    args: Args<'a>,
    nonce: &'a str,
}

#[derive(Serialize)]
struct Args<'a> {
    pid: u32,
    activity: &'a Activity,
}

fn ipc_names() -> impl Iterator<Item = String> {
    (0..10).map(|slot| format!("discord-ipc-{slot}"))
}

fn encode(op: u32, payload: &impl Serialize) -> Result<Vec<u8>> {
    let body = serde_json::to_vec(payload)?;
    let mut frame = op.to_le_bytes().to_vec();
    frame.extend(u32::try_from(body.len())?.to_le_bytes());
    frame.extend(body);
    Ok(frame)
}

fn send(conn: &mut impl Write, op: u32, payload: &impl Serialize) -> Result<()> {
    conn.write_all(&encode(op, payload)?)?;
    Ok(())
}

fn recv(conn: &mut impl Read) -> Result<(u32, Map<String, Value>)> {
    let mut header = [[0; 4]; 2];
    conn.read_exact(header.as_flattened_mut())?;
    let [op, length] = header.map(u32::from_le_bytes);
    let mut body = Vec::new();
    if conn.by_ref().take(length.into()).read_to_end(&mut body)? < length as usize {
        bail!("socket closed");
    }
    serde_json::from_slice(&body)
        .map(|payload| (op, payload))
        .context("malformed frame")
}

fn greet(conn: &mut Conn, client_id: &str) -> Result<u32> {
    send(conn, OP_HANDSHAKE, &Handshake { v: 1, client_id })?;
    Ok(recv(conn)?.0)
}

fn answer_pings(conn: &mut Conn) -> Result<()> {
    while readable(conn)? {
        let (op, payload) = recv(conn)?;
        if op == OP_PING {
            send(conn, OP_PONG, &payload)?;
        }
    }
    Ok(())
}

fn nonce() -> String {
    format!("{:016x}", RandomState::new().hash_one(()))
}

pub struct Presence {
    client_id: String,
    conn: Option<Conn>,
}

impl Presence {
    pub fn new(client_id: &str) -> Self {
        Self {
            client_id: client_id.to_owned(),
            conn: None,
        }
    }

    pub fn client_id(&self) -> &str {
        &self.client_id
    }

    pub fn connected(&self) -> bool {
        self.conn.is_some()
    }

    pub fn connect(&mut self) -> bool {
        self.attach(open_candidates())
    }

    fn attach(&mut self, candidates: impl Iterator<Item = Conn>) -> bool {
        for mut conn in candidates {
            let greeting = greet(&mut conn, &self.client_id);
            self.conn = Some(conn);
            if matches!(greeting, Ok(OP_FRAME)) {
                return true;
            }
            self.close();
        }
        false
    }

    pub fn set(&mut self, activity: &Activity) -> bool {
        match self.request(activity) {
            Ok(rejection) => {
                if let Some(detail) = rejection {
                    log::fail("Discord rejected the activity", detail);
                }
                true
            }
            Err(_) => {
                self.close();
                false
            }
        }
    }

    fn request(&mut self, activity: &Activity) -> Result<Option<Value>> {
        let conn = self.conn.as_mut().context("not connected")?;
        let nonce = nonce();
        let command = Command {
            cmd: "SET_ACTIVITY",
            args: Args {
                pid: std::process::id(),
                activity,
            },
            nonce: &nonce,
        };
        send(conn, OP_FRAME, &command)?;
        loop {
            let (op, mut payload) = recv(conn)?;
            if op == OP_PING {
                send(conn, OP_PONG, &payload)?;
            } else if op == OP_FRAME
                && payload.get("nonce").and_then(Value::as_str) == Some(nonce.as_str())
            {
                let rejected = payload.get("evt").and_then(Value::as_str) == Some("ERROR");
                return Ok(rejected.then(|| payload.remove("data").unwrap_or_default()));
            }
        }
    }

    pub fn drain(&mut self) {
        if self
            .conn
            .as_mut()
            .is_some_and(|conn| answer_pings(conn).is_err())
        {
            self.close();
        }
    }

    pub fn close(&mut self) {
        if let Some(mut conn) = self.conn.take() {
            let _ = send(&mut conn, OP_CLOSE, &Map::new());
        }
    }
}

pub fn build_activity(label: &str, is_hm: bool, start: i64) -> (&'static str, Activity) {
    let activity = Activity {
        status_display_type: 2,
        details: label.to_owned(),
        timestamps: Timestamps { start },
        buttons: [Button {
            label: "Play",
            url: "https://xeralin.github.io/ThrowbackFAQ/getting-started/",
        }],
    };
    let client_id = if is_hm {
        "1544119720662278215"
    } else {
        "1544119665394192525"
    };
    (client_id, activity)
}

#[cfg(unix)]
mod ipc {
    use std::collections::HashSet;
    use std::ffi::OsString;
    use std::fs;
    use std::io;
    use std::iter;
    use std::os::fd::{AsRawFd, FromRawFd};
    use std::os::unix::ffi::OsStrExt;
    use std::os::unix::net::UnixStream;
    use std::path::{Path, PathBuf};

    use super::{IPC_TIMEOUT, ipc_names};

    pub type Conn = UnixStream;

    pub fn open_candidates() -> impl Iterator<Item = Conn> {
        socket_paths(bases(std::env::var_os)).filter_map(|path| open(&path).ok())
    }

    pub fn readable(conn: &Conn) -> io::Result<bool> {
        let mut slot = libc::pollfd {
            fd: conn.as_raw_fd(),
            events: libc::POLLIN,
            revents: 0,
        };
        loop {
            let ready = unsafe { libc::poll(&raw mut slot, 1, 0) };
            if ready != -1 {
                return Ok(ready > 0);
            }
            let error = io::Error::last_os_error();
            if error.kind() != io::ErrorKind::Interrupted {
                return Err(error);
            }
        }
    }

    pub(super) fn bases(variable: impl Fn(&'static str) -> Option<OsString>) -> Vec<PathBuf> {
        ["XDG_RUNTIME_DIR", "TMPDIR", "TMP", "TEMP"]
            .into_iter()
            .filter_map(variable)
            .filter(|value| !value.is_empty())
            .map(PathBuf::from)
            .chain([PathBuf::from("/tmp")])
            .collect()
    }

    pub(super) fn socket_paths(bases: Vec<PathBuf>) -> impl Iterator<Item = PathBuf> {
        let mut seen = HashSet::new();
        bases
            .into_iter()
            .flat_map(|root| {
                let apps = children(&root.join("app"));
                let sandboxes = children(&root.join(".flatpak"));
                iter::once(root)
                    .chain(apps)
                    .chain(sandboxes.into_iter().map(|sandbox| sandbox.join("xdg-run")))
            })
            .flat_map(|folder| ipc_names().map(move |name| folder.join(name)))
            .filter(move |path| path.exists() && seen.insert(path.clone()))
    }

    fn children(folder: &Path) -> Vec<PathBuf> {
        let mut paths: Vec<PathBuf> = fs::read_dir(folder)
            .into_iter()
            .flatten()
            .flatten()
            .map(|entry| entry.path())
            .collect();
        paths.sort();
        paths
    }

    pub(super) fn open(path: &Path) -> io::Result<Conn> {
        let name = path.as_os_str().as_bytes();
        let mut address = libc::sockaddr_un {
            sun_family: libc::AF_UNIX as libc::sa_family_t,
            sun_path: [0; _],
        };
        if name.len() >= address.sun_path.len() {
            return Err(io::ErrorKind::InvalidInput.into());
        }
        for (slot, byte) in address.sun_path.iter_mut().zip(name) {
            *slot = *byte as libc::c_char;
        }
        let socket = unsafe {
            libc::socket(
                libc::AF_UNIX,
                libc::SOCK_STREAM | libc::SOCK_CLOEXEC | libc::SOCK_NONBLOCK,
                0,
            )
        };
        if socket == -1 {
            return Err(io::Error::last_os_error());
        }
        let conn = unsafe { UnixStream::from_raw_fd(socket) };
        let size = size_of_val(&address) as libc::socklen_t;
        if unsafe { libc::connect(socket, (&raw const address).cast(), size) } == -1 {
            return Err(io::Error::last_os_error());
        }
        conn.set_nonblocking(false)?;
        conn.set_read_timeout(Some(IPC_TIMEOUT))?;
        conn.set_write_timeout(Some(IPC_TIMEOUT))?;
        Ok(conn)
    }
}

#[cfg(windows)]
mod ipc {
    use std::io::{self, Read, Write};
    use std::os::windows::io::{AsRawHandle, HandleOrInvalid, HandleOrNull, OwnedHandle};
    use std::ptr;

    use windows_sys::Win32::Foundation::{ERROR_IO_PENDING, GENERIC_READ, GENERIC_WRITE, HANDLE};
    use windows_sys::Win32::Storage::FileSystem::{
        CreateFileW, FILE_FLAG_OVERLAPPED, OPEN_EXISTING, ReadFile, WriteFile,
    };
    use windows_sys::Win32::System::IO::{
        CancelIoEx, GetOverlappedResult, GetOverlappedResultEx, OVERLAPPED,
    };
    use windows_sys::Win32::System::Pipes::PeekNamedPipe;
    use windows_sys::Win32::System::Threading::CreateEventW;
    use windows_sys::core::BOOL;

    use super::{IPC_TIMEOUT, ipc_names};

    pub struct Conn {
        pipe: OwnedHandle,
        event: Option<OwnedHandle>,
    }

    impl Conn {
        fn open(name: &str) -> Option<Self> {
            let path: Vec<u16> = format!(r"\\.\pipe\{name}")
                .encode_utf16()
                .chain([0])
                .collect();
            let pipe = unsafe {
                HandleOrInvalid::from_raw_handle(CreateFileW(
                    path.as_ptr(),
                    GENERIC_READ | GENERIC_WRITE,
                    0,
                    ptr::null(),
                    OPEN_EXISTING,
                    FILE_FLAG_OVERLAPPED,
                    ptr::null_mut(),
                ))
            };
            let pipe = OwnedHandle::try_from(pipe).ok()?;
            let event = unsafe {
                HandleOrNull::from_raw_handle(CreateEventW(ptr::null(), 1, 0, ptr::null()))
            };
            Some(Self {
                pipe,
                event: OwnedHandle::try_from(event).ok(),
            })
        }

        fn transfer(
            &self,
            start: impl FnOnce(HANDLE, *mut OVERLAPPED) -> BOOL,
        ) -> io::Result<usize> {
            let pipe = self.pipe.as_raw_handle();
            let mut overlapped = OVERLAPPED {
                hEvent: self
                    .event
                    .as_ref()
                    .map_or(ptr::null_mut(), AsRawHandle::as_raw_handle),
                ..Default::default()
            };
            let overlapped = &raw mut overlapped;
            if start(pipe, overlapped) == 0 {
                let error = io::Error::last_os_error();
                if error.raw_os_error() != Some(ERROR_IO_PENDING as i32) {
                    return Err(error);
                }
            }
            let mut count = 0;
            let timeout = IPC_TIMEOUT.as_millis() as u32;
            if unsafe { GetOverlappedResultEx(pipe, overlapped, &mut count, timeout, 0) } != 0 {
                return Ok(count as usize);
            }
            let error = io::Error::last_os_error();
            unsafe {
                CancelIoEx(pipe, overlapped);
                GetOverlappedResult(pipe, overlapped, &mut count, 1);
            }
            Err(error)
        }
    }

    impl Read for Conn {
        fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
            let length = u32::try_from(buffer.len()).unwrap_or(u32::MAX);
            self.transfer(|pipe, overlapped| unsafe {
                ReadFile(
                    pipe,
                    buffer.as_mut_ptr(),
                    length,
                    ptr::null_mut(),
                    overlapped,
                )
            })
        }
    }

    impl Write for Conn {
        fn write(&mut self, data: &[u8]) -> io::Result<usize> {
            let length = u32::try_from(data.len()).unwrap_or(u32::MAX);
            self.transfer(|pipe, overlapped| unsafe {
                WriteFile(pipe, data.as_ptr(), length, ptr::null_mut(), overlapped)
            })
        }

        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    pub fn open_candidates() -> impl Iterator<Item = Conn> {
        ipc_names().filter_map(|name| Conn::open(&name))
    }

    pub fn readable(conn: &Conn) -> io::Result<bool> {
        let mut available = 0;
        let peeked = unsafe {
            PeekNamedPipe(
                conn.pipe.as_raw_handle(),
                ptr::null_mut(),
                0,
                ptr::null_mut(),
                &mut available,
                ptr::null_mut(),
            )
        };
        if peeked == 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(available != 0)
    }
}

#[cfg(test)]
mod tests {
    use std::io;

    use super::*;

    const PLAY_URL: &str = "https://xeralin.github.io/ThrowbackFAQ/getting-started/";

    struct Trickle<'a>(&'a [u8]);

    impl Read for Trickle<'_> {
        fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
            let count = self.0.len().min(buffer.len()).min(1);
            buffer[..count].copy_from_slice(&self.0[..count]);
            self.0 = &self.0[count..];
            Ok(count)
        }
    }

    fn raw(op: u32, body: &[u8]) -> Vec<u8> {
        let mut frame = op.to_le_bytes().to_vec();
        frame.extend((body.len() as u32).to_le_bytes());
        frame.extend(body);
        frame
    }

    fn hex(bytes: &[u8]) -> String {
        bytes.iter().map(|byte| format!("{byte:02x}")).collect()
    }

    fn body(frame: &[u8]) -> &str {
        std::str::from_utf8(&frame[8..]).unwrap()
    }

    #[test]
    fn frames_carry_opcode_length_and_compact_json() {
        let handshake = Handshake {
            v: 1,
            client_id: "1544119665394192525",
        };
        assert_eq!(
            encode(OP_HANDSHAKE, &handshake).unwrap(),
            raw(
                OP_HANDSHAKE,
                br#"{"v":1,"client_id":"1544119665394192525"}"#
            )
        );
        assert_eq!(
            hex(&encode(OP_CLOSE, &Map::new()).unwrap()),
            "02000000020000007b7d"
        );
    }

    #[test]
    fn set_activity_frame_wraps_the_activity_with_pid_and_nonce() {
        let (client_id, activity) = build_activity("Y9S2 New Blood", true, 1794528000);
        assert_eq!(client_id, "1544119720662278215");
        let command = Command {
            cmd: "SET_ACTIVITY",
            args: Args {
                pid: 4242,
                activity: &activity,
            },
            nonce: "0f8fad5b-d9cb-469f-a165-70867728950e",
        };
        assert_eq!(
            body(&encode(OP_FRAME, &command).unwrap()),
            format!(
                r#"{{"cmd":"SET_ACTIVITY","args":{{"pid":4242,"activity":{{"status_display_type":2,"details":"Y9S2 New Blood","timestamps":{{"start":1794528000}},"buttons":[{{"label":"Play","url":"{PLAY_URL}"}}]}}}},"nonce":"0f8fad5b-d9cb-469f-a165-70867728950e"}}"#
            )
        );
    }

    #[test]
    fn build_activity_keeps_the_exact_field_set() {
        let (client_id, activity) = build_activity("Y5S3 Shadow Legacy", false, 1700000000);
        assert_eq!(client_id, "1544119665394192525");
        assert_eq!(
            body(&encode(OP_FRAME, &activity).unwrap()),
            format!(
                r#"{{"status_display_type":2,"details":"Y5S3 Shadow Legacy","timestamps":{{"start":1700000000}},"buttons":[{{"label":"Play","url":"{PLAY_URL}"}}]}}"#
            )
        );
        assert_eq!(
            activity,
            build_activity("Y5S3 Shadow Legacy", true, 1700000000).1
        );
        assert_ne!(
            activity,
            build_activity("Y5S3 Shadow Legacy", false, 1700000001).1
        );
    }

    #[test]
    fn recv_reads_frames_across_partial_reads() {
        let mut stream = raw(OP_FRAME, br#"{"cmd":"DISPATCH","evt":"READY"}"#);
        stream.extend(raw(7, b" {\"a\": 1, \"a\": 2} "));
        let mut reader = Trickle(&stream);
        let (op, payload) = recv(&mut reader).unwrap();
        assert_eq!(op, OP_FRAME);
        assert_eq!(payload["evt"], "READY");
        let (op, payload) = recv(&mut reader).unwrap();
        assert_eq!(op, 7);
        assert_eq!(Value::Object(payload), serde_json::json!({"a": 2}));
        assert!(recv(&mut reader).is_err());
    }

    #[test]
    fn recv_rejects_bodies_that_are_not_json_objects() {
        for malformed in [
            &b""[..],
            b"[]",
            b"\"x\"",
            b"1",
            b"null",
            b"true",
            b"{",
            b"\xff",
            b"{\"a\":1}{\"b\":2}",
        ] {
            let frame = raw(OP_FRAME, malformed);
            let error = recv(&mut frame.as_slice()).unwrap_err();
            assert_eq!(error.to_string(), "malformed frame", "{malformed:?}");
        }
        let frame = raw(OP_FRAME, b"{}");
        assert_eq!(recv(&mut frame.as_slice()).unwrap(), (OP_FRAME, Map::new()));
    }

    #[test]
    fn recv_fails_when_the_stream_ends_inside_a_frame() {
        let frame = raw(OP_FRAME, b"{\"a\": 1}");
        assert!(recv(&mut &frame[..0]).is_err());
        assert!(recv(&mut &frame[..7]).is_err());
        let error = recv(&mut &frame[..frame.len() - 1]).unwrap_err();
        assert_eq!(error.to_string(), "socket closed");
        let mut endless = OP_FRAME.to_le_bytes().to_vec();
        endless.extend(u32::MAX.to_le_bytes());
        endless.extend(b"{\"a\"");
        let error = recv(&mut endless.as_slice()).unwrap_err();
        assert_eq!(error.to_string(), "socket closed");
    }

    #[test]
    fn nonces_differ() {
        let nonces: std::collections::HashSet<String> = (0..200).map(|_| nonce()).collect();
        assert_eq!(nonces.len(), 200);
    }

    #[test]
    fn a_fresh_presence_is_idle() {
        let mut presence = Presence::new("1544119665394192525");
        assert_eq!(presence.client_id(), "1544119665394192525");
        assert!(!presence.connected());
        assert!(!presence.attach(std::iter::empty()));
        assert!(!presence.set(&build_activity("Y1S0 Vanilla", false, 1).1));
        presence.drain();
        presence.close();
        assert!(!presence.connected());
    }

    #[test]
    fn presence_can_move_to_the_worker_thread() {
        fn movable<T: Send>() {}
        movable::<Presence>();
        movable::<Activity>();
    }

    #[cfg(unix)]
    mod sockets {
        use std::ffi::OsString;
        use std::fs;
        use std::os::fd::AsRawFd;
        use std::os::unix::fs::symlink;
        use std::os::unix::net::{UnixListener, UnixStream};
        use std::path::{Path, PathBuf};
        use std::ptr;
        use std::sync::atomic::{AtomicBool, Ordering};
        use std::thread;
        use std::time::Instant;

        use super::super::ipc::{bases, open, socket_paths};
        use super::*;

        const PATIENCE: Duration = Duration::from_secs(5);
        const INTERRUPTED_PROBES: u32 = 16;

        fn touch(path: PathBuf) {
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(path, b"").unwrap();
        }

        fn read_frame(stream: &mut UnixStream) -> (u32, String) {
            let mut header = [[0; 4]; 2];
            stream.read_exact(header.as_flattened_mut()).unwrap();
            let [op, length] = header.map(u32::from_le_bytes);
            let mut body = vec![0; length as usize];
            stream.read_exact(&mut body).unwrap();
            (op, String::from_utf8(body).unwrap())
        }

        fn write_frame(stream: &mut UnixStream, op: u32, body: &str) {
            stream.write_all(&raw(op, body.as_bytes())).unwrap();
        }

        fn hung_up(stream: &mut UnixStream) -> bool {
            stream.read(&mut [0; 1]).unwrap() == 0
        }

        fn nonce_of(command: &str) -> String {
            let command: Value = serde_json::from_str(command).unwrap();
            command["nonce"].as_str().unwrap().to_owned()
        }

        fn linked(client_id: &str) -> (Presence, UnixStream) {
            let (client, server) = UnixStream::pair().unwrap();
            server.set_read_timeout(Some(PATIENCE)).unwrap();
            let presence = Presence {
                client_id: client_id.to_owned(),
                conn: Some(client),
            };
            (presence, server)
        }

        fn accept(listener: &UnixListener) -> UnixStream {
            let (stream, _) = listener.accept().unwrap();
            stream.set_read_timeout(Some(PATIENCE)).unwrap();
            stream
        }

        #[test]
        fn bases_follow_the_environment_order_and_end_with_tmp() {
            let all = |name: &'static str| Some(OsString::from(format!("/{name}")));
            assert_eq!(
                bases(all),
                [
                    Path::new("/XDG_RUNTIME_DIR"),
                    Path::new("/TMPDIR"),
                    Path::new("/TMP"),
                    Path::new("/TEMP"),
                    Path::new("/tmp")
                ]
            );
            let some = |name: &'static str| match name {
                "XDG_RUNTIME_DIR" => Some(OsString::from("/run/user/1000")),
                "TMPDIR" => Some(OsString::new()),
                "TEMP" => Some(OsString::from("/var/tmp/")),
                _ => None,
            };
            assert_eq!(
                bases(some),
                [
                    Path::new("/run/user/1000"),
                    Path::new("/var/tmp/"),
                    Path::new("/tmp")
                ]
            );
            assert_eq!(bases(|_| None), [Path::new("/tmp")]);
        }

        #[test]
        fn socket_paths_follow_the_search_order_of_discord() {
            let dir = tempfile::tempdir().unwrap();
            let run = dir.path().join("run");
            let other = dir.path().join("other");
            for path in [
                "discord-ipc-3",
                "discord-ipc-0",
                "discord-ipc-10",
                "app/com.discordapp.Discord/discord-ipc-0",
                "app/dev.vencord.Vesktop/discord-ipc-1",
                "app/.hidden/discord-ipc-2",
                "app/a-b/discord-ipc-0",
                "app/a/discord-ipc-0",
                "app/B/discord-ipc-0",
                "app/afile",
                "app/discord-ipc-5",
                ".flatpak/dev.vencord.Vesktop/xdg-run/discord-ipc-0",
                ".flatpak/com.discordapp.Discord/xdg-run/discord-ipc-9",
                ".flatpak/x-y/xdg-run/discord-ipc-4",
                ".flatpak/x/xdg-run/discord-ipc-4",
                ".flatpak/nope/other/discord-ipc-0",
                ".flatpak/plainfile",
                ".flatpak/deep/xdg-run/nested/discord-ipc-0",
            ] {
                touch(run.join(path));
            }
            touch(other.join("discord-ipc-7"));
            symlink(run.join("app/a"), run.join("app/link")).unwrap();
            symlink(dir.path().join("missing"), run.join("app/broken")).unwrap();
            symlink(
                dir.path().join("missing/discord-ipc-6"),
                run.join("discord-ipc-6"),
            )
            .unwrap();
            let bases = vec![
                PathBuf::from(format!("{}/", run.display())),
                run.clone(),
                PathBuf::from(format!("{}//", other.display())),
                dir.path().join("absent"),
            ];
            let found: Vec<PathBuf> = socket_paths(bases).collect();
            let expected: Vec<PathBuf> = [
                "run/discord-ipc-0",
                "run/discord-ipc-3",
                "run/app/.hidden/discord-ipc-2",
                "run/app/B/discord-ipc-0",
                "run/app/a/discord-ipc-0",
                "run/app/a-b/discord-ipc-0",
                "run/app/com.discordapp.Discord/discord-ipc-0",
                "run/app/dev.vencord.Vesktop/discord-ipc-1",
                "run/app/link/discord-ipc-0",
                "run/.flatpak/com.discordapp.Discord/xdg-run/discord-ipc-9",
                "run/.flatpak/dev.vencord.Vesktop/xdg-run/discord-ipc-0",
                "run/.flatpak/x/xdg-run/discord-ipc-4",
                "run/.flatpak/x-y/xdg-run/discord-ipc-4",
                "other/discord-ipc-7",
            ]
            .into_iter()
            .map(|path| dir.path().join(path))
            .collect();
            assert_eq!(found, expected);
        }

        #[test]
        fn socket_paths_look_at_each_folder_only_when_reached() {
            let dir = tempfile::tempdir().unwrap();
            let first = dir.path().join("first");
            let second = dir.path().join("second");
            touch(first.join("discord-ipc-0"));
            fs::create_dir(&second).unwrap();
            let mut paths = socket_paths(vec![first.clone(), second.clone()]);
            assert_eq!(paths.next(), Some(first.join("discord-ipc-0")));
            touch(first.join("discord-ipc-4"));
            touch(second.join("app/late/discord-ipc-1"));
            assert_eq!(paths.next(), Some(first.join("discord-ipc-4")));
            assert_eq!(paths.next(), Some(second.join("app/late/discord-ipc-1")));
            assert_eq!(paths.next(), None);
        }

        #[test]
        fn open_connects_in_blocking_mode_with_the_ipc_timeout() {
            let dir = tempfile::tempdir().unwrap();
            let path = dir.path().join("discord-ipc-0");
            let _listener = UnixListener::bind(&path).unwrap();
            let conn = open(&path).unwrap();
            assert_eq!(conn.read_timeout().unwrap(), Some(Duration::from_secs(10)));
            assert_eq!(conn.write_timeout().unwrap(), Some(Duration::from_secs(10)));
            let status = unsafe { libc::fcntl(conn.as_raw_fd(), libc::F_GETFL) };
            assert_eq!(status & libc::O_NONBLOCK, 0);
            let descriptor = unsafe { libc::fcntl(conn.as_raw_fd(), libc::F_GETFD) };
            assert_eq!(descriptor & libc::FD_CLOEXEC, libc::FD_CLOEXEC);
        }

        #[test]
        fn open_fails_for_paths_without_a_listener() {
            let dir = tempfile::tempdir().unwrap();
            let stale = dir.path().join("discord-ipc-0");
            drop(UnixListener::bind(&stale).unwrap());
            let file = dir.path().join("discord-ipc-1");
            fs::write(&file, b"").unwrap();
            assert!(stale.exists());
            assert!(open(&stale).is_err());
            assert!(open(&file).is_err());
            assert!(open(&dir.path().join("discord-ipc-2")).is_err());
            assert!(open(&dir.path().join("x".repeat(120))).is_err());
        }

        #[test]
        fn open_skips_a_full_backlog_without_waiting() {
            let dir = tempfile::tempdir().unwrap();
            let path = dir.path().join("discord-ipc-0");
            let listener = UnixListener::bind(&path).unwrap();
            assert_eq!(unsafe { libc::listen(listener.as_raw_fd(), 0) }, 0);
            let _queued = open(&path).unwrap();
            assert_eq!(open(&path).unwrap_err().kind(), io::ErrorKind::WouldBlock);
        }

        #[test]
        fn readable_reports_pending_data_and_hangup() {
            let (mut client, mut server) = UnixStream::pair().unwrap();
            assert!(!readable(&client).unwrap());
            server.write_all(b"x").unwrap();
            assert!(readable(&client).unwrap());
            client.read_exact(&mut [0; 1]).unwrap();
            assert!(!readable(&client).unwrap());
            drop(server);
            assert!(readable(&client).unwrap());
        }

        #[test]
        fn readable_polls_again_after_a_signal() {
            extern "C" fn ignore(_: libc::c_int) {}
            let handler = ignore as extern "C" fn(libc::c_int) as libc::sighandler_t;
            assert_ne!(
                unsafe { libc::signal(libc::SIGUSR1, handler) },
                libc::SIG_ERR
            );
            let (client, _server) = UnixStream::pair().unwrap();
            let polling = unsafe { libc::pthread_self() };
            let done = AtomicBool::new(false);
            let started = Instant::now();
            let failure = thread::scope(|scope| {
                scope.spawn(|| {
                    while !done.load(Ordering::Relaxed) {
                        unsafe { libc::pthread_kill(polling, libc::SIGUSR1) };
                    }
                });
                let mut interrupted_probes = 0;
                let mut failure = None;
                while failure.is_none()
                    && interrupted_probes < INTERRUPTED_PROBES
                    && started.elapsed() < PATIENCE
                {
                    if unsafe { libc::poll(ptr::null_mut(), 0, 0) } == -1 {
                        interrupted_probes += 1;
                    }
                    failure =
                        Some(readable(&client)).filter(|pending| !matches!(pending, Ok(false)));
                }
                done.store(true, Ordering::Relaxed);
                failure
            });
            assert!(failure.is_none(), "{failure:?}");
        }

        #[test]
        fn connect_walks_the_candidates_until_one_greets_with_a_frame() {
            let dir = tempfile::tempdir().unwrap();
            drop(UnixListener::bind(dir.path().join("discord-ipc-0")).unwrap());
            touch(dir.path().join("discord-ipc-1"));
            let refusing = UnixListener::bind(dir.path().join("discord-ipc-2")).unwrap();
            let silent = UnixListener::bind(dir.path().join("discord-ipc-3")).unwrap();
            let greeting = UnixListener::bind(dir.path().join("discord-ipc-4")).unwrap();
            let unused = UnixListener::bind(dir.path().join("discord-ipc-5")).unwrap();
            unused.set_nonblocking(true).unwrap();
            let discord = thread::spawn(move || {
                let mut first = accept(&refusing);
                let refused = read_frame(&mut first);
                write_frame(
                    &mut first,
                    OP_CLOSE,
                    r#"{"code":4000,"message":"Invalid Client ID"}"#,
                );
                let goodbye = read_frame(&mut first);
                let first_hung_up = hung_up(&mut first);
                let mut second = accept(&silent);
                let ignored = read_frame(&mut second);
                drop(second);
                let mut third = accept(&greeting);
                let greeted = read_frame(&mut third);
                write_frame(&mut third, OP_FRAME, r#"{"cmd":"DISPATCH","evt":"READY"}"#);
                (refused, goodbye, first_hung_up, ignored, greeted, third)
            });
            let mut presence = Presence::new("1544119665394192525");
            let candidates =
                socket_paths(vec![dir.path().to_path_buf()]).filter_map(|path| open(&path).ok());
            assert!(presence.attach(candidates));
            assert!(presence.connected());
            let (refused, goodbye, first_hung_up, ignored, greeted, _third) =
                discord.join().unwrap();
            let handshake = (
                OP_HANDSHAKE,
                r#"{"v":1,"client_id":"1544119665394192525"}"#.to_owned(),
            );
            assert_eq!(refused, handshake);
            assert_eq!(goodbye, (OP_CLOSE, "{}".to_owned()));
            assert!(first_hung_up);
            assert_eq!(ignored, handshake);
            assert_eq!(greeted, handshake);
            assert_eq!(
                unused.accept().unwrap_err().kind(),
                io::ErrorKind::WouldBlock
            );
        }

        #[test]
        fn connect_gives_up_when_no_candidate_greets() {
            let (client, mut server) = UnixStream::pair().unwrap();
            write_frame(&mut server, OP_PING, "{}");
            let mut presence = Presence::new("1");
            assert!(!presence.attach([client].into_iter()));
            assert!(!presence.connected());
            assert_eq!(
                read_frame(&mut server),
                (OP_HANDSHAKE, r#"{"v":1,"client_id":"1"}"#.to_owned())
            );
            assert_eq!(read_frame(&mut server), (OP_CLOSE, "{}".to_owned()));
            assert!(hung_up(&mut server));
        }

        #[test]
        fn set_sends_the_activity_and_waits_for_its_nonce() {
            let (mut presence, mut server) = linked("1544119720662278215");
            let (_, activity) = build_activity("Y9S2 New Blood", true, 1794528000);
            let discord = thread::spawn(move || {
                let (op, command) = read_frame(&mut server);
                let nonce = nonce_of(&command);
                write_frame(&mut server, OP_PING, r#"{"z":1,"a":[true,null]}"#);
                let pong = read_frame(&mut server);
                write_frame(
                    &mut server,
                    OP_FRAME,
                    r#"{"cmd":"DISPATCH","evt":"ERROR","nonce":null}"#,
                );
                write_frame(&mut server, OP_CLOSE, &format!(r#"{{"nonce":"{nonce}"}}"#));
                write_frame(
                    &mut server,
                    OP_FRAME,
                    &format!(
                        r#"{{"cmd":"SET_ACTIVITY","data":{{}},"evt":null,"nonce":"{nonce}"}}"#
                    ),
                );
                (op, command, nonce, pong, server)
            });
            assert!(presence.set(&activity));
            let (op, command, nonce, pong, _server) = discord.join().unwrap();
            assert_eq!(op, OP_FRAME);
            assert_eq!(
                command,
                format!(
                    r#"{{"cmd":"SET_ACTIVITY","args":{{"pid":{},"activity":{{"status_display_type":2,"details":"Y9S2 New Blood","timestamps":{{"start":1794528000}},"buttons":[{{"label":"Play","url":"{PLAY_URL}"}}]}}}},"nonce":"{nonce}"}}"#,
                    std::process::id()
                )
            );
            assert_eq!(pong, (OP_PONG, r#"{"z":1,"a":[true,null]}"#.to_owned()));
            assert!(!readable(presence.conn.as_ref().unwrap()).unwrap());
            assert!(presence.connected());
        }

        #[test]
        fn a_rejected_activity_is_logged_once_and_counts_as_sent() {
            let (mut presence, mut server) = linked("1");
            let (_, activity) = build_activity("Y1S0 Vanilla", false, 1);
            let discord = thread::spawn(move || {
                for data in [
                    r#""data":{"code":4000,"message":"child \"activity\" fails"},"#,
                    "",
                ] {
                    let nonce = nonce_of(&read_frame(&mut server).1);
                    write_frame(
                        &mut server,
                        OP_FRAME,
                        &format!(
                            r#"{{"cmd":"SET_ACTIVITY",{data}"evt":"ERROR","nonce":"{nonce}"}}"#
                        ),
                    );
                }
                server
            });
            assert!(presence.set(&activity));
            assert!(presence.set(&activity));
            assert!(presence.connected());
            assert_eq!(
                crate::log::take_recorded(),
                [
                    r#"Discord rejected the activity - {"code":4000,"message":"child \"activity\" fails"}"#,
                    "Discord rejected the activity - null"
                ]
            );
            discord.join().unwrap();
        }

        #[test]
        fn set_gives_up_and_closes_when_discord_stays_silent() {
            let (mut presence, mut server) = linked("1");
            let pause = Duration::from_millis(50);
            let conn = presence.conn.as_ref().unwrap();
            conn.set_read_timeout(Some(pause)).unwrap();
            let started = Instant::now();
            assert!(!presence.set(&build_activity("Y1S0 Vanilla", false, 1).1));
            assert!(started.elapsed() >= pause / 2);
            assert!(!presence.connected());
            assert_eq!(read_frame(&mut server).0, OP_FRAME);
            assert_eq!(read_frame(&mut server), (OP_CLOSE, "{}".to_owned()));
            assert!(hung_up(&mut server));
        }

        #[test]
        fn set_fails_and_closes_after_a_hangup() {
            let (mut presence, server) = linked("1");
            drop(server);
            assert!(!presence.set(&build_activity("Y1S0 Vanilla", false, 1).1));
            assert!(!presence.connected());
        }

        #[test]
        fn drain_answers_pings_and_drops_other_frames() {
            let (mut presence, mut server) = linked("1");
            presence.drain();
            assert!(presence.connected());
            write_frame(&mut server, OP_FRAME, r#"{"cmd":"DISPATCH","evt":"READY"}"#);
            write_frame(&mut server, OP_PING, r#"{"seq":7}"#);
            write_frame(&mut server, OP_CLOSE, "{}");
            presence.drain();
            assert!(presence.connected());
            assert_eq!(
                read_frame(&mut server),
                (OP_PONG, r#"{"seq":7}"#.to_owned())
            );
            server.set_nonblocking(true).unwrap();
            assert_eq!(
                server.read(&mut [0; 1]).unwrap_err().kind(),
                io::ErrorKind::WouldBlock
            );
        }

        #[test]
        fn a_ping_is_opcode_3_and_its_answer_opcode_4_on_the_wire() {
            let (mut presence, mut server) = linked("1");
            server
                .write_all(b"\x03\0\0\0\x09\0\0\0{\"seq\":7}")
                .unwrap();
            presence.drain();
            let mut pong = [0; 17];
            server.read_exact(&mut pong).unwrap();
            assert_eq!(hex(&pong[..8]), "0400000009000000");
            assert_eq!(body(&pong), r#"{"seq":7}"#);
        }

        #[test]
        fn drain_closes_after_a_hangup_or_a_malformed_frame() {
            let (mut presence, server) = linked("1");
            drop(server);
            presence.drain();
            assert!(!presence.connected());

            let (mut presence, mut server) = linked("1");
            write_frame(&mut server, OP_FRAME, "[]");
            presence.drain();
            assert!(!presence.connected());
            assert_eq!(read_frame(&mut server), (OP_CLOSE, "{}".to_owned()));
            assert!(hung_up(&mut server));
        }

        #[test]
        fn close_sends_the_close_frame_and_hangs_up() {
            let (mut presence, mut server) = linked("1");
            presence.close();
            presence.close();
            assert!(!presence.connected());
            let mut sent = Vec::new();
            server.read_to_end(&mut sent).unwrap();
            assert_eq!(hex(&sent), "02000000020000007b7d");
        }
    }
}
