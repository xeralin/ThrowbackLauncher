use std::fs;
use std::path::PathBuf;

use anyhow::Result;

use crate::env::{LIBERATOR_NAME, Paths, VERSION, remove_if_present};
use crate::github::{Progress, download_error, fetch_to, github_asset};
use crate::pe::file_version;

fn release_url() -> String {
    format!("https://api.github.com/repos/xeralin/ThrowbackLauncher/releases/tags/v{VERSION}")
}

pub fn liberator_ready(paths: &Paths) -> bool {
    file_version(&paths.liberator_bin()).is_some_and(|version| version == VERSION)
}

#[cfg(any(windows, test))]
pub(crate) fn adopt(paths: &Paths, folder: &std::path::Path) -> std::io::Result<()> {
    let shipped = folder.join(LIBERATOR_NAME);
    if !shipped.is_file() {
        return Ok(());
    }
    fs::create_dir_all(paths.bin_dir())?;
    fs::rename(shipped, paths.liberator_bin())
}

pub fn ensure_liberator(paths: &Paths, progress: Progress, force: bool) -> Result<PathBuf> {
    let bin = paths.liberator_bin();
    if force || !liberator_ready(paths) {
        let fetch = || -> Result<()> {
            let (_, url) = github_asset(paths, &release_url(), LIBERATOR_NAME)?;
            fs::create_dir_all(paths.bin_dir())?;
            fetch_to(&url, &bin, progress, None)?;
            Ok(remove_if_present(&bin.with_extension("version"))?)
        };
        fetch().map_err(|error| download_error("Liberator download failed", error))?;
    }
    Ok(bin)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::github::testing::{Reply, Server, seed};
    use crate::log::take_recorded;
    use crate::pe::versioned_exe;

    fn paths() -> (tempfile::TempDir, Paths) {
        let dir = tempfile::tempdir().unwrap();
        let paths = Paths::new(dir.path().join("asset"), dir.path().join("data"));
        (dir, paths)
    }

    #[test]
    fn the_file_of_this_version_is_fetched_once() {
        let (_dir, paths) = paths();
        let server = Server::start(|_| Reply::new(200, versioned_exe(VERSION)));
        seed(
            &paths,
            &release_url(),
            &[(LIBERATOR_NAME, &server.url("/file"))],
        );
        assert!(!liberator_ready(&paths));

        let bin = ensure_liberator(&paths, None, false).unwrap();
        assert_eq!(bin, paths.bin_dir().join("Liberator.exe"));
        assert_eq!(fs::read(&bin).unwrap(), versioned_exe(VERSION));
        assert!(liberator_ready(&paths));
        ensure_liberator(&paths, None, false).unwrap();
        assert_eq!(server.paths(), ["/file"]);

        ensure_liberator(&paths, None, true).unwrap();
        assert_eq!(server.paths().len(), 2);
        assert!(take_recorded().is_empty());
    }

    #[test]
    fn a_file_shipped_beside_the_program_is_taken_over() {
        let (_dir, paths) = paths();
        let folder = paths.data_root.join("app.pending");
        adopt(&paths, &folder).unwrap();
        assert!(!paths.bin_dir().exists());

        fs::create_dir_all(&folder).unwrap();
        fs::write(folder.join(LIBERATOR_NAME), versioned_exe(VERSION)).unwrap();
        fs::create_dir_all(paths.bin_dir()).unwrap();
        fs::write(paths.liberator_bin(), "old").unwrap();
        adopt(&paths, &folder).unwrap();
        assert_eq!(
            fs::read(paths.liberator_bin()).unwrap(),
            versioned_exe(VERSION)
        );
        assert!(!folder.join(LIBERATOR_NAME).exists());
        assert!(liberator_ready(&paths));
    }

    #[test]
    fn a_file_of_another_version_is_replaced_and_the_old_version_file_goes() {
        let (_dir, paths) = paths();
        let server = Server::start(|_| Reply::new(200, versioned_exe(VERSION)));
        seed(
            &paths,
            &release_url(),
            &[(LIBERATOR_NAME, &server.url("/file"))],
        );
        let leftover = paths.liberator_bin().with_extension("version");
        fs::write(paths.liberator_bin(), versioned_exe("0.0.1")).unwrap();
        fs::write(&leftover, VERSION).unwrap();
        assert!(!liberator_ready(&paths));

        ensure_liberator(&paths, None, false).unwrap();
        assert_eq!(
            fs::read(paths.liberator_bin()).unwrap(),
            versioned_exe(VERSION)
        );
        assert!(liberator_ready(&paths));
        assert!(!leftover.exists());
    }

    #[test]
    fn a_failed_download_keeps_the_old_file_and_names_the_failure() {
        let (_dir, paths) = paths();
        let server = Server::start(|_| Reply::new(404, ""));
        seed(
            &paths,
            &release_url(),
            &[(LIBERATOR_NAME, &server.url("/gone"))],
        );
        fs::write(paths.liberator_bin(), "old").unwrap();

        let error = ensure_liberator(&paths, None, false).unwrap_err();
        assert_eq!(error.to_string(), "Liberator download failed");
        assert_eq!(take_recorded().len(), 1);
        assert_eq!(fs::read_to_string(paths.liberator_bin()).unwrap(), "old");
        assert!(!liberator_ready(&paths));
    }
}
