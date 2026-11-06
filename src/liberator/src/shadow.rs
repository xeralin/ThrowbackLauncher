use core::ffi::c_void;
use std::os::windows::ffi::OsStrExt;
use std::path::{Path, PathBuf};

use windows_sys::Win32::Foundation::{
    CloseHandle, GetLastError, ERROR_ALREADY_EXISTS, INVALID_HANDLE_VALUE, WAIT_OBJECT_0,
};
use windows_sys::Win32::System::LibraryLoader::{GetModuleHandleA, GetProcAddress};
use windows_sys::Win32::System::Memory::{
    CreateFileMappingW, MapViewOfFile, OpenFileMappingW, UnmapViewOfFile, VirtualAllocEx,
    VirtualFreeEx, VirtualProtectEx, FILE_MAP_READ, FILE_MAP_WRITE, MEM_COMMIT, MEM_RELEASE,
    MEM_RESERVE, PAGE_EXECUTE_READWRITE, PAGE_READONLY, PAGE_READWRITE,
};
use windows_sys::Win32::System::Threading::{
    CreateRemoteThread, GetExitCodeThread, WaitForSingleObject, LPTHREAD_START_ROUTINE,
};
use windows_sys::{s, w};

use crate::runtime::{Engine, WARMUP_MODE};
use crate::tables::*;

static SHADOW_DLL: &[u8] = include_bytes!("../shadow/Shadow.dll");

pub(crate) fn shadow_regions_for_build(build: &str) -> Option<&'static [ShadowRegion]> {
    SHADOW_SEASONS
        .iter()
        .find(|s| s.build == build)
        .map(|s| s.regions)
}

pub(crate) fn playlist_sync(build: &str) -> Option<&'static PlaylistSync> {
    PLAYLIST_SYNC.iter().find(|s| s.build == build)
}

pub(crate) fn warmup_gate(build: &str) -> Option<&'static WarmupGate> {
    WARMUP_GATES.iter().find(|s| s.build == build)
}

impl Engine {
    pub(crate) fn section(&self, name: &[u8]) -> Option<(u64, u32)> {
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
            if sec[..name.len()] == *name && sec[name.len()..8].iter().all(|&b| b == 0) {
                let size = u32::from_le_bytes([sec[8], sec[9], sec[10], sec[11]]);
                let base =
                    self.base + u32::from_le_bytes([sec[12], sec[13], sec[14], sec[15]]) as u64;
                return Some((base, size));
            }
        }
        None
    }

    fn alloc_near(&self, near_addr: u64, size: u64, kind: u32) -> u64 {
        let base = near_addr & !0xFFFFu64;
        let mut d = 0x10000u64;
        while d < 0x40000000 {
            if base > d {
                let p = unsafe {
                    VirtualAllocEx(
                        self.proc,
                        (base - d) as usize as *const c_void,
                        size as usize,
                        kind,
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
                    kind,
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

    fn load_shadow(&self, path: &Path) -> bool {
        let wpath: Vec<u8> = path
            .as_os_str()
            .encode_wide()
            .chain(std::iter::once(0))
            .flat_map(u16::to_le_bytes)
            .collect();
        let rem = self.alloc(wpath.len(), PAGE_READWRITE);
        if rem == 0 {
            return false;
        }
        let mut ok = false;
        let mut pending = false;
        if self.write_mem(rem, &wpath) {
            let kernel = unsafe { GetModuleHandleA(s!("kernel32.dll")) };
            let ll = unsafe { GetProcAddress(kernel, s!("LoadLibraryW")) };
            let start: LPTHREAD_START_ROUTINE = unsafe { std::mem::transmute(ll) };
            let th = unsafe {
                CreateRemoteThread(
                    self.proc,
                    core::ptr::null(),
                    0,
                    start,
                    rem as usize as *const c_void,
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
            unsafe { VirtualFreeEx(self.proc, rem as *mut c_void, 0, MEM_RELEASE) };
        }
        ok
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

    pub(crate) fn shadow_write(&self, offset: u64, bytes: &[u8]) {
        let addr = self.at(offset);
        if !self.shadow_pages.contains(&(addr & !0xFFFu64)) {
            return;
        }
        let mut buf = self.bytes(offset, bytes).to_vec();
        let mod_hi = self.base + self.modsize as u64;
        crate::relocate::relocate(&mut buf, addr, self.base, mod_hi, self.shadow_delta);
        self.write_mem((addr as i64 + self.shadow_delta) as u64, &buf);
    }

    fn release_and_fail(&self, shadow_base: u64) -> bool {
        unsafe { VirtualFreeEx(self.proc, shadow_base as *mut c_void, 0, MEM_RELEASE) };
        false
    }

    fn site_bytes<'a>(&self, rva: u64, dx: &'a [u8]) -> Option<&'a [u8]> {
        let hook = self.at(rva);
        let live = if self.shadow_pages.contains(&(hook & !0xFFFu64)) {
            (hook as i64 + self.shadow_delta) as u64
        } else {
            hook
        };
        let moved = self.bytes(rva, dx);
        let mut current = vec![0u8; moved.len()];
        (hook != 0 && self.read_mem(live, &mut current) == moved.len() && current == moved)
            .then_some(moved)
    }

    fn write_site(&self, rva: u64, opcode: u8, cave: u64, len: usize) {
        let hook = self.at(rva);
        let mut code = vec![opcode];
        code.extend_from_slice(&((cave as i64 - (hook + 5) as i64) as i32).to_le_bytes());
        code.resize(len, 0x90);
        if self.shadow_pages.contains(&(hook & !0xFFFu64)) {
            self.shadow_write(rva, &code);
        } else {
            self.write_mem(hook, &code);
        }
    }

    pub(crate) fn arm_warmup_gate(&self, site: &WarmupGate) {
        let (no, back) = (self.at(site.no), self.at(site.back));
        let Some(moved) = self.site_bytes(site.site, site.moved) else {
            return;
        };
        if no == 0 || back == 0 {
            return;
        }
        let cave = self.alloc_near(self.base, 0x1000, MEM_COMMIT | MEM_RESERVE);
        if cave == 0 {
            return;
        }
        let (high, low) = (site.mode >> 3, site.mode & 7);
        let rel = |c: &mut Vec<u8>, target: u64| {
            let next = cave + c.len() as u64 + 4;
            c.extend_from_slice(&((target as i64 - next as i64) as i32).to_le_bytes());
        };
        let mut c: Vec<u8> = vec![0x48 | (high * 5), 0x85, 0xC0 | (low * 9), 0x0F, 0x84];
        rel(&mut c, back);
        c.extend_from_slice(&[0x40 | high, 0x83, 0xB8 | low]);
        c.extend_from_slice(&site.kind.to_le_bytes());
        c.extend_from_slice(&[0x02, 0x0F, 0x84]);
        rel(&mut c, no);
        c.extend_from_slice(&[0x50, 0x48 | high, 0x8B, 0x80 | low]);
        c.extend_from_slice(&site.asset.to_le_bytes());
        c.extend_from_slice(&[
            0x48, 0x85, 0xC0, 0x74, 0x1F, 0x48, 0x8B, 0x00, 0x48, 0x85, 0xC0, 0x74, 0x17, 0x48,
            0x8B, 0x40, 0x10, 0x48, 0x3B, 0x05, 0x12, 0x00, 0x00, 0x00, 0x58, 0x0F, 0x84,
        ]);
        rel(&mut c, no);
        c.push(0xE9);
        rel(&mut c, back);
        c.extend_from_slice(&[0x58, 0xE9]);
        rel(&mut c, back);
        c.extend_from_slice(&WARMUP_MODE.to_le_bytes());
        if self.write_mem(cave, &c) {
            self.write_site(site.site, 0xE9, cave, moved.len());
        }
    }

    pub(crate) fn arm_playlist_sync(&self, site: &PlaylistSync) {
        let targets: Vec<u64> = match site.send {
            PlaylistSend::Block {
                vptr,
                ctor,
                fill,
                msg,
                send,
                dtor,
                ..
            } => vec![vptr, ctor, fill, msg, send, dtor],
            PlaylistSend::Channel {
                vptr,
                ctor,
                fill,
                msg,
                send,
                dtor,
                cookie,
            } => vec![vptr, ctor, fill, msg, send, dtor, cookie],
            PlaylistSend::Call(target) => vec![target],
        }
        .into_iter()
        .map(|rva| self.at(rva))
        .collect();
        if targets.contains(&0) {
            return;
        }
        let Some(moved) = self.site_bytes(site.hook, site.moved) else {
            return;
        };
        let cave = self.alloc_near(self.base, 0x1000, MEM_COMMIT | MEM_RESERVE);
        if cave == 0 {
            return;
        }
        let gm = if self.renderer.is_some() {
            site.alt_gm
        } else {
            site.gm
        };
        let rel = |c: &mut Vec<u8>, target: u64| {
            let next = cave + c.len() as u64 + 4;
            c.extend_from_slice(&((target as i64 - next as i64) as i32).to_le_bytes());
        };
        let frame: u32 = match site.send {
            PlaylistSend::Block { frame, .. } => frame,
            PlaylistSend::Channel { .. } => 0x218,
            PlaylistSend::Call(_) => 0x1A8,
        };
        let mut c: Vec<u8> = vec![0x53, 0x41, 0x50, 0x41, 0x51, 0x57, 0x48, 0x81, 0xEC];
        c.extend_from_slice(&frame.to_le_bytes());
        c.extend_from_slice(&[0x48 | (gm >> 3), 0x8B, 0xD8 | (gm & 7), 0x80, 0xBB]);
        c.extend_from_slice(&site.host.to_le_bytes());
        c.extend_from_slice(&[0x00, 0x0F, 0x84, 0, 0, 0, 0]);
        let j1 = c.len() - 4;
        c.extend_from_slice(&[0x48, 0x8B, 0x8B]);
        c.extend_from_slice(&site.net.to_le_bytes());
        c.extend_from_slice(&[0x48, 0x85, 0xC9, 0x0F, 0x84, 0, 0, 0, 0]);
        let j2 = c.len() - 4;
        if let [vptr, ctor, fill, msg, send, dtor] = targets[..] {
            c.extend_from_slice(&[0x48, 0x8D, 0x3D]);
            rel(&mut c, vptr);
            c.extend_from_slice(&[
                0x48, 0x8D, 0x4C, 0x24, 0x28, 0x48, 0x89, 0x7C, 0x24, 0x20, 0xE8,
            ]);
            rel(&mut c, ctor);
            c.extend_from_slice(&[0x48, 0x8D, 0x93]);
            c.extend_from_slice(&site.playlist.to_le_bytes());
            c.extend_from_slice(&[0x48, 0x8D, 0x4C, 0x24, 0x28, 0xE8]);
            rel(&mut c, fill);
            c.extend_from_slice(&[0x4C, 0x8D, 0x44, 0x24, 0x20, 0x48, 0x8B, 0x8B]);
            c.extend_from_slice(&site.net.to_le_bytes());
            c.extend_from_slice(&[0x48, 0x8D, 0x15]);
            rel(&mut c, msg);
            c.push(0xE8);
            rel(&mut c, send);
            c.extend_from_slice(&[
                0x48, 0x8D, 0x4C, 0x24, 0x28, 0x48, 0x89, 0x7C, 0x24, 0x20, 0xE8,
            ]);
            rel(&mut c, dtor);
        } else if let [vptr, ctor, fill, msg, send, dtor, cookie] = targets[..] {
            c.extend_from_slice(&[0x48, 0x8D, 0x3D]);
            rel(&mut c, vptr);
            c.extend_from_slice(&[
                0x48, 0x8D, 0x4C, 0x24, 0x40, 0x48, 0x89, 0x7C, 0x24, 0x38, 0xE8,
            ]);
            rel(&mut c, ctor);
            c.extend_from_slice(&[0x48, 0x8D, 0x93]);
            c.extend_from_slice(&site.playlist.to_le_bytes());
            c.extend_from_slice(&[0x48, 0x8D, 0x4C, 0x24, 0x40, 0xE8]);
            rel(&mut c, fill);
            c.extend_from_slice(&[0x48, 0x8B, 0x8B]);
            c.extend_from_slice(&site.net.to_le_bytes());
            c.extend_from_slice(&[0x48, 0x8B, 0x49, 0x10, 0x48, 0x85, 0xC9, 0x74, 0x00]);
            let j3 = c.len() - 1;
            c.extend_from_slice(&[
                0x48, 0x8B, 0x89, 0x90, 0x00, 0x00, 0x00, 0x48, 0x85, 0xC9, 0x74, 0x00,
            ]);
            let j4 = c.len() - 1;
            c.extend_from_slice(&[
                0x48, 0x8B, 0x49, 0x08, 0x8B, 0x51, 0x3C, 0x83, 0xFA, 0xFF, 0x74, 0x00,
            ]);
            let j5 = c.len() - 1;
            c.extend_from_slice(&[0x48, 0x8B, 0x0D]);
            rel(&mut c, cookie);
            c.extend_from_slice(&[
                0x48, 0x89, 0x8C, 0x24, 0x08, 0x02, 0x00, 0x00, 0x48, 0x8D, 0x8C, 0x24, 0x08, 0x02,
                0x00, 0x00, 0x48, 0x89, 0x4C, 0x24, 0x30, 0xC7, 0x44, 0x24, 0x28, 0xFF, 0xFF, 0xFF,
                0xFF, 0xC7, 0x44, 0x24, 0x20, 0x00, 0x00, 0x00, 0x00, 0x4C, 0x8D, 0x05,
            ]);
            rel(&mut c, msg);
            c.extend_from_slice(&[
                0x48, 0x8D, 0x8C, 0x24, 0x00, 0x02, 0x00, 0x00, 0x4C, 0x8D, 0x4C, 0x24, 0x38, 0xE8,
            ]);
            rel(&mut c, send);
            for j in [j3, j4, j5] {
                c[j] = (c.len() - j - 1) as u8;
            }
            c.extend_from_slice(&[
                0x48, 0x8D, 0x4C, 0x24, 0x40, 0x48, 0x89, 0x7C, 0x24, 0x38, 0xE8,
            ]);
            rel(&mut c, dtor);
        } else {
            c.extend_from_slice(&[0x48, 0x8D, 0x93]);
            c.extend_from_slice(&site.playlist.to_le_bytes());
            c.push(0xE8);
            rel(&mut c, targets[0]);
        }
        let out = c.len();
        for j in [j1, j2] {
            c[j..j + 4].copy_from_slice(&((out - j - 4) as u32).to_le_bytes());
        }
        c.extend_from_slice(&[0x48, 0x81, 0xC4]);
        c.extend_from_slice(&frame.to_le_bytes());
        c.extend_from_slice(&[0x5F, 0x41, 0x59, 0x41, 0x58, 0x5B]);
        c.extend_from_slice(moved);
        c.push(0xC3);
        if self.write_mem(cave, &c) {
            self.write_site(site.hook, 0xE8, cave, moved.len());
        }
    }

    pub(crate) fn shadow_run(&mut self, regions: &[ShadowRegion]) -> bool {
        let Some((text_base, text_size)) = self.section(b".text") else {
            return false;
        };
        let mod_hi = self.base + self.modsize as u64;

        let mut windows: Vec<(u64, u64)> = Vec::new();
        for r in regions {
            let page = self.at(r.offset) & !0xFFFu64;
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

        let mut copies: Vec<(u64, u64, Vec<u8>)> = Vec::new();
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
            if buf.chunks(0x1000).any(|page| page.iter().all(|&b| b == 0)) {
                return false;
            }
            copies.push((win_base, win_end, buf));
        }
        let probe = copies
            .iter()
            .fold(0xCBF2_9CE4_8422_2325u64, |h, (_, _, buf)| {
                buf.iter()
                    .fold(h, |h, &b| (h ^ u64::from(b)).wrapping_mul(0x100_0000_01B3))
            });
        if probe != self.shadow_probe {
            self.shadow_probe = probe;
            return false;
        }

        let shadow_base = self.alloc_near(text_base, text_size as u64, MEM_RESERVE);
        if shadow_base == 0 {
            return false;
        }
        let reserved_delta = shadow_base as i64 - text_base as i64;

        let mut committed: Vec<(u64, u64)> = Vec::new();
        for (win_base, win_end, mut buf) in copies {
            let win_size = buf.len();
            for r in regions {
                let addr = self.at(r.offset);
                let patch = self.bytes(r.offset, r.patch);
                if addr >= win_base && addr + patch.len() as u64 <= win_end {
                    let o = (addr - win_base) as usize;
                    buf[o..o + patch.len()].copy_from_slice(patch);
                }
            }
            crate::relocate::relocate(&mut buf, win_base, self.base, mod_hi, reserved_delta);
            let copy_base = (win_base as i64 + reserved_delta) as u64;
            let committed_copy = unsafe {
                VirtualAllocEx(
                    self.proc,
                    copy_base as usize as *const c_void,
                    win_size,
                    MEM_COMMIT,
                    PAGE_EXECUTE_READWRITE,
                )
            };
            if committed_copy.is_null() {
                continue;
            }
            if !self.write_mem(copy_base, &buf) {
                continue;
            }
            committed.push((win_base, win_end));
        }
        if committed.is_empty() {
            return self.release_and_fail(shadow_base);
        }

        let mut payload = [0u8; 32];
        payload[4..8].copy_from_slice(&3u32.to_le_bytes());
        payload[8..16].copy_from_slice(&text_base.to_le_bytes());
        payload[16..24].copy_from_slice(&(text_size as u64).to_le_bytes());
        payload[24..32].copy_from_slice(&shadow_base.to_le_bytes());
        let map = unsafe {
            CreateFileMappingW(
                INVALID_HANDLE_VALUE,
                core::ptr::null(),
                PAGE_READWRITE,
                0,
                payload.len() as u32,
                w!("ShadowRegions"),
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
        let loaded = existed || shadow_dll_path().is_some_and(|dll| self.load_shadow(&dll));
        unsafe { CloseHandle(map) };
        if !loaded || (!existed && !shadow_mapping_held()) {
            return self.release_and_fail(shadow_base);
        }
        if existed {
            unsafe { VirtualFreeEx(self.proc, shadow_base as *mut c_void, 0, MEM_RELEASE) };
        }

        self.shadow_pages.clear();
        self.shadow_delta = active_delta;
        for r in regions {
            let pa = self.at(r.offset) & !0xFFFu64;
            let covered = committed.iter().any(|&(b, e)| pa >= b && pa < e);
            if covered && !self.shadow_pages.contains(&pa) {
                self.shadow_pages.push(pa);
            }
        }
        true
    }
}

fn shadow_dll_path() -> Option<PathBuf> {
    let path = std::env::temp_dir().join("Shadow.dll");
    if std::fs::read(&path).ok().as_deref() != Some(SHADOW_DLL) {
        std::fs::write(&path, SHADOW_DLL).ok()?;
    }
    Some(path)
}

fn shadow_mapping_held() -> bool {
    let map = unsafe { OpenFileMappingW(FILE_MAP_READ, 0, w!("ShadowRegions")) };
    if map.is_null() {
        return false;
    }
    unsafe { CloseHandle(map) };
    true
}
