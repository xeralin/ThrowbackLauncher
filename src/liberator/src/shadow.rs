use core::ffi::c_void;

use windows_sys::Win32::Foundation::{
    CloseHandle, GetLastError, ERROR_ALREADY_EXISTS, INVALID_HANDLE_VALUE, WAIT_OBJECT_0,
};
use windows_sys::Win32::System::Diagnostics::Debug::WriteProcessMemory;
use windows_sys::Win32::System::LibraryLoader::{GetModuleHandleA, GetProcAddress};
use windows_sys::Win32::System::Memory::{
    CreateFileMappingW, MapViewOfFile, UnmapViewOfFile, VirtualAllocEx, VirtualFreeEx,
    VirtualProtectEx, FILE_MAP_WRITE, MEM_COMMIT, MEM_RELEASE, MEM_RESERVE, PAGE_EXECUTE_READWRITE,
    PAGE_READONLY, PAGE_READWRITE,
};
use windows_sys::Win32::System::Threading::{
    CreateRemoteThread, GetExitCodeThread, WaitForSingleObject, LPTHREAD_START_ROUTINE,
};

use crate::tables::*;
use crate::win::Engine;

static SHADOW_DLL: &[u8] = include_bytes!("../shadow/Shadow.dll");

pub(crate) fn shadow_regions_for_build(build: &str) -> Option<&'static [ShadowRegion]> {
    for ms in SHADOW_SEASONS {
        if ms.build == build {
            return Some(ms.regions);
        }
    }
    None
}

impl Engine {
    pub(crate) fn get_text_section(&self) -> Option<(u64, u32)> {
        let mut dos = [0u8; 64];
        if self.read_mem(self.base, &mut dos) < 64 {
            return None;
        }
        let lfanew = u32::from_le_bytes([dos[60], dos[61], dos[62], dos[63]]) as u64;
        let mut hdr = [0u8; 24];
        if self.read_mem(self.base + lfanew, &mut hdr) < 24 {
            return None;
        }
        let num_sections = u16::from_le_bytes([hdr[6], hdr[7]]);
        let opt_size = u16::from_le_bytes([hdr[20], hdr[21]]) as u64;
        let sec_table = self.base + lfanew + 24 + opt_size;
        for i in 0..num_sections as u64 {
            let mut sec = [0u8; 40];
            if self.read_mem(sec_table + i * 40, &mut sec) < 40 {
                return None;
            }
            if &sec[0..5] == b".text" && sec[5] == 0 {
                let text_size = u32::from_le_bytes([sec[8], sec[9], sec[10], sec[11]]);
                let text_base =
                    self.base + u32::from_le_bytes([sec[12], sec[13], sec[14], sec[15]]) as u64;
                return Some((text_base, text_size));
            }
        }
        None
    }

    fn reserve_near(&self, near_addr: u64, size: u64) -> u64 {
        let base = near_addr & !0xFFFFu64;
        let mut d = 0x10000u64;
        while d < 0x40000000 {
            if base > d {
                let p = unsafe {
                    VirtualAllocEx(
                        self.proc,
                        (base - d) as usize as *const c_void,
                        size as usize,
                        MEM_RESERVE,
                        PAGE_EXECUTE_READWRITE,
                    )
                };
                if !p.is_null() {
                    return p as u64;
                }
            }
            let p2 = unsafe {
                VirtualAllocEx(
                    self.proc,
                    (base + d) as usize as *const c_void,
                    size as usize,
                    MEM_RESERVE,
                    PAGE_EXECUTE_READWRITE,
                )
            };
            if !p2.is_null() {
                return p2 as u64;
            }
            d += 0x10000;
        }
        0
    }

    fn commit_at(&self, addr: u64, size: u32) -> u64 {
        unsafe {
            VirtualAllocEx(
                self.proc,
                addr as usize as *const c_void,
                size as usize,
                MEM_COMMIT,
                PAGE_EXECUTE_READWRITE,
            ) as u64
        }
    }

    fn load_shadow(&self, path: &str) -> bool {
        let wpath: Vec<u16> = path.encode_utf16().chain(std::iter::once(0)).collect();
        let bytes = wpath.len() * 2;
        let rem = unsafe {
            VirtualAllocEx(
                self.proc,
                core::ptr::null(),
                bytes,
                MEM_COMMIT | MEM_RESERVE,
                PAGE_READWRITE,
            )
        };
        if rem.is_null() {
            return false;
        }
        let mut ok = false;
        let mut pending = false;
        let mut wr: usize = 0;
        let wrote = unsafe {
            WriteProcessMemory(
                self.proc,
                rem,
                wpath.as_ptr() as *const c_void,
                bytes,
                &mut wr,
            )
        };
        if wrote != 0 {
            let kernel = unsafe { GetModuleHandleA(b"kernel32.dll\0".as_ptr()) };
            let ll = unsafe { GetProcAddress(kernel, b"LoadLibraryW\0".as_ptr()) };
            let start: LPTHREAD_START_ROUTINE = unsafe { std::mem::transmute(ll) };
            let th = unsafe {
                CreateRemoteThread(
                    self.proc,
                    core::ptr::null(),
                    0,
                    start,
                    rem as *const c_void,
                    0,
                    core::ptr::null_mut(),
                )
            };
            if !th.is_null() {
                pending = unsafe { WaitForSingleObject(th, 10000) } != WAIT_OBJECT_0;
                if !pending {
                    let mut ret: u32 = 0;
                    unsafe { GetExitCodeThread(th, &mut ret) };
                    ok = ret != 0;
                }
                unsafe { CloseHandle(th) };
            }
        }
        if !pending {
            unsafe { VirtualFreeEx(self.proc, rem, 0, MEM_RELEASE) };
        }
        ok
    }

    fn shadow_dll_path(&self) -> Option<String> {
        let path = std::env::temp_dir().join(concat!("Shadow_", env!("CARGO_PKG_VERSION"), ".dll"));
        if !path.exists() {
            std::fs::write(&path, SHADOW_DLL).ok()?;
        }
        Some(path.to_string_lossy().into_owned())
    }

    pub(crate) fn shadow_arm_pages(&self) {
        for &pa in &self.shadow_pages {
            let mut old: u32 = 0;
            unsafe {
                VirtualProtectEx(
                    self.proc,
                    pa as usize as *const c_void,
                    0x1000,
                    PAGE_READONLY,
                    &mut old,
                )
            };
        }
    }

    pub(crate) fn shadow_write(&self, offset: u64, bytes: &[u8]) -> bool {
        if self.shadow_delta == 0 {
            return false;
        }
        let addr = self.base + offset;
        if !self.shadow_pages.contains(&(addr & !0xFFFu64)) {
            return false;
        }
        let mut buf = bytes.to_vec();
        let mod_hi = self.base + self.modsize as u64;
        let fix = crate::scanrip::scan_rip_rel(&buf, addr, self.base, mod_hi);
        crate::scanrip::apply_rip_delta(&mut buf, &fix, self.shadow_delta);
        self.write_mem((addr as i64 + self.shadow_delta) as u64, &buf)
    }

    fn release_and_fail(&self, shadow_base: u64) -> i32 {
        unsafe { VirtualFreeEx(self.proc, shadow_base as *mut c_void, 0, MEM_RELEASE) };
        1
    }

    pub(crate) fn shadow_run(&mut self, build: &str) -> i32 {
        let (text_base, text_size) = match self.get_text_section() {
            Some(x) => x,
            None => return 1,
        };
        let mod_hi = self.base + self.modsize as u64;
        let regions = match shadow_regions_for_build(build) {
            Some(r) if !r.is_empty() => r,
            _ => return 1,
        };

        let shadow_base = self.reserve_near(text_base, text_size as u64);
        let reserved_delta = shadow_base as i64 - text_base as i64;
        if shadow_base == 0 {
            return 1;
        }
        if !(-0x3FFFFFFF..=0x3FFFFFFF).contains(&reserved_delta) {
            return self.release_and_fail(shadow_base);
        }

        let mut windows: Vec<(u64, u64)> = Vec::new();
        for r in regions {
            let page = (self.base + r.offset) & !0xFFFu64;
            windows.push((page - 0x1000, page + 0x2000));
        }
        windows.sort_by_key(|a| a.0);
        let mut merged: Vec<(u64, u64)> = Vec::new();
        for w in windows {
            if let Some(last) = merged.last_mut() {
                if w.0 <= last.1 {
                    if w.1 > last.1 {
                        last.1 = w.1;
                    }
                    continue;
                }
            }
            merged.push(w);
        }

        let mut committed: Vec<(u64, u64)> = Vec::new();
        for (win_base, win_end) in merged {
            let win_size = (win_end - win_base) as usize;
            let mut buf = vec![0u8; win_size];
            let mut filled = 0usize;
            while filled < win_size {
                let got = self.read_mem(win_base + filled as u64, &mut buf[filled..]);
                if got == 0 {
                    break;
                }
                filled += got;
            }
            if filled != win_size {
                continue;
            }
            for r in regions {
                let addr = self.base + r.offset;
                if addr >= win_base && addr + r.patch.len() as u64 <= win_end {
                    let o = (addr - win_base) as usize;
                    buf[o..o + r.patch.len()].copy_from_slice(r.patch);
                }
            }
            let fix = crate::scanrip::scan_rip_rel(&buf, win_base, self.base, mod_hi);
            crate::scanrip::apply_rip_delta(&mut buf, &fix, reserved_delta);
            let copy_base = (win_base as i64 + reserved_delta) as u64;
            if self.commit_at(copy_base, win_size as u32) == 0 {
                continue;
            }
            let mut wr: usize = 0;
            let wrote = unsafe {
                WriteProcessMemory(
                    self.proc,
                    copy_base as usize as *const c_void,
                    buf.as_ptr() as *const c_void,
                    win_size,
                    &mut wr,
                )
            };
            if wrote == 0 || wr != win_size {
                continue;
            }
            committed.push((win_base, win_end));
        }
        if committed.is_empty() {
            return self.release_and_fail(shadow_base);
        }

        let mut payload = [0u8; 32];
        payload[0..4].copy_from_slice(&1u32.to_le_bytes());
        payload[4..8].copy_from_slice(&3u32.to_le_bytes());
        payload[8..16].copy_from_slice(&text_base.to_le_bytes());
        payload[16..24].copy_from_slice(&(text_size as u64).to_le_bytes());
        payload[24..32].copy_from_slice(&shadow_base.to_le_bytes());
        let name: Vec<u16> = "ShadowRegions"
            .encode_utf16()
            .chain(std::iter::once(0))
            .collect();
        let map = unsafe {
            CreateFileMappingW(
                INVALID_HANDLE_VALUE,
                core::ptr::null(),
                PAGE_READWRITE,
                0,
                payload.len() as u32,
                name.as_ptr(),
            )
        };
        if map.is_null() {
            return self.release_and_fail(shadow_base);
        }
        let existed = unsafe { GetLastError() } == ERROR_ALREADY_EXISTS;
        let view = unsafe { MapViewOfFile(map, FILE_MAP_WRITE, 0, 0, 0) };
        if view.Value.is_null() {
            unsafe { CloseHandle(map) };
            return self.release_and_fail(shadow_base);
        }
        let mut active_delta = reserved_delta;
        if existed {
            let mut old = [0u8; 32];
            unsafe {
                core::ptr::copy_nonoverlapping(view.Value as *const u8, old.as_mut_ptr(), old.len())
            };
            let old_text = u64::from_le_bytes(old[8..16].try_into().unwrap());
            let old_shadow = u64::from_le_bytes(old[24..32].try_into().unwrap());
            if old_text != text_base || old_shadow == 0 {
                unsafe { UnmapViewOfFile(view) };
                unsafe { CloseHandle(map) };
                return self.release_and_fail(shadow_base);
            }
            active_delta = old_shadow as i64 - old_text as i64;
        } else {
            unsafe {
                core::ptr::copy_nonoverlapping(
                    payload.as_ptr(),
                    view.Value as *mut u8,
                    payload.len(),
                )
            };
        }
        unsafe { UnmapViewOfFile(view) };
        let loaded = if existed {
            true
        } else {
            match self.shadow_dll_path() {
                Some(dll) => self.load_shadow(&dll),
                None => false,
            }
        };
        unsafe { CloseHandle(map) };
        if !loaded {
            return self.release_and_fail(shadow_base);
        }
        if existed {
            unsafe { VirtualFreeEx(self.proc, shadow_base as *mut c_void, 0, MEM_RELEASE) };
        }

        self.shadow_pages.clear();
        self.shadow_delta = active_delta;
        for r in regions {
            let pa = (self.base + r.offset) & !0xFFFu64;
            let covered = committed.iter().any(|&(b, e)| pa >= b && pa < e);
            if covered && !self.shadow_pages.contains(&pa) {
                self.shadow_pages.push(pa);
            }
        }
        0
    }
}
