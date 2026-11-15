use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::thread;
use std::time::{Duration, Instant};

use crate::env::{EXE_NAME, IS_WINDOWS, PACKAGED, Paths, unlink};

const OUTCOME: &str = ".update-outcome";
pub(crate) const POLL_INTERVAL: Duration = Duration::from_millis(250);
const LEFTOVER_DEADLINE: Duration = Duration::from_secs(5);

pub fn write_outcome(paths: &Paths, ok: bool) {
    let _ = fs::write(paths.data_root.join(OUTCOME), if ok { "ok" } else { "" });
}

pub fn take_outcome(paths: &Paths) -> Option<bool> {
    let marker = paths.data_root.join(OUTCOME);
    let outcome = fs::read(&marker).ok()?;
    let _ = unlink(&marker);
    Some(outcome == b"ok")
}

pub fn staging_file(program: &Path) -> PathBuf {
    program.with_extension("new")
}

fn retired_file(program: &Path) -> PathBuf {
    program.with_extension("old")
}

pub fn replace_program(program: &Path, staged: &Path) -> io::Result<()> {
    if IS_WINDOWS {
        retire_and_replace(program, staged)
    } else {
        fs::rename(staged, program)
    }
}

fn retire_and_replace(program: &Path, staged: &Path) -> io::Result<()> {
    let retired = retired_file(program);
    let _ = fs::remove_file(&retired);
    let fresh = match fs::rename(program, &retired) {
        Err(error) if error.kind() == io::ErrorKind::NotFound => true,
        moved => moved.map(|()| false)?,
    };
    fs::rename(staged, program).inspect_err(|_| {
        if !fresh {
            let _ = fs::rename(&retired, program);
        }
    })
}

pub fn clear_leftovers(paths: &Paths) {
    if IS_WINDOWS && PACKAGED {
        let deadline = Instant::now() + LEFTOVER_DEADLINE;
        crate::migrate::remove_program_folders(paths, deadline);
        clear(&retired_file(&paths.asset_root.join(EXE_NAME)), deadline);
    }
}

pub(crate) fn clear(path: &Path, deadline: Instant) -> bool {
    loop {
        remove(path);
        if !path.exists() {
            return true;
        }
        if Instant::now() > deadline {
            return false;
        }
        thread::sleep(POLL_INTERVAL);
    }
}

fn remove(path: &Path) {
    if !path.is_dir() {
        let _ = unlink(path);
        return;
    }
    match unlink(&path.join(EXE_NAME)) {
        Err(error) if error.kind() != io::ErrorKind::NotFound => {}
        _ => {
            let _ = fs::remove_dir_all(path);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn outcome_is_taken_once() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().to_path_buf();
        let paths = Paths::new(root.clone(), root);
        assert_eq!(take_outcome(&paths), None);
        write_outcome(&paths, true);
        assert_eq!(take_outcome(&paths), Some(true));
        assert_eq!(take_outcome(&paths), None);
        write_outcome(&paths, false);
        assert_eq!(take_outcome(&paths), Some(false));
    }

    #[test]
    fn a_running_program_is_retired_and_replaced() {
        let dir = tempfile::tempdir().unwrap();
        let program = dir.path().join(EXE_NAME);
        let staged = staging_file(&program);
        fs::write(&program, "old").unwrap();
        fs::write(&staged, "new").unwrap();
        fs::write(retired_file(&program), "older").unwrap();
        retire_and_replace(&program, &staged).unwrap();
        assert_eq!(fs::read_to_string(&program).unwrap(), "new");
        assert_eq!(fs::read_to_string(retired_file(&program)).unwrap(), "old");
        assert!(!staged.exists());
    }

    #[test]
    fn a_missing_program_is_put_in_place() {
        let dir = tempfile::tempdir().unwrap();
        let program = dir.path().join(EXE_NAME);
        let staged = staging_file(&program);
        fs::write(&staged, "new").unwrap();
        retire_and_replace(&program, &staged).unwrap();
        assert_eq!(fs::read_to_string(&program).unwrap(), "new");
        assert!(!retired_file(&program).exists());
        assert!(!staged.exists());
    }

    #[test]
    fn the_program_stays_when_the_staged_file_is_missing() {
        let dir = tempfile::tempdir().unwrap();
        let program = dir.path().join(EXE_NAME);
        fs::write(&program, "old").unwrap();
        assert!(retire_and_replace(&program, &staging_file(&program)).is_err());
        assert_eq!(fs::read_to_string(&program).unwrap(), "old");
        assert!(!retired_file(&program).exists());
    }

    #[test]
    fn leftovers_go_unless_their_program_cannot_be_deleted() {
        let dir = tempfile::tempdir().unwrap();
        let previous = dir.path().join("app.previous");
        fs::create_dir_all(previous.join("lib")).unwrap();
        fs::write(previous.join("lib").join("python.dll"), "x").unwrap();
        let program = previous.join(EXE_NAME);
        fs::create_dir(&program).unwrap();
        fs::write(program.join("busy"), "x").unwrap();
        remove(&previous);
        assert!(previous.join("lib").join("python.dll").is_file());

        fs::remove_dir_all(&program).unwrap();
        assert!(clear(&previous, Instant::now()));
        assert!(!previous.exists());
        assert!(clear(&previous, Instant::now()));
    }
}
