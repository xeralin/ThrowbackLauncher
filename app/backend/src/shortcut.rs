use std::ffi::OsString;
use std::os::windows::ffi::OsStringExt;
use std::path::{Path, PathBuf};

use windows::Win32::System::Com::{
    CLSCTX_INPROC_SERVER, COINIT_APARTMENTTHREADED, CoCreateInstance, CoInitializeEx,
    CoUninitialize, IPersistFile, STGM_READ,
};
use windows::Win32::UI::Shell::{IShellLinkW, ShellLink};
use windows::core::{HSTRING, Interface, Result};

const PATH_CAPACITY: usize = 32_768;

struct Com(bool);

impl Com {
    fn init() -> Self {
        Self(unsafe { CoInitializeEx(None, COINIT_APARTMENTTHREADED) }.is_ok())
    }
}

impl Drop for Com {
    fn drop(&mut self) {
        if self.0 {
            unsafe { CoUninitialize() };
        }
    }
}

fn link() -> Result<IShellLinkW> {
    unsafe { CoCreateInstance(&ShellLink, None, CLSCTX_INPROC_SERVER) }
}

fn point(shell: &IShellLinkW, program: &Path, folder: &Path) -> Result<()> {
    let program = HSTRING::from(program);
    unsafe {
        shell.SetPath(&program)?;
        shell.SetWorkingDirectory(&HSTRING::from(folder))?;
        shell.SetIconLocation(&program, 0)
    }
}

fn target(shell: &IShellLinkW) -> Result<PathBuf> {
    let mut buffer = vec![0u16; PATH_CAPACITY];
    unsafe { shell.GetPath(&mut buffer, std::ptr::null_mut(), 0) }?;
    let end = buffer
        .iter()
        .position(|&unit| unit == 0)
        .unwrap_or(buffer.len());
    Ok(OsString::from_wide(&buffer[..end]).into())
}

pub(crate) fn write(file: &Path, program: &Path, folder: &Path) -> Result<()> {
    let _com = Com::init();
    let shell = link()?;
    point(&shell, program, folder)?;
    unsafe {
        shell
            .cast::<IPersistFile>()?
            .Save(&HSTRING::from(file), true)
    }
}

pub(crate) fn repoint(
    files: &[PathBuf],
    stale: impl Fn(&Path) -> bool,
    program: &Path,
    folder: &Path,
) {
    let _com = Com::init();
    let update = |file: &Path| -> Result<()> {
        let shell = link()?;
        let store = shell.cast::<IPersistFile>()?;
        unsafe { store.Load(&HSTRING::from(file), STGM_READ) }?;
        if stale(&target(&shell)?) {
            point(&shell, program, folder)?;
            unsafe { store.Save(&HSTRING::from(file), true) }?;
        }
        Ok(())
    };
    for file in files {
        let _ = update(file);
    }
}
