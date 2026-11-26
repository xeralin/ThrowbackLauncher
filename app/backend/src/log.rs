use std::fmt::Display;
use std::fs;
use std::io::Write;
use std::path::PathBuf;
use std::process::ExitStatus;
#[cfg(not(test))]
use std::sync::Mutex;

use crate::env::VERSION;

struct Log {
    file: PathBuf,
    started: bool,
}

#[cfg(not(test))]
static LOG: Mutex<Option<Log>> = Mutex::new(None);

#[cfg(not(test))]
fn with_log<R>(run: impl FnOnce(&mut Option<Log>) -> R) -> R {
    run(&mut LOG.lock().unwrap_or_else(|poisoned| poisoned.into_inner()))
}

#[cfg(test)]
thread_local! {
    static LOG: std::cell::RefCell<Option<Log>> = const { std::cell::RefCell::new(None) };
    static RECORDED: std::cell::RefCell<Vec<String>> = const { std::cell::RefCell::new(Vec::new()) };
}

#[cfg(test)]
fn with_log<R>(run: impl FnOnce(&mut Option<Log>) -> R) -> R {
    LOG.with_borrow_mut(run)
}

#[cfg(test)]
pub fn take_recorded() -> Vec<String> {
    RECORDED.take()
}

pub fn exit_text(status: ExitStatus) -> String {
    let code = status.code();
    code.map_or_else(|| status.to_string(), |code| format!("exit={code}"))
}

pub fn init(errors_file: PathBuf) {
    with_log(|log| {
        *log = Some(Log {
            file: errors_file,
            started: false,
        })
    });
}

fn record(line: &str) {
    let stamp = chrono::Local::now().format("%Y-%m-%d %H:%M:%S");
    with_log(|log| write(log, &stamp.to_string(), line));
}

fn write(log: &mut Option<Log>, stamp: &str, line: &str) {
    let Some(log) = log.as_mut() else {
        return;
    };
    if let Some(parent) = log.file.parent() {
        let _ = fs::create_dir_all(parent);
    }
    let Ok(mut file) = fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&log.file)
    else {
        return;
    };
    let empty = file.metadata().is_ok_and(|metadata| metadata.len() == 0);
    if !log.started || empty {
        log.started = true;
        let _ = writeln!(file, "[{stamp}] v{VERSION}");
    }
    let _ = writeln!(file, "[{stamp}] {line}");
}

pub fn fail(message: &str, detail: impl Display) -> String {
    #[cfg(test)]
    RECORDED.with_borrow_mut(|lines| lines.push(format!("{message} - {detail}")));
    record(&format!("ERROR {message} - {detail}"));
    message.to_string()
}

pub fn install_panic_hook() {
    let previous = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let kind = if std::thread::current().name() == Some("main") {
            "CRASH"
        } else {
            "THREAD CRASH"
        };
        record(&format!("{kind}\n{info}"));
        previous(info);
    }));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn writes_a_version_header_once_and_returns_the_message() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("bin").join("errors.txt");
        init(file.clone());
        assert_eq!(
            fail("Settings save failed", "disk full"),
            "Settings save failed"
        );
        fail("Second", "detail");
        assert_eq!(
            take_recorded(),
            ["Settings save failed - disk full", "Second - detail"]
        );
        let text = fs::read_to_string(file).unwrap();
        let lines: Vec<&str> = text.lines().collect();
        assert_eq!(lines.len(), 3);
        assert!(lines[0].ends_with(&format!("] v{VERSION}")));
        assert!(lines[1].ends_with("] ERROR Settings save failed - disk full"));
        assert!(lines[2].ends_with("] ERROR Second - detail"));
        assert_eq!(&lines[0][..1], "[");
        assert_eq!(lines[0].find(']'), Some(20));
    }
}
