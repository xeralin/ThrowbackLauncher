use std::fs::{self, File};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::thread;
use std::time::{Duration, Instant};

const POLL: Duration = Duration::from_millis(100);

pub struct Lock {
    file: Option<File>,
    path: PathBuf,
}

enum Claim {
    Held(File),
    Busy,
    Failed,
}

impl Lock {
    pub fn acquire(path: &Path, wait: Duration) -> Option<Self> {
        let deadline = Instant::now() + wait;
        loop {
            match platform::claim(path) {
                Claim::Held(file) => {
                    return Some(Self {
                        file: Some(file),
                        path: path.to_path_buf(),
                    });
                }
                Claim::Failed => return None,
                Claim::Busy => {}
            }
            let left = deadline.saturating_duration_since(Instant::now());
            if left.is_zero() {
                return None;
            }
            thread::sleep(POLL.min(left));
        }
    }
}

impl Drop for Lock {
    fn drop(&mut self) {
        if cfg!(windows) {
            self.file.take();
        }
        let _ = fs::remove_file(&self.path);
        self.file.take();
    }
}

fn record(mut file: File, path: &Path) -> Claim {
    let contents = format!(
        "{}\n{}\n{}\n\n{}\n",
        std::process::id(),
        platform::process_name(),
        platform::host_name(),
        platform::boot_id()
    );
    let written = file
        .set_len(0)
        .and_then(|()| file.write_all(contents.as_bytes()));
    if written.is_err() {
        let _ = fs::remove_file(path);
        return Claim::Failed;
    }
    let _ = file.sync_data();
    Claim::Held(file)
}

pub struct Listener {
    #[cfg(unix)]
    path: PathBuf,
}

impl Listener {
    pub fn start(key: &str, on_wake: impl Fn() + Send + 'static) -> io::Result<Self> {
        platform::listen(&platform::endpoint(key), on_wake)
    }
}

pub fn wake(key: &str) {
    platform::wake(&platform::endpoint(key));
}

#[cfg(unix)]
mod platform {
    use std::fs::{self, OpenOptions, TryLockError};
    use std::io;
    use std::os::unix::fs::MetadataExt;
    use std::os::unix::net::{UnixListener, UnixStream};
    use std::path::{Path, PathBuf};
    use std::thread;

    use super::{Claim, Listener, record};

    pub fn claim(path: &Path) -> Claim {
        loop {
            let opened = OpenOptions::new()
                .read(true)
                .write(true)
                .create(true)
                .truncate(false)
                .open(path);
            let Ok(file) = opened else {
                return Claim::Failed;
            };
            match file.try_lock() {
                Ok(()) => {}
                Err(TryLockError::WouldBlock) => return Claim::Busy,
                Err(TryLockError::Error(_)) => return Claim::Failed,
            }
            let named = fs::metadata(path).ok();
            let held = file.metadata().ok();
            let current = held.zip(named).is_some_and(|(held, named)| {
                held.dev() == named.dev() && held.ino() == named.ino()
            });
            if current {
                return record(file, path);
            }
        }
    }

    pub fn process_name() -> String {
        let exe = std::env::current_exe().unwrap_or_default();
        let name = exe.file_name().unwrap_or_default().to_string_lossy();
        name.strip_suffix(" (deleted)").unwrap_or(&name).to_owned()
    }

    pub fn host_name() -> String {
        first_line("/proc/sys/kernel/hostname")
    }

    pub fn boot_id() -> String {
        first_line("/proc/sys/kernel/random/boot_id")
    }

    fn first_line(file: &str) -> String {
        let text = fs::read_to_string(file).unwrap_or_default();
        text.lines().next().unwrap_or_default().to_owned()
    }

    pub fn endpoint(key: &str) -> PathBuf {
        std::env::temp_dir().join(key)
    }

    pub fn listen(path: &Path, on_wake: impl Fn() + Send + 'static) -> io::Result<Listener> {
        let _ = fs::remove_file(path);
        let socket = UnixListener::bind(path)?;
        thread::spawn(move || {
            while socket.accept().is_ok() {
                on_wake();
            }
        });
        Ok(Listener {
            path: path.to_path_buf(),
        })
    }

    pub fn wake(path: &Path) {
        let _ = UnixStream::connect(path);
    }

    impl Drop for Listener {
        fn drop(&mut self) {
            let _ = fs::remove_file(&self.path);
        }
    }
}

#[cfg(windows)]
mod platform {
    use std::fs::OpenOptions;
    use std::io;
    use std::os::windows::fs::OpenOptionsExt;
    use std::os::windows::io::{AsRawHandle, HandleOrInvalid, OwnedHandle};
    use std::path::Path;
    use std::ptr;
    use std::thread;

    use windows_sys::Win32::Foundation::{
        ERROR_ACCESS_DENIED, ERROR_NO_DATA, ERROR_PIPE_CONNECTED, ERROR_SHARING_VIOLATION,
    };
    use windows_sys::Win32::Storage::FileSystem::{FILE_SHARE_READ, PIPE_ACCESS_DUPLEX};
    use windows_sys::Win32::System::Pipes::{
        ConnectNamedPipe, CreateNamedPipeW, PIPE_READMODE_BYTE, PIPE_REJECT_REMOTE_CLIENTS,
        PIPE_TYPE_BYTE, PIPE_UNLIMITED_INSTANCES, PIPE_WAIT,
    };
    use windows_sys::Win32::UI::WindowsAndMessaging::{ASFW_ANY, AllowSetForegroundWindow};

    use super::{Claim, Listener, record};

    pub fn claim(path: &Path) -> Claim {
        let opened = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(true)
            .share_mode(FILE_SHARE_READ)
            .open(path);
        match opened {
            Ok(file) => record(file, path),
            Err(error) => match error.raw_os_error().map(|code| code as u32) {
                Some(ERROR_SHARING_VIOLATION) => Claim::Busy,
                Some(ERROR_ACCESS_DENIED) if path.exists() => Claim::Busy,
                _ => Claim::Failed,
            },
        }
    }

    pub fn process_name() -> String {
        let exe = std::env::current_exe().unwrap_or_default();
        let name = exe.file_stem().unwrap_or_default();
        name.to_string_lossy().into_owned()
    }

    pub fn host_name() -> String {
        std::env::var("COMPUTERNAME").unwrap_or_default()
    }

    pub fn boot_id() -> String {
        String::new()
    }

    pub fn endpoint(key: &str) -> String {
        format!(r"\\.\pipe\{key}")
    }

    fn pipe(name: &[u16]) -> io::Result<OwnedHandle> {
        let handle = unsafe {
            HandleOrInvalid::from_raw_handle(CreateNamedPipeW(
                name.as_ptr(),
                PIPE_ACCESS_DUPLEX,
                PIPE_TYPE_BYTE | PIPE_READMODE_BYTE | PIPE_WAIT | PIPE_REJECT_REMOTE_CLIENTS,
                PIPE_UNLIMITED_INSTANCES,
                0,
                0,
                0,
                ptr::null(),
            ))
        };
        OwnedHandle::try_from(handle).map_err(|_| io::Error::last_os_error())
    }

    pub fn listen(name: &str, on_wake: impl Fn() + Send + 'static) -> io::Result<Listener> {
        let name: Vec<u16> = name.encode_utf16().chain([0]).collect();
        let mut current = pipe(&name)?;
        thread::spawn(move || {
            loop {
                let joined = unsafe { ConnectNamedPipe(current.as_raw_handle(), ptr::null_mut()) }
                    != 0
                    || matches!(
                        io::Error::last_os_error()
                            .raw_os_error()
                            .map(|code| code as u32),
                        Some(ERROR_PIPE_CONNECTED | ERROR_NO_DATA)
                    );
                if !joined {
                    return;
                }
                let Ok(following) = pipe(&name) else {
                    return;
                };
                current = following;
                on_wake();
            }
        });
        Ok(Listener {})
    }

    pub fn wake(name: &str) {
        unsafe { AllowSetForegroundWindow(ASFW_ANY) };
        let _ = OpenOptions::new().read(true).write(true).open(name);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const PATIENCE: Duration = Duration::from_secs(5);

    fn lines(path: &Path) -> Vec<String> {
        let text = fs::read_to_string(path).unwrap();
        text.split('\n').map(str::to_owned).collect()
    }

    #[test]
    fn a_lock_is_exclusive_until_it_is_dropped() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join(".lock");
        let lock = Lock::acquire(&path, Duration::ZERO).unwrap();
        assert!(Lock::acquire(&path, Duration::ZERO).is_none());
        drop(lock);
        assert!(!path.exists());
        assert!(Lock::acquire(&path, Duration::ZERO).is_some());
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn the_lock_file_names_its_holder_the_way_qt_reads_it() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join(".lock");
        let _lock = Lock::acquire(&path, Duration::ZERO).unwrap();
        let exe = std::env::current_exe().unwrap();
        let host = fs::read_to_string("/proc/sys/kernel/hostname").unwrap();
        let boot = fs::read_to_string("/proc/sys/kernel/random/boot_id").unwrap();
        assert_eq!(
            lines(&path),
            [
                std::process::id().to_string(),
                exe.file_name().unwrap().to_string_lossy().into_owned(),
                host.trim_end().to_owned(),
                String::new(),
                boot.trim_end().to_owned(),
                String::new(),
            ]
        );
        assert_eq!(boot.trim_end().len(), 36);
    }

    #[test]
    fn a_file_left_behind_by_a_dead_holder_is_taken_over() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join(".lock");
        fs::write(&path, "4194999\nThrowbackLauncher\nelsewhere\n\n\n").unwrap();
        let _lock = Lock::acquire(&path, Duration::ZERO).unwrap();
        assert_eq!(lines(&path)[0], std::process::id().to_string());
    }

    #[test]
    fn acquire_waits_for_the_holder_to_leave() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join(".lock");
        let lock = Lock::acquire(&path, Duration::ZERO).unwrap();
        let holder = thread::spawn(move || {
            thread::sleep(Duration::from_millis(150));
            drop(lock);
        });
        let started = Instant::now();
        assert!(Lock::acquire(&path, PATIENCE).is_some());
        assert!(started.elapsed() < Duration::from_secs(2));
        holder.join().unwrap();
    }

    #[test]
    fn acquire_gives_up_at_the_deadline() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join(".lock");
        let _lock = Lock::acquire(&path, Duration::ZERO).unwrap();
        let started = Instant::now();
        assert!(Lock::acquire(&path, Duration::from_millis(250)).is_none());
        let waited = started.elapsed();
        assert!(waited >= Duration::from_millis(250), "{waited:?}");
        assert!(waited < Duration::from_secs(2), "{waited:?}");
    }

    #[test]
    fn a_place_that_cannot_hold_a_file_fails_without_waiting() {
        let root = tempfile::tempdir().unwrap();
        let blocker = root.path().join("file");
        fs::write(&blocker, "").unwrap();
        let started = Instant::now();
        assert!(Lock::acquire(&blocker.join(".lock"), PATIENCE).is_none());
        assert!(started.elapsed() < Duration::from_secs(1));
    }

    #[cfg(unix)]
    mod sockets {
        use std::sync::mpsc;

        use super::*;

        #[test]
        fn a_second_start_wakes_the_listener() {
            let root = tempfile::tempdir().unwrap();
            let path = root.path().join("ThrowbackLauncher-test");
            fs::write(&path, "stale").unwrap();
            let (sender, woken) = mpsc::channel();
            let listener = platform::listen(&path, move || {
                let _ = sender.send(());
            })
            .unwrap();
            platform::wake(&path);
            platform::wake(&path);
            woken.recv_timeout(PATIENCE).unwrap();
            woken.recv_timeout(PATIENCE).unwrap();
            drop(listener);
            assert!(!path.exists());
            platform::wake(&path);
            assert!(woken.recv_timeout(Duration::from_millis(100)).is_err());
        }

        #[test]
        fn the_endpoint_sits_in_the_temp_folder_under_the_instance_key() {
            assert_eq!(
                platform::endpoint("ThrowbackLauncher-033a01f1"),
                std::env::temp_dir().join("ThrowbackLauncher-033a01f1")
            );
        }
    }
}
