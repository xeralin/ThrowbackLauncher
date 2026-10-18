use core::ffi::c_void;
use std::collections::HashMap;
use std::ffi::CStr;
use std::io::{BufRead, BufReader, Write};
use std::mem::{size_of, zeroed};
use std::net::TcpStream;
use std::sync::mpsc::{self, RecvTimeoutError};
use std::time::{Duration, Instant};

use windows_sys::s;
use windows_sys::Win32::Foundation::{CloseHandle, HANDLE, INVALID_HANDLE_VALUE, STILL_ACTIVE};
use windows_sys::Win32::System::Diagnostics::Debug::{ReadProcessMemory, WriteProcessMemory};
use windows_sys::Win32::System::Diagnostics::ToolHelp::{
    CreateToolhelp32Snapshot, Module32First, Process32First, Process32Next, MODULEENTRY32,
    PROCESSENTRY32, TH32CS_SNAPMODULE, TH32CS_SNAPMODULE32, TH32CS_SNAPPROCESS,
};
use windows_sys::Win32::System::LibraryLoader::{GetModuleHandleA, GetProcAddress};
use windows_sys::Win32::System::Memory::{VirtualAllocEx, MEM_COMMIT, MEM_RESERVE};
use windows_sys::Win32::System::Threading::{
    GetExitCodeProcess, OpenProcess, PROCESS_CREATE_THREAD, PROCESS_QUERY_INFORMATION,
    PROCESS_VM_OPERATION, PROCESS_VM_READ, PROCESS_VM_WRITE,
};

use crate::shadow::shadow_regions_for_build;
use crate::tables::*;
use crate::tree::*;
use crate::unlock::{unlock, Badges, Onboarding, Ownership};

fn find_process(names: &[&'static str]) -> Option<(u32, &'static str)> {
    let snap = unsafe { CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0) };
    if snap == INVALID_HANDLE_VALUE {
        return None;
    }
    let mut pe: PROCESSENTRY32 = unsafe { zeroed() };
    pe.dwSize = size_of::<PROCESSENTRY32>() as u32;
    let mut found = None;
    if unsafe { Process32First(snap, &mut pe) } != 0 {
        loop {
            let exe = pe.szExeFile.map(|c| c as u8);
            let exe = CStr::from_bytes_until_nul(&exe).map_or(&[][..], CStr::to_bytes);
            if let Some(name) = names
                .iter()
                .find(|n| exe.eq_ignore_ascii_case(n.as_bytes()))
            {
                found = Some((pe.th32ProcessID, *name));
                break;
            }
            if unsafe { Process32Next(snap, &mut pe) } == 0 {
                break;
            }
        }
    }
    unsafe { CloseHandle(snap) };
    found
}

fn module_info(pid: u32) -> Option<(u64, u32)> {
    let snap = unsafe { CreateToolhelp32Snapshot(TH32CS_SNAPMODULE | TH32CS_SNAPMODULE32, pid) };
    if snap == INVALID_HANDLE_VALUE {
        return None;
    }
    let mut me: MODULEENTRY32 = unsafe { zeroed() };
    me.dwSize = size_of::<MODULEENTRY32>() as u32;
    let found = unsafe { Module32First(snap, &mut me) } != 0;
    unsafe { CloseHandle(snap) };
    found.then_some((me.modBaseAddr as u64, me.modBaseSize))
}

#[derive(Default)]
pub struct Engine {
    pub proc: HANDLE,
    pub base: u64,
    pub modsize: u32,
    pub exe: &'static str,
    pub(crate) shadow_pages: Vec<u64>,
    pub(crate) shadow_delta: i64,
}

impl Engine {
    pub fn attach(&mut self) -> bool {
        let Some((pid, name)) = find_process(&[
            "RainbowSixGame.exe",
            "RainbowSix.exe",
            "RainbowSix_DX11.exe",
        ]) else {
            return false;
        };
        let h = unsafe {
            OpenProcess(
                PROCESS_QUERY_INFORMATION
                    | PROCESS_VM_READ
                    | PROCESS_VM_WRITE
                    | PROCESS_VM_OPERATION
                    | PROCESS_CREATE_THREAD,
                0,
                pid,
            )
        };
        if h.is_null() {
            return false;
        }
        let Some((base, size)) = module_info(pid) else {
            unsafe { CloseHandle(h) };
            return false;
        };
        self.proc = h;
        self.base = base;
        self.modsize = size;
        self.exe = name;
        true
    }

    pub fn process_alive(&self) -> bool {
        let mut code: u32 = 0;
        let ok = unsafe { GetExitCodeProcess(self.proc, &mut code) };
        ok != 0 && code == STILL_ACTIVE as u32
    }

    pub fn read_mem(&self, addr: u64, buf: &mut [u8]) -> usize {
        let mut got: usize = 0;
        unsafe {
            ReadProcessMemory(
                self.proc,
                addr as usize as *const c_void,
                buf.as_mut_ptr() as *mut c_void,
                buf.len(),
                &mut got,
            );
        }
        got
    }

    pub(crate) fn read_u64(&self, addr: u64) -> u64 {
        let mut b = [0u8; 8];
        if self.read_mem(addr, &mut b) == 8 {
            u64::from_le_bytes(b)
        } else {
            0
        }
    }

    fn read_ptr(&self, addr: u64) -> u64 {
        let mut b = [0u8; 8];
        self.read_mem(addr, &mut b[..6]);
        u64::from_le_bytes(b)
    }

    fn read1(&self, addr: u64) -> u8 {
        let mut b = [0u8; 1];
        self.read_mem(addr, &mut b);
        b[0]
    }

    fn read_u32(&self, addr: u64) -> u32 {
        let mut b = [0u8; 4];
        self.read_mem(addr, &mut b);
        u32::from_le_bytes(b)
    }

    fn read_ascii(&self, addr: u64, len: usize) -> String {
        let mut buf = vec![0u8; len];
        self.read_mem(addr, &mut buf);
        let end = buf
            .windows(2)
            .position(|w| w == [0, 0])
            .unwrap_or(buf.len());
        let text: Vec<u8> = buf[..end].iter().copied().filter(|&c| c != 0).collect();
        String::from_utf8_lossy(&text).into_owned()
    }

    pub(crate) fn alloc(&self, size: usize, protect: u32) -> u64 {
        unsafe {
            VirtualAllocEx(
                self.proc,
                core::ptr::null(),
                size,
                MEM_COMMIT | MEM_RESERVE,
                protect,
            ) as u64
        }
    }

    pub fn write_mem(&self, addr: u64, bytes: &[u8]) -> bool {
        let mut wr: usize = 0;
        let ok = unsafe {
            WriteProcessMemory(
                self.proc,
                addr as usize as *const c_void,
                bytes.as_ptr() as *const c_void,
                bytes.len(),
                &mut wr,
            )
        };
        ok != 0 && wr == bytes.len()
    }

    pub fn detect_build(&self) -> Option<String> {
        const CHUNK: u32 = 1 << 20;
        const OVERLAP: u32 = 128;
        let (data, size) = self.section(b".data")?;
        let mut buf = vec![0u8; (CHUNK + OVERLAP) as usize];
        let mut off: u32 = 0;
        while off < size {
            let remain = size - off;
            let toread = remain.min(CHUNK + OVERLAP);
            let got = self.read_mem(data + off as u64, &mut buf[..toread as usize]);
            if let Some(m) = crate::scan_build::scan_build(&buf[..got]) {
                return Some(m);
            }
            off += CHUNK;
        }
        None
    }
}

fn find_chain(feature: &str, build: &str) -> Option<&'static FeatureChain> {
    FEATURE_CHAINS
        .iter()
        .find(|fc| fc.feature == feature && fc.build == build)
}

impl Engine {
    fn build_nodes(
        &self,
        address: u64,
        direct: bool,
        name_off: i32,
        children_off: i32,
        depth: i32,
    ) -> TNode {
        let mut node = TNode::default();
        let num = if direct {
            address
        } else {
            self.read_u64(self.read_u64(address))
        };
        let name = self.read_ascii(self.read_u64(num + name_off as u64), 100);
        node.text = if name.is_empty() {
            "<No Name>".to_string()
        } else {
            name
        };
        node.id = format!("{}", self.read_ptr(address));
        if depth >= 8 {
            return node;
        }
        let mut cnt = [0u8; 2];
        self.read_mem(num + children_off as u64 + 8, &mut cnt);
        let cnt = u16::from_le_bytes(cnt);
        let cnt = if cnt > 64 { 0 } else { cnt };
        let kids_base = self.read_u64(num + children_off as u64);
        for j in 0..cnt {
            let child = self.build_nodes(
                kids_base + (j as u64) * 8,
                false,
                name_off,
                children_off,
                depth + 1,
            );
            node.children.push(child);
        }
        node
    }
}

fn relabel_event(mp: &mut TNode, extract: &[usize], rm_path: &[usize], rm_idx: usize, label: &str) {
    if let Some(c) = get_path_mut(mp, extract).cloned() {
        mp.children.push(c);
    }
    rm(mp, rm_path, rm_idx);
    set_text(mp, &[5], label);
}

impl Engine {
    fn resolve_ptr_chain(&self, base_offset: u64, offsets: &[u64]) -> u64 {
        let Some((last, path)) = offsets.split_last() else {
            return 0;
        };
        let ptr = path
            .iter()
            .try_fold(self.read_u64(self.base + base_offset), |p, off| {
                (p != 0).then(|| self.read_u64(p.wrapping_add(*off)))
            });
        match ptr {
            Some(p) if p != 0 => p.wrapping_add(*last),
            _ => 0,
        }
    }
}

fn json_value<'a>(line: &'a str, key: &str) -> Option<&'a str> {
    let key = format!("\"{key}\"");
    let rest = &line[line.find(&key)? + key.len()..];
    rest.trim_start_matches([' ', '\t'])
        .strip_prefix(':')
        .map(|v| v.trim_start_matches([' ', '\t']))
}

fn json_str<'a>(line: &'a str, key: &str) -> Option<&'a str> {
    json_value(line, key)?.strip_prefix('"')?.split('"').next()
}

fn json_bool(line: &str, key: &str) -> bool {
    json_value(line, key).is_some_and(|v| v.starts_with("true"))
}

static CAPS: [(&str, &str); 12] = [
    ("deathless", "SetDeathless"),
    ("disableAI", "SetDisableAI"),
    ("unlimitedAmmo", "SetUnlimitedAmmo"),
    ("unlimitedEquip", "SetUnlimitedEquipment"),
    ("infiniteTime", "SetInfiniteTime"),
    ("disablePrimary", "SetDisablePrimaryWeapon"),
    ("disableSecondary", "SetDisableSecondaryWeapon"),
    ("disablePrimaryGadget", "SetDisablePrimaryGadget"),
    ("disableSecondaryGadget", "SetDisableSecondaryGadget"),
    ("displayBuild", "SetDisplayBuild"),
    ("endRound", "EndRound"),
    ("endMatch", "EndMatch"),
];

static STATE_WRITES: &[(&str, &str, u64, i32, u8)] = &[
    ("Y1S0_8194013", "EndRound", 0x5880AA8, 0x66C, 5),
    ("Y1S0_8194013", "EndMatch", 0x5880AA8, 0x66C, 2),
];

static INIT_BYTES: &[(&str, u64, u8)] = &[("Y8S3_62486471", 0x7FCA406, 0)];

static WATERMARKS: &[(&str, u64, u64, u64, u64)] = &[
    ("Y6S4_37046065", 0x60D353A, 0x8111FF8, 0x13E3, 0x13E9),
    ("Y7S1_39785137", 0x699F56A, 0x8B90F80, 0x1423, 0x1428),
    ("Y7S2_43489433", 0x676C429, 0x895D618, 0x1483, 0x1488),
    ("Y7S3_47165842", 0x6D39679, 0x8ED3F40, 0x14CB, 0x14D0),
    ("Y7S4_50497889", 0x6E508D8, 0x8852A78, 0x147B, 0x1480),
    ("Y8S1_55217154", 0x6B2C4F8, 0x8D75F20, 0x14BB, 0x14C0),
];

static WATERMARK_OVERLAYS: &[(&str, u64, u64, u64)] =
    &[("Y8S2_58222837", 0x6DF1EA0, 0x755A398, 0x6D73D98)];

fn patches_have(feature: &str, build: &str) -> bool {
    PATCHES
        .iter()
        .any(|p| p.feature == feature && p.build == build)
}

const TREE_NULL: &str = "{\"event\":\"tree\",\"tree\":null}";

static Y5_DISABLE_SLOTS: [&str; 4] = [
    "disablePrimary",
    "disableSecondary",
    "disablePrimaryGadget",
    "disableSecondaryGadget",
];

const Y5_SLOTS: [i32; 4] = [0x10, 0x18, 0x28, 0x38];

static Y5S1_TOGGLES: &[(&str, u64, &[u8])] = &[
    (
        "deathless",
        0x1842673,
        &[
            0xC7, 0x87, 0x68, 0x01, 0x00, 0x00, 0x78, 0x00, 0x00, 0x00, 0x90, 0x90, 0x90, 0x90,
            0x90,
        ],
    ),
    (
        "unlimitedEquip",
        0x12984AB,
        &[
            0xC7, 0x41, 0x54, 0x06, 0x00, 0x00, 0x00, 0x48, 0x8B, 0x49, 0x18, 0x90, 0x90, 0x90,
            0x90, 0xEB,
        ],
    ),
    ("disableAI", 0x1081B80, &[0xC3]),
    (
        "unlimitedAmmo",
        0x1AB36DB,
        &[
            0x44, 0x3B, 0x40, 0x7C, 0x90, 0x48, 0x89, 0x7C, 0x24, 0x50, 0x7E, 0x05, 0x44, 0x89,
            0x40, 0x7C, 0x90,
        ],
    ),
    ("displayBuild", 0x7386F8, &[0x90, 0x90]),
    ("displayBuild", 0x104E6C, &[0x00]),
    ("displayBuild", 0x738287, &[0xB1, 0x01, 0x90]),
    ("endMatch", 0x1B6CF8A, &[0xEB, 0x00]),
];

static Y6S1_TOGGLES: &[(&str, u64, &[u8])] = &[
    ("displayBuild", 0x7125FA, &[0xEB, 0x09]),
    (
        "displayBuild",
        0x357DC5D,
        &[0x48, 0x8D, 0x15, 0x37, 0xB1, 0x0F, 0x02],
    ),
    ("displayBuild", 0x7121CA, &[0xB0, 0x01, 0x90]),
    (
        "deathless",
        0x273893,
        &[
            0x44, 0x8D, 0x34, 0x25, 0x78, 0x00, 0x00, 0x00, 0x45, 0x89, 0xB4, 0x24, 0x70, 0x01,
            0x00, 0x00,
        ],
    ),
    ("unlimitedAmmo", 0x1386388, &[0x0F, 0x1F, 0x00]),
    (
        "unlimitedAmmo",
        0x1392026,
        &[0x0F, 0x1F, 0x80, 0x00, 0x00, 0x00, 0x00],
    ),
    (
        "unlimitedAmmo",
        0x139214C,
        &[0x0F, 0x1F, 0x80, 0x00, 0x00, 0x00, 0x00],
    ),
    ("unlimitedAmmo", 0x16337B5, &[0x00]),
    ("unlimitedAmmo", 0x162F1DD, &[0x72]),
    ("unlimitedAmmo", 0x162F20F, &[0x83]),
    ("unlimitedAmmo", 0x163049F, &[0x92]),
    ("disableAI", 0x339EB80, &[0xC3]),
    ("endMatch", 0x116F1F9, &[0xB0, 0xB7]),
    (
        "unlimitedEquip",
        0x2F6B42D,
        &[0x48, 0xC7, 0xC6, 0x06, 0x00, 0x00, 0x00],
    ),
    ("unlimitedEquip", 0x905731, &[0x83, 0xC0, 0x00]),
    ("unlimitedEquip", 0x4391077, &[0x83, 0xC0, 0x00]),
    ("unlimitedEquip", 0x252E792, &[0x83, 0x46, 0x44, 0x00]),
    ("unlimitedEquip", 0x29F280, &[0x83, 0x43, 0x44, 0x00]),
    ("unlimitedEquip", 0x43910F0, &[0x83, 0x43, 0x44, 0x00]),
    (
        "unlimitedEquip",
        0x1D93F9F,
        &[
            0xBE, 0x06, 0x00, 0x00, 0x00, 0x48, 0x89, 0xCB, 0x66, 0x0F, 0x1F, 0x44, 0x00, 0x00,
        ],
    ),
    (
        "unlimitedEquip",
        0x238DFBD,
        &[
            0xBD, 0xFF, 0xFF, 0xFF, 0xFF, 0xBE, 0x06, 0x00, 0x00, 0x00, 0x0F, 0x1F, 0x00,
        ],
    ),
    (
        "unlimitedEquip",
        0x4387D42,
        &[
            0xBD, 0xFF, 0xFF, 0xFF, 0xFF, 0xBE, 0x06, 0x00, 0x00, 0x00, 0x66, 0x90,
        ],
    ),
    (
        "unlimitedEquip",
        0x438A39A,
        &[
            0xBD, 0xFF, 0xFF, 0xFF, 0xFF, 0xBF, 0x06, 0x00, 0x00, 0x00, 0x0F, 0x1F, 0x44, 0x00,
            0x00,
        ],
    ),
];

static Y6S1_15500403_TOGGLES: &[(&str, u64, &[u8])] = &[
    ("displayBuild", 0x3D689EA, &[0xEB, 0x09]),
    (
        "displayBuild",
        0x4017F8D,
        &[0x48, 0x8D, 0x15, 0xF0, 0x39, 0x68, 0x01],
    ),
    ("displayBuild", 0x3D685BA, &[0xB0, 0x01, 0x90]),
    (
        "deathless",
        0x45BCA23,
        &[
            0x44, 0x8D, 0x34, 0x25, 0x78, 0x00, 0x00, 0x00, 0x45, 0x89, 0xB4, 0x24, 0x70, 0x01,
            0x00, 0x00,
        ],
    ),
    ("unlimitedAmmo", 0x25A19E3, &[0x0F, 0x1F, 0x00]),
    (
        "unlimitedAmmo",
        0x25AD7E9,
        &[0x0F, 0x1F, 0x80, 0x00, 0x00, 0x00, 0x00],
    ),
    (
        "unlimitedAmmo",
        0x25AD907,
        &[0x0F, 0x1F, 0x80, 0x00, 0x00, 0x00, 0x00],
    ),
    ("unlimitedAmmo", 0x25B7EF5, &[0x00]),
    ("unlimitedAmmo", 0x25B39D3, &[0x72]),
    ("unlimitedAmmo", 0x25B3A05, &[0x83]),
    ("unlimitedAmmo", 0x25B4C58, &[0x92]),
    ("disableAI", 0x45E20, &[0xC3]),
    ("endMatch", 0x224D2B1, &[0xB0, 0xF4]),
    (
        "unlimitedEquip",
        0x365270D,
        &[0x48, 0xC7, 0xC6, 0x06, 0x00, 0x00, 0x00],
    ),
    ("unlimitedEquip", 0x22F28A1, &[0x83, 0xC0, 0x00]),
    ("unlimitedEquip", 0x5FB307, &[0x83, 0xC0, 0x00]),
    ("unlimitedEquip", 0x22F78EF, &[0x83, 0x46, 0x44, 0x00]),
    ("unlimitedEquip", 0x2C0FFA0, &[0x83, 0x43, 0x44, 0x00]),
    ("unlimitedEquip", 0x5FB380, &[0x83, 0x43, 0x44, 0x00]),
    (
        "unlimitedEquip",
        0x2D1FE5F,
        &[
            0xBE, 0x06, 0x00, 0x00, 0x00, 0x48, 0x89, 0xCB, 0x66, 0x0F, 0x1F, 0x44, 0x00, 0x00,
        ],
    ),
    (
        "unlimitedEquip",
        0x2E93CFD,
        &[
            0xBD, 0xFF, 0xFF, 0xFF, 0xFF, 0xBE, 0x06, 0x00, 0x00, 0x00, 0x0F, 0x1F, 0x00,
        ],
    ),
    (
        "unlimitedEquip",
        0x5F1FB2,
        &[
            0xBD, 0xFF, 0xFF, 0xFF, 0xFF, 0xBE, 0x06, 0x00, 0x00, 0x00, 0x66, 0x90,
        ],
    ),
    (
        "unlimitedEquip",
        0x5F460A,
        &[
            0xBD, 0xFF, 0xFF, 0xFF, 0xFF, 0xBF, 0x06, 0x00, 0x00, 0x00, 0x0F, 0x1F, 0x44, 0x00,
            0x00,
        ],
    ),
];

static Y6S2_TOGGLES: &[(&str, u64, &[u8])] = &[
    ("displayBuild", 0x2DEC67A, &[0xEB, 0x09]),
    (
        "displayBuild",
        0x3B44E9D,
        &[0x48, 0x8D, 0x15, 0x69, 0x97, 0xC9, 0x01],
    ),
    ("displayBuild", 0x2DEC35F, &[0xB0, 0x01, 0x90]),
    (
        "deathless",
        0x43D8EA7,
        &[
            0x0F, 0x1F, 0x00, 0xB8, 0x78, 0x00, 0x00, 0x00, 0x41, 0x89, 0x86, 0x78, 0x01, 0x00,
            0x00, 0xF3, 0x0F, 0x2A, 0xF0,
        ],
    ),
    ("unlimitedAmmo", 0x3354C44, &[0x0F, 0x1F, 0x00]),
    (
        "unlimitedAmmo",
        0x33602CB,
        &[0x0F, 0x1F, 0x80, 0x00, 0x00, 0x00, 0x00],
    ),
    (
        "unlimitedAmmo",
        0x33603FF,
        &[0x0F, 0x1F, 0x80, 0x00, 0x00, 0x00, 0x00],
    ),
    ("unlimitedAmmo", 0x336A955, &[0x00]),
    ("unlimitedAmmo", 0x3366737, &[0x72]),
    ("unlimitedAmmo", 0x3366769, &[0x83]),
    ("unlimitedAmmo", 0x33678B9, &[0x92]),
    ("disableAI", 0x1C70EA0, &[0xC3]),
    ("endMatch", 0x90875B, &[0x75, 0x45, 0xB0, 0x01]),
    (
        "unlimitedEquip",
        0x3C96134,
        &[0x48, 0xC7, 0xC6, 0x06, 0x00, 0x00, 0x00],
    ),
    ("unlimitedEquip", 0x3CA2CC7, &[0x83, 0xC0, 0x00]),
    ("unlimitedEquip", 0x153F2D7, &[0x83, 0xC0, 0x00]),
    ("unlimitedEquip", 0x3CA7D82, &[0x83, 0x46, 0x44, 0x00]),
    ("unlimitedEquip", 0x107C1D0, &[0x83, 0x43, 0x44, 0x00]),
    ("unlimitedEquip", 0x153F350, &[0x83, 0x43, 0x44, 0x00]),
    (
        "unlimitedEquip",
        0x3B19BE,
        &[
            0xBD, 0xFF, 0xFF, 0xFF, 0xFF, 0xBE, 0x06, 0x00, 0x00, 0x00, 0x66, 0x0F, 0x1F, 0x44,
            0x00, 0x00,
        ],
    ),
    (
        "unlimitedEquip",
        0x2DA976D,
        &[
            0xBD, 0xFF, 0xFF, 0xFF, 0xFF, 0xBE, 0x06, 0x00, 0x00, 0x00, 0x0F, 0x1F, 0x00,
        ],
    ),
    (
        "unlimitedEquip",
        0x1536592,
        &[
            0xBD, 0xFF, 0xFF, 0xFF, 0xFF, 0xBE, 0x06, 0x00, 0x00, 0x00, 0x66, 0x90,
        ],
    ),
    (
        "unlimitedEquip",
        0x1538A1A,
        &[
            0xBD, 0xFF, 0xFF, 0xFF, 0xFF, 0xBF, 0x06, 0x00, 0x00, 0x00, 0x0F, 0x1F, 0x44, 0x00,
            0x00,
        ],
    ),
];

static Y6S2_15701375_TOGGLES: &[(&str, u64, &[u8])] = &[
    ("displayBuild", 0x2997F4A, &[0xEB, 0x09]),
    (
        "displayBuild",
        0x2F1410D,
        &[0x48, 0x8D, 0x15, 0xC0, 0x41, 0x8A, 0x02],
    ),
    ("displayBuild", 0x299776F, &[0xB0, 0x01, 0x90]),
    (
        "deathless",
        0x39F8127,
        &[
            0x0F, 0x1F, 0x00, 0xB8, 0x78, 0x00, 0x00, 0x00, 0x41, 0x89, 0x86, 0x78, 0x01, 0x00,
            0x00, 0xF3, 0x0F, 0x2A, 0xF0,
        ],
    ),
    ("unlimitedAmmo", 0x3B2E15C, &[0x0F, 0x1F, 0x00]),
    (
        "unlimitedAmmo",
        0x3B39DAF,
        &[0x0F, 0x1F, 0x80, 0x00, 0x00, 0x00, 0x00],
    ),
    (
        "unlimitedAmmo",
        0x3B39ED7,
        &[0x0F, 0x1F, 0x80, 0x00, 0x00, 0x00, 0x00],
    ),
    ("unlimitedAmmo", 0x3B44645, &[0x00]),
    ("unlimitedAmmo", 0x3B403AA, &[0x72]),
    ("unlimitedAmmo", 0x3B403DD, &[0x83]),
    ("unlimitedAmmo", 0x3B4159C, &[0x92]),
    ("disableAI", 0xC56B40, &[0xC3]),
    ("endMatch", 0x46399C5, &[0x75, 0x3B, 0xB0, 0x01]),
    (
        "unlimitedEquip",
        0x33C61A4,
        &[0x48, 0xC7, 0xC6, 0x06, 0x00, 0x00, 0x00],
    ),
    ("unlimitedEquip", 0x428FA97, &[0x83, 0xC0, 0x00]),
    ("unlimitedEquip", 0x4605C97, &[0x83, 0xC0, 0x00]),
    ("unlimitedEquip", 0x4294B92, &[0x83, 0x46, 0x44, 0x00]),
    ("unlimitedEquip", 0x20617C0, &[0x83, 0x43, 0x44, 0x00]),
    ("unlimitedEquip", 0x4605D10, &[0x83, 0x43, 0x44, 0x00]),
    (
        "unlimitedEquip",
        0x256F81E,
        &[
            0xBD, 0xFF, 0xFF, 0xFF, 0xFF, 0xBE, 0x06, 0x00, 0x00, 0x00, 0x66, 0x0F, 0x1F, 0x44,
            0x00, 0x00,
        ],
    ),
    (
        "unlimitedEquip",
        0x33F28D,
        &[
            0xBD, 0xFF, 0xFF, 0xFF, 0xFF, 0xBE, 0x06, 0x00, 0x00, 0x00, 0x0F, 0x1F, 0x00,
        ],
    ),
    (
        "unlimitedEquip",
        0x45FCF52,
        &[
            0xBD, 0xFF, 0xFF, 0xFF, 0xFF, 0xBE, 0x06, 0x00, 0x00, 0x00, 0x66, 0x90,
        ],
    ),
    (
        "unlimitedEquip",
        0x45FF3DA,
        &[
            0xBD, 0xFF, 0xFF, 0xFF, 0xFF, 0xBF, 0x06, 0x00, 0x00, 0x00, 0x0F, 0x1F, 0x44, 0x00,
            0x00,
        ],
    ),
];

static Y6S3_TOGGLES: &[(&str, u64, &[u8])] = &[
    ("displayBuild", 0x1E1424A, &[0xEB, 0x09]),
    (
        "displayBuild",
        0x4657FDD,
        &[0x48, 0x8D, 0x15, 0x1C, 0xED, 0x68, 0x01],
    ),
    ("displayBuild", 0x1E13EEF, &[0xB0, 0x01, 0x90]),
    (
        "deathless",
        0xEEACE7,
        &[
            0x0F, 0x1F, 0x00, 0xB8, 0x78, 0x00, 0x00, 0x00, 0x41, 0x89, 0x86, 0xBC, 0x01, 0x00,
            0x00, 0xF3, 0x0F, 0x2A, 0xF0,
        ],
    ),
    ("unlimitedAmmo", 0x2D97BD9, &[0x0F, 0x1F, 0x00]),
    (
        "unlimitedAmmo",
        0x3E95DBD,
        &[0x66, 0x0F, 0x1F, 0x44, 0x00, 0x00],
    ),
    (
        "unlimitedAmmo",
        0x3E95E98,
        &[0x66, 0x0F, 0x1F, 0x44, 0x00, 0x00],
    ),
    ("unlimitedAmmo", 0x3EA0BFF, &[0x00]),
    ("unlimitedAmmo", 0x3E9CA12, &[0x72]),
    ("unlimitedAmmo", 0x3E9CA45, &[0x83]),
    ("unlimitedAmmo", 0x3E9DAE6, &[0x92]),
    ("disableAI", 0x14D2480, &[0xC3]),
    ("endMatch", 0xDDFDDC, &[0x75, 0x44, 0xB0, 0x01]),
    (
        "unlimitedEquip",
        0x10AEF64,
        &[0x48, 0xC7, 0xC6, 0x06, 0x00, 0x00, 0x00],
    ),
    ("unlimitedEquip", 0x2D57017, &[0x83, 0xC0, 0x00]),
    ("unlimitedEquip", 0x1E84227, &[0x83, 0xC0, 0x00]),
    ("unlimitedEquip", 0x2D5C18D, &[0x83, 0x46, 0x44, 0x00]),
    ("unlimitedEquip", 0x1AC1D00, &[0x83, 0x43, 0x44, 0x00]),
    ("unlimitedEquip", 0x1E842A0, &[0x83, 0x43, 0x44, 0x00]),
    (
        "unlimitedEquip",
        0x43C321E,
        &[
            0xBD, 0xFF, 0xFF, 0xFF, 0xFF, 0xBE, 0x06, 0x00, 0x00, 0x00, 0x66, 0x0F, 0x1F, 0x44,
            0x00, 0x00,
        ],
    ),
    (
        "unlimitedEquip",
        0x28E910D,
        &[
            0xBD, 0xFF, 0xFF, 0xFF, 0xFF, 0xBE, 0x06, 0x00, 0x00, 0x00, 0x0F, 0x1F, 0x00,
        ],
    ),
    (
        "unlimitedEquip",
        0xBA9412,
        &[
            0xBD, 0xFF, 0xFF, 0xFF, 0xFF, 0xBE, 0x06, 0x00, 0x00, 0x00, 0x66, 0x90,
        ],
    ),
    (
        "unlimitedEquip",
        0xBAB7AA,
        &[
            0xBD, 0xFF, 0xFF, 0xFF, 0xFF, 0xBF, 0x06, 0x00, 0x00, 0x00, 0x0F, 0x1F, 0x44, 0x00,
            0x00,
        ],
    ),
];

static Y6S3_32627923_TOGGLES: &[(&str, u64, &[u8])] = &[
    ("displayBuild", 0x1B1F30A, &[0xEB, 0x09]),
    (
        "displayBuild",
        0x32EF57D,
        &[0x48, 0x8D, 0x15, 0x8A, 0x14, 0x71, 0x02],
    ),
    ("displayBuild", 0x1B1EEEF, &[0xB0, 0x01, 0x90]),
    (
        "deathless",
        0x222607,
        &[
            0x0F, 0x1F, 0x00, 0xB8, 0x78, 0x00, 0x00, 0x00, 0x41, 0x89, 0x86, 0x7C, 0x01, 0x00,
            0x00, 0xF3, 0x0F, 0x2A, 0xF0,
        ],
    ),
    ("unlimitedAmmo", 0x1B92CAB, &[0x0F, 0x1F, 0x00]),
    (
        "unlimitedAmmo",
        0x1B9E8DE,
        &[0x66, 0x0F, 0x1F, 0x44, 0x00, 0x00],
    ),
    (
        "unlimitedAmmo",
        0x1B9E9B9,
        &[0x66, 0x0F, 0x1F, 0x44, 0x00, 0x00],
    ),
    ("unlimitedAmmo", 0x1BA8F37, &[0x00]),
    ("unlimitedAmmo", 0x1BA4EFF, &[0x72]),
    ("unlimitedAmmo", 0x1BA4F32, &[0x83]),
    ("unlimitedAmmo", 0x1BA5FFB, &[0x92]),
    ("disableAI", 0xC82EA0, &[0xC3]),
    ("endMatch", 0x2E8AAD8, &[0x75, 0x48, 0xB0, 0x01]),
    (
        "unlimitedEquip",
        0x1DD6E74,
        &[0x48, 0xC7, 0xC6, 0x06, 0x00, 0x00, 0x00],
    ),
    ("unlimitedEquip", 0x26EC647, &[0x83, 0xC0, 0x00]),
    ("unlimitedEquip", 0x3F1BCC7, &[0x83, 0xC0, 0x00]),
    ("unlimitedEquip", 0x26F16F9, &[0x83, 0x46, 0x44, 0x00]),
    ("unlimitedEquip", 0x3E089A0, &[0x83, 0x43, 0x44, 0x00]),
    ("unlimitedEquip", 0x3F1BD40, &[0x83, 0x43, 0x44, 0x00]),
    (
        "unlimitedEquip",
        0x1F986EE,
        &[
            0xBD, 0xFF, 0xFF, 0xFF, 0xFF, 0xBE, 0x06, 0x00, 0x00, 0x00, 0x66, 0x0F, 0x1F, 0x44,
            0x00, 0x00,
        ],
    ),
    (
        "unlimitedEquip",
        0x3F8879D,
        &[
            0xBD, 0xFF, 0xFF, 0xFF, 0xFF, 0xBE, 0x06, 0x00, 0x00, 0x00, 0x0F, 0x1F, 0x00,
        ],
    ),
    (
        "unlimitedEquip",
        0x2947D2,
        &[
            0xBD, 0xFF, 0xFF, 0xFF, 0xFF, 0xBE, 0x06, 0x00, 0x00, 0x00, 0x66, 0x90,
        ],
    ),
    (
        "unlimitedEquip",
        0x296ACA,
        &[
            0xBD, 0xFF, 0xFF, 0xFF, 0xFF, 0xBF, 0x06, 0x00, 0x00, 0x00, 0x0F, 0x1F, 0x44, 0x00,
            0x00,
        ],
    ),
];

static Y6S4_TOGGLES: &[(&str, u64, &[u8])] = &[
    (
        "deathless",
        0x2A16527,
        &[
            0x0F, 0x1F, 0x00, 0xB8, 0x78, 0x00, 0x00, 0x00, 0x41, 0x89, 0x87, 0xBC, 0x01, 0x00,
            0x00, 0xF3, 0x0F, 0x2A, 0xF0,
        ],
    ),
    ("unlimitedAmmo", 0x32BE4D9, &[0x0F, 0x1F, 0x00]),
    (
        "unlimitedAmmo",
        0x32CA18E,
        &[0x66, 0x0F, 0x1F, 0x44, 0x00, 0x00],
    ),
    (
        "unlimitedAmmo",
        0x32CA269,
        &[0x66, 0x0F, 0x1F, 0x44, 0x00, 0x00],
    ),
    ("unlimitedAmmo", 0x32D6BAD, &[0x00]),
    ("unlimitedAmmo", 0x32D2EC0, &[0x83]),
    ("unlimitedAmmo", 0x32D3F2D, &[0x92]),
    ("disableAI", 0x404DB50, &[0xC3]),
    ("endMatch", 0x7E4D82, &[0x78, 0x3E, 0xB0, 0x01]),
    (
        "unlimitedEquip",
        0x103E814,
        &[0x48, 0xC7, 0xC6, 0x06, 0x00, 0x00, 0x00],
    ),
    ("unlimitedEquip", 0x233393A, &[0x8D, 0x43, 0x00]),
    ("unlimitedEquip", 0x490D9E9, &[0x8D, 0x47, 0x00]),
    ("unlimitedEquip", 0x23388DF, &[0x83, 0x46, 0x44, 0x00]),
    ("unlimitedEquip", 0x833980, &[0x83, 0x43, 0x44, 0x00]),
    ("unlimitedEquip", 0x490DA60, &[0x83, 0x43, 0x44, 0x00]),
    (
        "unlimitedEquip",
        0x3A48C2E,
        &[
            0xBD, 0xFF, 0xFF, 0xFF, 0xFF, 0xBE, 0x06, 0x00, 0x00, 0x00, 0x66, 0x0F, 0x1F, 0x44,
            0x00, 0x00,
        ],
    ),
    (
        "unlimitedEquip",
        0x2FC77CD,
        &[
            0xBD, 0xFF, 0xFF, 0xFF, 0xFF, 0xBE, 0x06, 0x00, 0x00, 0x00, 0x0F, 0x1F, 0x00,
        ],
    ),
    (
        "unlimitedEquip",
        0x49058D2,
        &[
            0xBD, 0xFF, 0xFF, 0xFF, 0xFF, 0xBE, 0x06, 0x00, 0x00, 0x00, 0x66, 0x90,
        ],
    ),
    (
        "unlimitedEquip",
        0x49078FA,
        &[
            0xBD, 0xFF, 0xFF, 0xFF, 0xFF, 0xBF, 0x06, 0x00, 0x00, 0x00, 0x0F, 0x1F, 0x44, 0x00,
            0x00,
        ],
    ),
];

static Y7S1_TOGGLES: &[(&str, u64, &[u8])] = &[
    (
        "deathless",
        0x4D9A3E7,
        &[
            0x0F, 0x1F, 0x00, 0xB8, 0x78, 0x00, 0x00, 0x00, 0x41, 0x89, 0x85, 0xBC, 0x01, 0x00,
            0x00, 0xF3, 0x0F, 0x2A, 0xF0,
        ],
    ),
    ("unlimitedAmmo", 0x141E694, &[0x66, 0x90]),
    (
        "unlimitedAmmo",
        0x572E3F,
        &[0x66, 0x0F, 0x1F, 0x44, 0x00, 0x00],
    ),
    (
        "unlimitedAmmo",
        0x572F5B,
        &[0x66, 0x0F, 0x1F, 0x44, 0x00, 0x00],
    ),
    ("unlimitedAmmo", 0x581109, &[0x00]),
    ("unlimitedAmmo", 0x57CD0C, &[0x83]),
    ("unlimitedAmmo", 0x57DDF7, &[0x92]),
    ("disableAI", 0x25DAF40, &[0xC3]),
    ("endMatch", 0x4CE7A7C, &[0x74, 0x34, 0xB0, 0x01]),
    (
        "unlimitedEquip",
        0x1647675,
        &[0x48, 0xC7, 0xC6, 0x06, 0x00, 0x00, 0x00],
    ),
    ("unlimitedEquip", 0xA742A2, &[0x41, 0x8D, 0x44, 0x24, 0x00]),
    ("unlimitedEquip", 0x2A35829, &[0x8D, 0x47, 0x00]),
    ("unlimitedEquip", 0xA79180, &[0x83, 0x43, 0x44, 0x00]),
    ("unlimitedEquip", 0xD3AE90, &[0x83, 0x43, 0x44, 0x00]),
    ("unlimitedEquip", 0x2A358A0, &[0x83, 0x43, 0x44, 0x00]),
    (
        "unlimitedEquip",
        0x157180B,
        &[
            0xBA, 0x06, 0x00, 0x00, 0x00, 0x66, 0x0F, 0x1F, 0x84, 0x00, 0x00, 0x00, 0x00, 0x00,
        ],
    ),
    (
        "unlimitedEquip",
        0x1573212,
        &[
            0xBA, 0x06, 0x00, 0x00, 0x00, 0x66, 0x0F, 0x1F, 0x84, 0x00, 0x00, 0x00, 0x00, 0x00,
        ],
    ),
    (
        "unlimitedEquip",
        0x1ADBAB4,
        &[
            0xBA, 0x06, 0x00, 0x00, 0x00, 0x66, 0x0F, 0x1F, 0x84, 0x00, 0x00, 0x00, 0x00, 0x00,
        ],
    ),
    (
        "unlimitedEquip",
        0x2A4AC9B,
        &[
            0xBA, 0x06, 0x00, 0x00, 0x00, 0x66, 0x0F, 0x1F, 0x84, 0x00, 0x00, 0x00, 0x00, 0x00,
        ],
    ),
    (
        "unlimitedEquip",
        0x2A50FE6,
        &[
            0xBA, 0x06, 0x00, 0x00, 0x00, 0x66, 0x0F, 0x1F, 0x84, 0x00, 0x00, 0x00, 0x00, 0x00,
        ],
    ),
    (
        "unlimitedEquip",
        0x2A53806,
        &[
            0xBA, 0x06, 0x00, 0x00, 0x00, 0x66, 0x0F, 0x1F, 0x84, 0x00, 0x00, 0x00, 0x00, 0x00,
        ],
    ),
    (
        "unlimitedEquip",
        0x2A69269,
        &[
            0xBA, 0x06, 0x00, 0x00, 0x00, 0x66, 0x0F, 0x1F, 0x84, 0x00, 0x00, 0x00, 0x00, 0x00,
        ],
    ),
    (
        "unlimitedEquip",
        0x24D7340,
        &[
            0xBA, 0x06, 0x00, 0x00, 0x00, 0x66, 0x0F, 0x1F, 0x44, 0x00, 0x00,
        ],
    ),
    (
        "unlimitedEquip",
        0x40020B0,
        &[
            0xB8, 0x06, 0x00, 0x00, 0x00, 0x0F, 0x1F, 0x80, 0x00, 0x00, 0x00, 0x00,
        ],
    ),
    (
        "unlimitedEquip",
        0x4004C4A,
        &[
            0xBD, 0xFF, 0xFF, 0xFF, 0xFF, 0xBE, 0x06, 0x00, 0x00, 0x00, 0x0F, 0x1F, 0x44, 0x00,
            0x00,
        ],
    ),
    (
        "unlimitedEquip",
        0x1573E2A,
        &[0xBD, 0xFF, 0xFF, 0xFF, 0xFF, 0x90],
    ),
];

static Y7S2_TOGGLES: &[(&str, u64, &[u8])] = &[
    (
        "deathless",
        0x1F2E543,
        &[
            0x6A, 0x78, 0x58, 0xF3, 0x40, 0x0F, 0x2A, 0xF0, 0x41, 0xC7, 0x85, 0xE8, 0x01, 0x00,
            0x00, 0x6A, 0x5D, 0x77, 0xCF,
        ],
    ),
    ("unlimitedAmmo", 0x266E47F, &[0x0F, 0x1F, 0x00]),
    (
        "unlimitedAmmo",
        0x267860E,
        &[0x66, 0x0F, 0x1F, 0x44, 0x00, 0x00],
    ),
    (
        "unlimitedAmmo",
        0x2678751,
        &[0x66, 0x0F, 0x1F, 0x44, 0x00, 0x00],
    ),
    ("unlimitedAmmo", 0x2685E1D, &[0x00]),
    ("unlimitedAmmo", 0x2681F0E, &[0x83]),
    ("unlimitedAmmo", 0x2682FFE, &[0x0C]),
    ("disableAI", 0x13A2670, &[0xC3]),
    ("endMatch", 0x8561E5, &[0x74, 0x3B, 0xB0, 0x01]),
    (
        "unlimitedEquip",
        0x30D1BE5,
        &[0x48, 0xC7, 0xC6, 0x06, 0x00, 0x00, 0x00],
    ),
    ("unlimitedEquip", 0x2617082, &[0x41, 0x8D, 0x44, 0x24, 0x00]),
    ("unlimitedEquip", 0x32F0419, &[0x8D, 0x47, 0x00]),
    ("unlimitedEquip", 0x261BF3B, &[0x83, 0x43, 0x44, 0x00]),
    ("unlimitedEquip", 0x172D6A0, &[0x83, 0x43, 0x44, 0x00]),
    ("unlimitedEquip", 0x32F0990, &[0x83, 0x43, 0x44, 0x00]),
    (
        "unlimitedEquip",
        0x42E60EE,
        &[
            0xBA, 0x06, 0x00, 0x00, 0x00, 0x66, 0x0F, 0x1F, 0x84, 0x00, 0x00, 0x00, 0x00, 0x00,
        ],
    ),
    (
        "unlimitedEquip",
        0x42E7B52,
        &[
            0xBA, 0x06, 0x00, 0x00, 0x00, 0x66, 0x0F, 0x1F, 0x84, 0x00, 0x00, 0x00, 0x00, 0x00,
        ],
    ),
    (
        "unlimitedEquip",
        0x1D8BA94,
        &[
            0xBA, 0x06, 0x00, 0x00, 0x00, 0x66, 0x0F, 0x1F, 0x84, 0x00, 0x00, 0x00, 0x00, 0x00,
        ],
    ),
    (
        "unlimitedEquip",
        0x407F9D3,
        &[
            0xBA, 0x06, 0x00, 0x00, 0x00, 0x66, 0x0F, 0x1F, 0x84, 0x00, 0x00, 0x00, 0x00, 0x00,
        ],
    ),
    (
        "unlimitedEquip",
        0x4085CF6,
        &[
            0xBA, 0x06, 0x00, 0x00, 0x00, 0x66, 0x0F, 0x1F, 0x84, 0x00, 0x00, 0x00, 0x00, 0x00,
        ],
    ),
    (
        "unlimitedEquip",
        0x4088676,
        &[
            0xBA, 0x06, 0x00, 0x00, 0x00, 0x66, 0x0F, 0x1F, 0x84, 0x00, 0x00, 0x00, 0x00, 0x00,
        ],
    ),
    (
        "unlimitedEquip",
        0x38B8319,
        &[
            0xBA, 0x06, 0x00, 0x00, 0x00, 0x66, 0x0F, 0x1F, 0x84, 0x00, 0x00, 0x00, 0x00, 0x00,
        ],
    ),
    (
        "unlimitedEquip",
        0x22763B0,
        &[
            0xBA, 0x06, 0x00, 0x00, 0x00, 0x66, 0x0F, 0x1F, 0x44, 0x00, 0x00,
        ],
    ),
    (
        "unlimitedEquip",
        0x30A41D0,
        &[
            0xB8, 0x06, 0x00, 0x00, 0x00, 0x0F, 0x1F, 0x80, 0x00, 0x00, 0x00, 0x00,
        ],
    ),
    (
        "unlimitedEquip",
        0x30A6D4A,
        &[
            0xBD, 0xFF, 0xFF, 0xFF, 0xFF, 0xBE, 0x06, 0x00, 0x00, 0x00, 0x0F, 0x1F, 0x44, 0x00,
            0x00,
        ],
    ),
    (
        "unlimitedEquip",
        0x42E876A,
        &[0xBD, 0xFF, 0xFF, 0xFF, 0xFF, 0x90],
    ),
];

static Y7S3_TOGGLES: &[(&str, u64, &[u8])] = &[
    (
        "deathless",
        0x3CDD73A,
        &[
            0x6A, 0x78, 0x58, 0xF3, 0x40, 0x0F, 0x2A, 0xF0, 0x40, 0xC7, 0x86, 0xEC, 0x01, 0x00,
            0x00, 0xF5, 0x4A, 0xAC, 0xE8,
        ],
    ),
    ("unlimitedAmmo", 0x6B0CB6, &[0x0F, 0x1F, 0x00]),
    (
        "unlimitedAmmo",
        0x6BD4E1,
        &[0x66, 0x0F, 0x1F, 0x44, 0x00, 0x00],
    ),
    (
        "unlimitedAmmo",
        0x6BD632,
        &[0x66, 0x0F, 0x1F, 0x44, 0x00, 0x00],
    ),
    ("unlimitedAmmo", 0x6CBBE0, &[0x00]),
    ("unlimitedAmmo", 0x6C7A07, &[0x83]),
    ("unlimitedAmmo", 0x6C8DF1, &[0x0C, 0x80]),
    ("disableAI", 0x360BB50, &[0xC3]),
    ("endMatch", 0x2C6F5EC, &[0x74, 0x44, 0xB1, 0x01]),
    (
        "unlimitedEquip",
        0x2A438F5,
        &[0x48, 0xC7, 0xC6, 0x06, 0x00, 0x00, 0x00],
    ),
    ("unlimitedEquip", 0x54888A9, &[0x8D, 0x47, 0x00]),
    ("unlimitedEquip", 0x28BF170, &[0x83, 0x43, 0x44, 0x00]),
    ("unlimitedEquip", 0x5488920, &[0x83, 0x43, 0x44, 0x00]),
    (
        "unlimitedEquip",
        0x3E23D3C,
        &[
            0xBA, 0x06, 0x00, 0x00, 0x00, 0x66, 0x0F, 0x1F, 0x84, 0x00, 0x00, 0x00, 0x00, 0x00,
        ],
    ),
    (
        "unlimitedEquip",
        0x3E25806,
        &[
            0xBA, 0x06, 0x00, 0x00, 0x00, 0x66, 0x0F, 0x1F, 0x84, 0x00, 0x00, 0x00, 0x00, 0x00,
        ],
    ),
    (
        "unlimitedEquip",
        0x161EC54,
        &[
            0xBA, 0x06, 0x00, 0x00, 0x00, 0x66, 0x0F, 0x1F, 0x84, 0x00, 0x00, 0x00, 0x00, 0x00,
        ],
    ),
    (
        "unlimitedEquip",
        0x580E004,
        &[0xBA, 0x06, 0x00, 0x00, 0x00, 0x0F, 0x1F, 0x00],
    ),
    (
        "unlimitedEquip",
        0x26505F6,
        &[
            0xBA, 0x06, 0x00, 0x00, 0x00, 0x66, 0x0F, 0x1F, 0x84, 0x00, 0x00, 0x00, 0x00, 0x00,
        ],
    ),
    (
        "unlimitedEquip",
        0x2652F06,
        &[
            0xBA, 0x06, 0x00, 0x00, 0x00, 0x66, 0x0F, 0x1F, 0x84, 0x00, 0x00, 0x00, 0x00, 0x00,
        ],
    ),
    (
        "unlimitedEquip",
        0x2669FC9,
        &[
            0xBA, 0x06, 0x00, 0x00, 0x00, 0x66, 0x0F, 0x1F, 0x84, 0x00, 0x00, 0x00, 0x00, 0x00,
        ],
    ),
    (
        "unlimitedEquip",
        0x2685340,
        &[
            0xBA, 0x06, 0x00, 0x00, 0x00, 0x66, 0x0F, 0x1F, 0x44, 0x00, 0x00,
        ],
    ),
    (
        "unlimitedEquip",
        0x54806C0,
        &[
            0xB8, 0x06, 0x00, 0x00, 0x00, 0x0F, 0x1F, 0x80, 0x00, 0x00, 0x00, 0x00,
        ],
    ),
    (
        "unlimitedEquip",
        0x54821B0,
        &[
            0xC7, 0x81, 0xC4, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0xB8, 0x06, 0x00, 0x00,
            0x00,
        ],
    ),
    (
        "unlimitedEquip",
        0x3E261AA,
        &[0xBD, 0xFF, 0xFF, 0xFF, 0xFF, 0x90],
    ),
];

static Y7S4_TOGGLES: &[(&str, u64, &[u8])] = &[
    (
        "deathless",
        0x1595C9,
        &[
            0x6A, 0x78, 0x58, 0xF3, 0x40, 0x0F, 0x2A, 0xF0, 0x41, 0xC7, 0x85, 0xD0, 0x01, 0x00,
            0x00, 0x9A, 0xB6, 0x02, 0xB4,
        ],
    ),
    ("unlimitedAmmo", 0x23D1BBC, &[0x0F, 0x1F, 0x00]),
    (
        "unlimitedAmmo",
        0x23DFB60,
        &[0x66, 0x0F, 0x1F, 0x44, 0x00, 0x00],
    ),
    (
        "unlimitedAmmo",
        0x23DFCD8,
        &[0x66, 0x0F, 0x1F, 0x44, 0x00, 0x00],
    ),
    ("unlimitedAmmo", 0x23EF01F, &[0x00]),
    ("unlimitedAmmo", 0x23EA3F4, &[0x83]),
    ("unlimitedAmmo", 0x23EB8D7, &[0x00]),
    ("disableAI", 0x146DFC0, &[0xC3]),
    ("endMatch", 0x16674EB, &[0x74, 0x45, 0xB1, 0x01]),
    (
        "unlimitedEquip",
        0xD06875,
        &[0x48, 0xC7, 0xC6, 0x06, 0x00, 0x00, 0x00],
    ),
    ("unlimitedEquip", 0x504D5C9, &[0x8D, 0x47, 0x00]),
    ("unlimitedEquip", 0x521B880, &[0x83, 0x43, 0x44, 0x00]),
    ("unlimitedEquip", 0x504D640, &[0x83, 0x43, 0x44, 0x00]),
    (
        "unlimitedEquip",
        0x4561219,
        &[
            0xBA, 0x06, 0x00, 0x00, 0x00, 0x66, 0x0F, 0x1F, 0x84, 0x00, 0x00, 0x00, 0x00, 0x00,
        ],
    ),
    (
        "unlimitedEquip",
        0x456260E,
        &[
            0xBA, 0x06, 0x00, 0x00, 0x00, 0x66, 0x0F, 0x1F, 0x84, 0x00, 0x00, 0x00, 0x00, 0x00,
        ],
    ),
    (
        "unlimitedEquip",
        0xE294E1,
        &[
            0xBA, 0x06, 0x00, 0x00, 0x00, 0x66, 0x0F, 0x1F, 0x84, 0x00, 0x00, 0x00, 0x00, 0x00,
        ],
    ),
    (
        "unlimitedEquip",
        0x5063BBB,
        &[
            0xBA, 0x06, 0x00, 0x00, 0x00, 0x66, 0x0F, 0x1F, 0x84, 0x00, 0x00, 0x00, 0x00, 0x00,
        ],
    ),
    (
        "unlimitedEquip",
        0x506AC86,
        &[
            0xBA, 0x06, 0x00, 0x00, 0x00, 0x66, 0x0F, 0x1F, 0x84, 0x00, 0x00, 0x00, 0x00, 0x00,
        ],
    ),
    (
        "unlimitedEquip",
        0x506CC96,
        &[
            0xBA, 0x06, 0x00, 0x00, 0x00, 0x66, 0x0F, 0x1F, 0x84, 0x00, 0x00, 0x00, 0x00, 0x00,
        ],
    ),
    (
        "unlimitedEquip",
        0x5084FC9,
        &[
            0xBA, 0x06, 0x00, 0x00, 0x00, 0x66, 0x0F, 0x1F, 0x84, 0x00, 0x00, 0x00, 0x00, 0x00,
        ],
    ),
    (
        "unlimitedEquip",
        0x3A931A0,
        &[
            0xBA, 0x06, 0x00, 0x00, 0x00, 0x66, 0x0F, 0x1F, 0x44, 0x00, 0x00,
        ],
    ),
    (
        "unlimitedEquip",
        0x42AC6E0,
        &[
            0xB8, 0x06, 0x00, 0x00, 0x00, 0x0F, 0x1F, 0x80, 0x00, 0x00, 0x00, 0x00,
        ],
    ),
    (
        "unlimitedEquip",
        0x5046550,
        &[
            0xC7, 0x81, 0xC4, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0xB8, 0x06, 0x00, 0x00,
            0x00,
        ],
    ),
    (
        "unlimitedEquip",
        0x456396A,
        &[0xBD, 0xFF, 0xFF, 0xFF, 0xFF, 0x90],
    ),
];

static Y8S1_TOGGLES: &[(&str, u64, &[u8])] = &[
    (
        "deathless",
        0x130B630,
        &[
            0x6A, 0x78, 0x58, 0xF3, 0x40, 0x0F, 0x2A, 0xF0, 0x40, 0xC7, 0x85, 0xB8, 0x01, 0x00,
            0x00, 0x58, 0x07, 0x49, 0xF8,
        ],
    ),
    ("unlimitedAmmo", 0x39D6CF0, &[0x0F, 0x1F, 0x00]),
    (
        "unlimitedAmmo",
        0x39E7015,
        &[0x66, 0x0F, 0x1F, 0x44, 0x00, 0x00],
    ),
    (
        "unlimitedAmmo",
        0x39E7254,
        &[0x66, 0x0F, 0x1F, 0x44, 0x00, 0x00],
    ),
    ("unlimitedAmmo", 0x3711793, &[0x66, 0x90]),
    ("unlimitedAmmo", 0x370C77C, &[0x83]),
    ("unlimitedAmmo", 0x370C770, &[0x0F, 0x1F, 0x00]),
    ("unlimitedAmmo", 0x1A2F2C0, &[0xC3]),
    ("disableAI", 0x27419A0, &[0xC3]),
    ("endMatch", 0x22B77CA, &[0x75, 0x46, 0xB1, 0x01]),
    (
        "unlimitedEquip",
        0x2423B5,
        &[0x48, 0xC7, 0xC6, 0x06, 0x00, 0x00, 0x00],
    ),
    ("unlimitedEquip", 0x1A42009, &[0x8D, 0x47, 0x00]),
    ("unlimitedEquip", 0x13E51A0, &[0x0F, 0x1F, 0x00]),
    ("unlimitedEquip", 0x1A420A0, &[0x0F, 0x1F, 0x00]),
    (
        "unlimitedEquip",
        0x430CE5A,
        &[
            0xBE, 0xFF, 0xFF, 0xFF, 0xFF, 0x31, 0xC0, 0xBF, 0x06, 0x00, 0x00, 0x00, 0x0F, 0x1F,
            0x40, 0x00,
        ],
    ),
    (
        "unlimitedEquip",
        0x430EB30,
        &[
            0x41, 0xBF, 0xFF, 0xFF, 0xFF, 0xFF, 0x31, 0xC0, 0xBF, 0x06, 0x00, 0x00, 0x00, 0x0F,
            0x1F, 0x44, 0x00, 0x00,
        ],
    ),
    (
        "unlimitedEquip",
        0x2688F39,
        &[
            0xBB, 0xFF, 0xFF, 0xFF, 0xFF, 0x31, 0xC0, 0xBE, 0x06, 0x00, 0x00, 0x00, 0x0F, 0x1F,
            0x40, 0x00,
        ],
    ),
    (
        "unlimitedEquip",
        0x2AD8CBB,
        &[
            0xBB, 0xFF, 0xFF, 0xFF, 0xFF, 0x31, 0xC0, 0xBE, 0x06, 0x00, 0x00, 0x00, 0x0F, 0x1F,
            0x40, 0x00,
        ],
    ),
    (
        "unlimitedEquip",
        0x2ADFC23,
        &[
            0x41, 0xBF, 0xFF, 0xFF, 0xFF, 0xFF, 0x31, 0xC0, 0xBF, 0x06, 0x00, 0x00, 0x00, 0x0F,
            0x1F, 0x44, 0x00, 0x00,
        ],
    ),
    (
        "unlimitedEquip",
        0x2ED2283,
        &[
            0x41, 0xBF, 0xFF, 0xFF, 0xFF, 0xFF, 0x31, 0xC0, 0xBF, 0x06, 0x00, 0x00, 0x00, 0x0F,
            0x1F, 0x44, 0x00, 0x00,
        ],
    ),
    (
        "unlimitedEquip",
        0x37AE362,
        &[
            0x41, 0xBF, 0xFF, 0xFF, 0xFF, 0xFF, 0x31, 0xC0, 0xBE, 0x06, 0x00, 0x00, 0x00, 0x0F,
            0x1F, 0x44, 0x00, 0x00,
        ],
    ),
    (
        "unlimitedEquip",
        0x2E99D5D,
        &[
            0xBD, 0xFF, 0xFF, 0xFF, 0xFF, 0x31, 0xC0, 0xBE, 0x06, 0x00, 0x00, 0x00, 0x90,
        ],
    ),
    (
        "unlimitedEquip",
        0x889E62,
        &[
            0xBD, 0xFF, 0xFF, 0xFF, 0xFF, 0x31, 0xC0, 0xBE, 0x06, 0x00, 0x00, 0x00,
        ],
    ),
    (
        "unlimitedEquip",
        0x88BCE0,
        &[
            0xC7, 0x81, 0xC4, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0xB8, 0x06, 0x00, 0x00,
            0x00,
        ],
    ),
    (
        "unlimitedEquip",
        0x430FA9A,
        &[0xBD, 0xFF, 0xFF, 0xFF, 0xFF, 0x90],
    ),
];

static Y8S2_TOGGLES: &[(&str, u64, &[u8])] = &[
    (
        "deathless",
        0x3B9ABD2,
        &[
            0x6A, 0x78, 0x58, 0xF3, 0x40, 0x0F, 0x2A, 0xF0, 0x41, 0xC7, 0x85, 0xCC, 0x01, 0x00,
            0x00, 0x69, 0x01, 0x2F, 0xEB,
        ],
    ),
    ("unlimitedAmmo", 0x2C82C1A, &[0x0F, 0x1F, 0x00]),
    (
        "unlimitedAmmo",
        0x2C91CF3,
        &[0x66, 0x0F, 0x1F, 0x44, 0x00, 0x00],
    ),
    (
        "unlimitedAmmo",
        0x2C91D97,
        &[0x66, 0x0F, 0x1F, 0x44, 0x00, 0x00],
    ),
    ("unlimitedAmmo", 0x2CA33B8, &[0x66, 0x90]),
    ("unlimitedAmmo", 0x2C9E046, &[0x83]),
    ("unlimitedAmmo", 0x2C9E225, &[0x0F, 0x1F, 0x00]),
    ("unlimitedAmmo", 0x5688750, &[0xC3]),
    ("disableAI", 0x17DADA0, &[0xC3]),
    ("endMatch", 0x26A8942, &[0x74, 0x3E, 0xB1, 0x01]),
    (
        "unlimitedEquip",
        0x1E339D3,
        &[0x48, 0xC7, 0xC7, 0x06, 0x00, 0x00, 0x00],
    ),
    ("unlimitedEquip", 0x55454B9, &[0x8D, 0x47, 0x00]),
    ("unlimitedEquip", 0x3312173, &[0x0F, 0x1F, 0x00]),
    (
        "unlimitedEquip",
        0x483A48A,
        &[
            0xBE, 0xFF, 0xFF, 0xFF, 0xFF, 0x31, 0xC0, 0xBF, 0x06, 0x00, 0x00, 0x00, 0x0F, 0x1F,
            0x40, 0x00,
        ],
    ),
    (
        "unlimitedEquip",
        0x483C14C,
        &[
            0x41, 0xBF, 0xFF, 0xFF, 0xFF, 0xFF, 0x31, 0xC0, 0xBF, 0x06, 0x00, 0x00, 0x00, 0x0F,
            0x1F, 0x44, 0x00, 0x00,
        ],
    ),
    (
        "unlimitedEquip",
        0x4806C29,
        &[
            0xBB, 0xFF, 0xFF, 0xFF, 0xFF, 0x31, 0xC0, 0xBE, 0x06, 0x00, 0x00, 0x00, 0x0F, 0x1F,
            0x40, 0x00,
        ],
    ),
    (
        "unlimitedEquip",
        0x555C3FB,
        &[
            0xBB, 0xFF, 0xFF, 0xFF, 0xFF, 0x31, 0xC0, 0xBE, 0x06, 0x00, 0x00, 0x00, 0x0F, 0x1F,
            0x40, 0x00,
        ],
    ),
    (
        "unlimitedEquip",
        0x5562D53,
        &[
            0x41, 0xBF, 0xFF, 0xFF, 0xFF, 0xFF, 0x31, 0xC0, 0xBF, 0x06, 0x00, 0x00, 0x00, 0x0F,
            0x1F, 0x44, 0x00, 0x00,
        ],
    ),
    (
        "unlimitedEquip",
        0x55658C3,
        &[
            0x41, 0xBF, 0xFF, 0xFF, 0xFF, 0xFF, 0x31, 0xC0, 0xBF, 0x06, 0x00, 0x00, 0x00, 0x0F,
            0x1F, 0x44, 0x00, 0x00,
        ],
    ),
    (
        "unlimitedEquip",
        0x557DCD2,
        &[
            0x41, 0xBF, 0xFF, 0xFF, 0xFF, 0xFF, 0x31, 0xC0, 0xBE, 0x06, 0x00, 0x00, 0x00, 0x0F,
            0x1F, 0x44, 0x00, 0x00,
        ],
    ),
    (
        "unlimitedEquip",
        0x288F99D,
        &[
            0xBD, 0xFF, 0xFF, 0xFF, 0xFF, 0x31, 0xC0, 0xBE, 0x06, 0x00, 0x00, 0x00, 0x90,
        ],
    ),
    (
        "unlimitedEquip",
        0x553B5E2,
        &[
            0xBD, 0xFF, 0xFF, 0xFF, 0xFF, 0x31, 0xC0, 0xBE, 0x06, 0x00, 0x00, 0x00,
        ],
    ),
    (
        "unlimitedEquip",
        0x553D310,
        &[
            0xC7, 0x81, 0xC8, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0xB8, 0x06, 0x00, 0x00,
            0x00,
        ],
    ),
];

fn shadow_toggles(build: &str) -> &'static [(&'static str, u64, &'static [u8])] {
    match build {
        "Y5S1_14303219" => Y5S1_TOGGLES,
        "Y6S1_15500403" => Y6S1_15500403_TOGGLES,
        "Y6S1_15680437" => Y6S1_TOGGLES,
        "Y6S2_15701375" => Y6S2_15701375_TOGGLES,
        "Y6S2_15772114" => Y6S2_TOGGLES,
        "Y6S3_32627923" => Y6S3_32627923_TOGGLES,
        "Y6S3_35130040" => Y6S3_TOGGLES,
        "Y6S4_37046065" => Y6S4_TOGGLES,
        "Y7S1_39785137" => Y7S1_TOGGLES,
        "Y7S2_43489433" => Y7S2_TOGGLES,
        "Y7S3_47165842" => Y7S3_TOGGLES,
        "Y7S4_50497889" => Y7S4_TOGGLES,
        "Y8S1_55217154" => Y8S1_TOGGLES,
        "Y8S2_58222837" => Y8S2_TOGGLES,
        _ => &[],
    }
}

const Y5_MIN_PHASE_MS: u32 = 15000;

fn y5_plain(v: u32) -> u32 {
    v
}

fn y5_plain_ptr(q: u64) -> u64 {
    q
}

fn y5s2_end(v: u32) -> u32 {
    v ^ 0xAACC7771
}

fn y5s2_duration(v: u32) -> u32 {
    v ^ 0xB5C30A45
}

fn y5s3_enc_end(v: u32) -> u32 {
    (v.rotate_right(1) ^ 0x1FCAA0F1).wrapping_add(0xCDD2E67E)
}

fn y5s3_dec_end(x: u32) -> u32 {
    x.wrapping_add(0x322D1982).rotate_left(1) ^ 0x3F9541E2
}

fn y5s3_enc_duration(v: u32) -> u32 {
    (v.wrapping_sub(0x7512A4CB) ^ 0xC0372D3A).rotate_left(7)
}

fn y5s3_dec_duration(x: u32) -> u32 {
    (x.rotate_right(7) ^ 0xC0372D3A).wrapping_add(0x7512A4CB)
}

fn y5s4_dec_gm(v: u64) -> u64 {
    v.rotate_left(31).wrapping_sub(0x4F) ^ 0x1A00B0A5BDEF13BC
}

fn y5s4_dec_state(x: u32) -> u32 {
    (x.wrapping_sub(0x76) ^ 0x6A).wrapping_add(0x40C1A09)
}

fn y5s4_enc_end(v: u32) -> u32 {
    v.wrapping_add(0x4FD387E6).rotate_left(19)
}

fn y5s4_dec_end(x: u32) -> u32 {
    x.rotate_left(13).wrapping_sub(0x4FD387E6)
}

fn y5s4_enc_duration(v: u32) -> u32 {
    v.wrapping_add(0x60).rotate_left(18).wrapping_add(0x54)
}

fn y5s4_dec_duration(x: u32) -> u32 {
    x.wrapping_sub(0x54).rotate_left(14).wrapping_sub(0x60)
}

fn y5s4_dec_list(q: u64) -> u64 {
    q.rotate_right(12).wrapping_sub(0x3F) ^ 0x965FBEC1F8C319E2
}

fn y5s4_dec_handle(q: u64) -> u64 {
    q.wrapping_add(0xC7F4D086D4D0F35E).rotate_left(60)
}

fn y6s1_dec_gm(v: u64) -> u64 {
    v.wrapping_add(0x3B842E1940CDD566)
        .rotate_left(16)
        .wrapping_sub(0x42)
}

fn y6s1_dec_state(x: u32) -> u32 {
    (x ^ 0x30).wrapping_sub(0x79).rotate_left(12)
}

fn y6s1_enc_end(v: u32) -> u32 {
    v.wrapping_sub(0x1B46A4C6)
        .rotate_left(1)
        .wrapping_add(0x3E7A8B09)
}

fn y6s1_dec_end(x: u32) -> u32 {
    x.wrapping_sub(0x3E7A8B09)
        .rotate_right(1)
        .wrapping_add(0x1B46A4C6)
}

fn y6s1_enc_duration(v: u32) -> u32 {
    (v ^ 0x4A8582E7).wrapping_add(0x60).rotate_left(29)
}

fn y6s1_dec_duration(x: u32) -> u32 {
    x.rotate_left(3).wrapping_sub(0x60) ^ 0x4A8582E7
}

fn y6s1_dec_list(q: u64) -> u64 {
    (q.wrapping_add(0x06B313D51570B92C) ^ 0x58764434C5748ED5).wrapping_add(0x426D171EC6C3525E)
}

fn y6s1_dec_handle(q: u64) -> u64 {
    q.rotate_left(57)
        .wrapping_add(0xD752A1C6BC72B9B4)
        .rotate_left(43)
}

fn y6s1_15500403_dec_gm(v: u64) -> u64 {
    (v.wrapping_add(0x60390E992B107F77) ^ 0x8427F60A304AC650).rotate_left(23)
}

fn y6s1_15500403_dec_state(x: u32) -> u32 {
    (x.rotate_left(16) ^ 0x3B).wrapping_add(0x39A03CA3)
}

fn y6s1_15500403_end(v: u32) -> u32 {
    v ^ 0x07536942
}

fn y6s1_15500403_duration(v: u32) -> u32 {
    v ^ 0x07305F6A
}

fn y6s1_15500403_dec_list(q: u64) -> u64 {
    (q ^ 0x3ABF551846123F52)
        .rotate_left(24)
        .wrapping_add(0xD05C6C8E8806669F)
}

fn y6s1_15500403_dec_handle(q: u64) -> u64 {
    q.rotate_left(40).wrapping_add(0x6CDD523938020578) ^ 0xB3E0F060399FED9A
}

fn y6s2_dec_gm(v: u64) -> u64 {
    (v ^ 0xDF9A1E6ACFE6E02C).rotate_left(20).wrapping_sub(0x15)
}

fn y6s2_dec_state(x: u32) -> u32 {
    (x.wrapping_add(0xCE9460F0) ^ 0xA63193E9).rotate_left(6)
}

fn y6s2_enc_end(v: u32) -> u32 {
    v.wrapping_add(0x7D)
}

fn y6s2_dec_end(x: u32) -> u32 {
    x.wrapping_sub(0x7D)
}

fn y6s2_enc_duration(v: u32) -> u32 {
    v.wrapping_add(0x5E).rotate_left(25) ^ 0xF9570113
}

fn y6s2_dec_duration(x: u32) -> u32 {
    (x ^ 0xF9570113).rotate_left(7).wrapping_sub(0x5E)
}

fn y6s2_dec_list(q: u64) -> u64 {
    (q.rotate_left(18) ^ 0xBF239912D7694702).wrapping_sub(0xD)
}

fn y6s2_dec_handle(q: u64) -> u64 {
    q ^ 0x0D61D1F4E77AF634
}

fn y6s2_15701375_dec_gm(v: u64) -> u64 {
    v.wrapping_add(0x77FF4DDF516EED3F)
        .rotate_left(36)
        .wrapping_sub(0x59)
}

fn y6s2_15701375_dec_state(x: u32) -> u32 {
    (x.wrapping_add(0x73C53D55) ^ 0x4C).wrapping_sub(0xE)
}

fn y6s2_15701375_enc_end(v: u32) -> u32 {
    v.rotate_left(9).wrapping_add(0xBEF7FAAA) ^ 0x98AA86A5
}

fn y6s2_15701375_dec_end(x: u32) -> u32 {
    (x ^ 0x98AA86A5).wrapping_add(0x41080556).rotate_left(23)
}

fn y6s2_15701375_enc_duration(v: u32) -> u32 {
    v.wrapping_add(0x57).rotate_left(29).wrapping_add(0x50)
}

fn y6s2_15701375_dec_duration(x: u32) -> u32 {
    x.wrapping_sub(0x50).rotate_left(3).wrapping_sub(0x57)
}

fn y6s2_15701375_dec_list(q: u64) -> u64 {
    (q.rotate_left(10) ^ 0x9).wrapping_add(0x06A84949F48B2E7C)
}

fn y6s2_15701375_dec_handle(q: u64) -> u64 {
    q.wrapping_add(0x7D0CABABCD9CA974).rotate_right(1) ^ 0x0F16A609DE2EC440
}

fn y6s3_dec_gm(v: u64) -> u64 {
    (v ^ 0xCD92F3B8EB06567D)
        .rotate_left(35)
        .wrapping_add(0xAB5724A58C0CB7DF)
}

fn y6s3_dec_state(x: u32) -> u32 {
    (x ^ 0xFC461D96).wrapping_add(0x636677AB)
}

fn y6s3_enc_end(v: u32) -> u32 {
    (v ^ 0xC6DA351C).rotate_left(11).wrapping_add(0x5804BF31)
}

fn y6s3_dec_end(x: u32) -> u32 {
    x.wrapping_add(0xA7FB40CF).rotate_left(21) ^ 0xC6DA351C
}

fn y6s3_enc_duration(v: u32) -> u32 {
    (v ^ 0x726F07F0).wrapping_add(0x2B0FB4F9) ^ 0xF6ADB83E
}

fn y6s3_dec_duration(x: u32) -> u32 {
    (x ^ 0xF6ADB83E).wrapping_add(0xD4F04B07) ^ 0x726F07F0
}

fn y6s3_dec_list(q: u64) -> u64 {
    (q.rotate_left(55) ^ 0x63ECAF13837D37CE).wrapping_sub(0x47)
}

fn y6s3_dec_handle(q: u64) -> u64 {
    q ^ 0x40E890261C57E524
}

fn y6s3_32627923_dec_gm(v: u64) -> u64 {
    (v.wrapping_sub(0x7E) ^ 0x79A56C116BE29149).rotate_left(27)
}

fn y6s3_32627923_dec_state(x: u32) -> u32 {
    (x.wrapping_sub(0x10) ^ 0x9D649583).rotate_left(1)
}

fn y6s3_32627923_end(v: u32) -> u32 {
    v ^ 0x5C089B7A
}

fn y6s3_32627923_enc_duration(v: u32) -> u32 {
    v.wrapping_add(0x26877E87).rotate_left(27)
}

fn y6s3_32627923_dec_duration(x: u32) -> u32 {
    x.rotate_left(5).wrapping_add(0xD9788179)
}

fn y6s3_32627923_dec_list(q: u64) -> u64 {
    (q ^ 0xD1B1A992532A8697)
        .rotate_left(15)
        .wrapping_add(0xB2DC771E4D41B3F5)
}

fn y6s3_32627923_dec_handle(q: u64) -> u64 {
    (q ^ 0xA07BCA1D7F136466).rotate_left(3) ^ 0xEEB133078F14E1A9
}

fn y6s4_dec_gm(v: u64) -> u64 {
    v.wrapping_add(0x708354B261C700F1).rotate_left(40) ^ 0xB03B7358A50DB0A2
}

fn y6s4_dec_state(x: u32) -> u32 {
    x ^ 0x6B328AFA
}

fn y6s4_enc_end(v: u32) -> u32 {
    v.rotate_left(6) ^ 0xC9A4B0C3
}

fn y6s4_dec_end(x: u32) -> u32 {
    (x ^ 0xC9A4B0C3).rotate_left(26)
}

fn y6s4_enc_duration(v: u32) -> u32 {
    (v ^ 0xECA1B735).rotate_left(28).wrapping_add(0x7F)
}

fn y6s4_dec_duration(x: u32) -> u32 {
    x.wrapping_sub(0x7F).rotate_left(4) ^ 0xECA1B735
}

fn y6s4_dec_list(q: u64) -> u64 {
    q.wrapping_sub(0x3C)
}

fn y6s4_dec_handle(q: u64) -> u64 {
    (q.rotate_left(18) ^ 0x622B67D13FBEC103).wrapping_add(0xACED39B0D65E9566)
}

fn y7s1_dec_gm(v: u64) -> u64 {
    (v ^ 0x5E).wrapping_sub(0x4F).rotate_left(18)
}

fn y7s1_dec_state(x: u32) -> u32 {
    x.wrapping_add(0x23EE2157)
        .rotate_left(23)
        .wrapping_add(0x88F9CAEF)
}

fn y7s1_enc_end(v: u32) -> u32 {
    (v ^ 0x8F9E0922).wrapping_add(0x6C7DCEE7) ^ 0x311BF333
}

fn y7s1_dec_end(x: u32) -> u32 {
    (x ^ 0x311BF333).wrapping_add(0x93823119) ^ 0x8F9E0922
}

fn y7s1_enc_duration(v: u32) -> u32 {
    (v ^ 0xE62FD8BC).wrapping_add(0xA0A475EB).rotate_left(25)
}

fn y7s1_dec_duration(x: u32) -> u32 {
    x.rotate_left(7).wrapping_add(0x5F5B8A15) ^ 0xE62FD8BC
}

fn y7s1_dec_list(q: u64) -> u64 {
    q.wrapping_add(0x4436F899CEB050DC).rotate_left(32)
}

fn y7s1_dec_handle(q: u64) -> u64 {
    q.wrapping_add(0x591B5BBFA4067679)
        .rotate_left(2)
        .wrapping_sub(0x5B)
}

fn y7s2_dec_gm(v: u64) -> u64 {
    v.wrapping_sub(0x24).rotate_left(26)
}

fn y7s2_dec_state(x: u32) -> u32 {
    x.rotate_left(21).wrapping_add(0x21E4D16C).rotate_left(9)
}

fn y7s2_enc_end(v: u32) -> u32 {
    v.wrapping_add(0x28).rotate_left(14) ^ 0x13864FE7
}

fn y7s2_dec_end(x: u32) -> u32 {
    (x ^ 0x13864FE7).rotate_left(18).wrapping_sub(0x28)
}

fn y7s2_enc_duration(v: u32) -> u32 {
    (v.rotate_right(1) ^ 0x57787BEF).rotate_left(2)
}

fn y7s2_dec_duration(x: u32) -> u32 {
    (x.rotate_left(30) ^ 0x57787BEF).rotate_left(1)
}

fn y7s2_dec_list(q: u64) -> u64 {
    q.rotate_left(12).wrapping_add(0x46B1E43967A4E4CE) ^ 0xE8988D4C99AFD732
}

fn y7s2_dec_handle(q: u64) -> u64 {
    q ^ 0x3CDECDFE3C7EA566
}

fn y7s3_dec_gm(x: u64) -> u64 {
    if x == 0 {
        0
    } else {
        x.wrapping_add(8)
    }
}

fn y7s3_dec_state(x: u32) -> u32 {
    (x ^ 0x76C590F0).rotate_left(10).wrapping_add(0x85D62FE7)
}

fn y7s3_enc_handle_uid(uid: u64) -> u64 {
    (uid.wrapping_add(0xFC3EE7A62D28711D) ^ 0x90A2653A7ED7DAA0).rotate_left(35)
}

fn y7s3_enc_end(v: u32) -> u32 {
    (v.wrapping_add(0x30) ^ 0x8D590FE1).wrapping_add(0x65)
}

fn y7s3_dec_end(x: u32) -> u32 {
    (x.wrapping_sub(0x65) ^ 0x8D590FE1).wrapping_sub(0x30)
}

fn y7s3_enc_duration(v: u32) -> u32 {
    (v.wrapping_add(0x15) ^ 0xFAAB2C31).wrapping_add(0x1A)
}

fn y7s3_dec_duration(x: u32) -> u32 {
    (x.wrapping_sub(0x1A) ^ 0xFAAB2C31).wrapping_sub(0x15)
}

fn y7s3_dec_list(q: u64) -> u64 {
    (q ^ 0x23).wrapping_sub(0x70).rotate_left(14)
}

fn y7s3_dec_handle(q: u64) -> u64 {
    (q ^ 0x724A6662C2E6B670).rotate_left(36)
}

fn y7s3_dec_slot(x: u64) -> u64 {
    x.rotate_left(15).wrapping_sub(0x3B).rotate_left(25)
}

fn y7s3_enc_item(v: u64) -> u64 {
    ((v ^ 0x10A1647733A3D61E).wrapping_add(0x16)) ^ 0x9D40A497E90DB751
}

fn y7s4_dec_gm(x: u64) -> u64 {
    if x == 0 {
        0
    } else {
        x.wrapping_add(0x10)
    }
}

fn y7s4_dec_state(x: u32) -> u32 {
    (x ^ 0x42E69C6C).rotate_left(18)
}

fn y7s4_enc_handle_uid(uid: u64) -> u64 {
    uid.rotate_left(55).wrapping_add(0xBA90FD188F239B6E)
}

fn y7s4_enc_end(v: u32) -> u32 {
    v.wrapping_add(0xE0D7B33A)
        .rotate_left(3)
        .wrapping_add(0x8967A3D8)
}

fn y7s4_dec_end(x: u32) -> u32 {
    x.wrapping_add(0x76985C28)
        .rotate_left(29)
        .wrapping_add(0x1F284CC6)
}

fn y7s4_enc_duration(v: u32) -> u32 {
    v.wrapping_add(0x7A)
}

fn y7s4_dec_duration(x: u32) -> u32 {
    x.wrapping_sub(0x7A)
}

fn y7s4_dec_list(q: u64) -> u64 {
    (q ^ 0x1E).wrapping_add(0x8140EA14CFF4006F)
}

fn y7s4_dec_count(q: u64) -> u64 {
    (q ^ 0x5B31E33AC7F4EA1B).wrapping_add(0xAA8DD94F63A91886)
}

fn y7s4_dec_handle(q: u64) -> u64 {
    q.rotate_left(21).wrapping_sub(0x33) ^ 0xBE00ED5BAC29A003
}

fn y7s4_dec_slot(x: u64) -> u64 {
    (x ^ 0x7A).wrapping_sub(0x1D).rotate_left(33)
}

fn y7s4_enc_item(v: u64) -> u64 {
    v ^ 0xCFA094E994ACCCBE
}

fn y8s1_dec_gm(x: u64) -> u64 {
    if x == 0 {
        0
    } else {
        x.wrapping_add(0x10)
    }
}

fn y8s1_dec_state(x: u32) -> u32 {
    x.wrapping_sub(0x6E)
        .rotate_left(23)
        .wrapping_add(0xFA1F920E)
}

fn y8s1_enc_handle_uid(uid: u64) -> u64 {
    uid.wrapping_add(0x479E8221442A91C9)
        .rotate_left(38)
        .wrapping_add(0xEC9874F04997D517)
}

fn y8s1_dec_settings(q: u64) -> u64 {
    q.wrapping_sub(0x5D)
}

fn y8s1_dec_settings_count(q: u64) -> u64 {
    q.wrapping_sub(0x5008375CE05B0795)
}

fn y8s1_enc_end(v: u32) -> u32 {
    (v ^ 0x9329A760).wrapping_add(0xE88748AA) ^ 0x204D7BF3
}

fn y8s1_dec_end(x: u32) -> u32 {
    (x ^ 0x204D7BF3).wrapping_sub(0xE88748AA) ^ 0x9329A760
}

fn y8s1_enc_duration(v: u32) -> u32 {
    v.wrapping_add(0x35)
        .rotate_left(26)
        .wrapping_add(0x35345A25)
}

fn y8s1_dec_duration(x: u32) -> u32 {
    x.wrapping_sub(0x35345A25).rotate_left(6).wrapping_sub(0x35)
}

fn y8s1_dec_list(q: u64) -> u64 {
    (q ^ 0x7B6FBDDDBC265A4A).wrapping_sub(0x76AFB4F2C670DE8D) ^ 0x8EFD9204A8213FAD
}

fn y8s1_dec_count(q: u64) -> u64 {
    (q ^ 0xB72B1697C94168E0).wrapping_sub(0x351F9475639BF2F1) ^ 0x413623F15870D5A4
}

fn y8s1_dec_handle(q: u64) -> u64 {
    (q.wrapping_sub(0x50FFCA8838A7AFDD) ^ 0x80F207DADEFC9DB8).rotate_left(8)
}

fn y8s1_dec_loadout(q: u64) -> u64 {
    (q ^ 0xE2AD727973B77D75)
        .rotate_left(32)
        .wrapping_add(0x81736E8996CDE9D6)
}

fn y8s1_dec_slot(x: u64) -> u64 {
    (x.wrapping_add(0x79A19EC8AB7EFC54) ^ 0x3C).wrapping_sub(0x5E)
}

fn y8s1_enc_item(v: u64) -> u64 {
    v.wrapping_add(0x1A)
        .rotate_left(20)
        .wrapping_add(0x389CC85538690568)
}

fn y8s2_dec_gm(x: u64) -> u64 {
    if x == 0 {
        0
    } else {
        x.wrapping_add(0x18)
    }
}

fn y8s2_dec_state(x: u32) -> u32 {
    x.wrapping_sub(0x3B)
}

fn y8s2_enc_handle_uid(uid: u64) -> u64 {
    uid.wrapping_add(0x2A).rotate_left(16) ^ 0x90FF4A8882C54932
}

fn y8s2_dec_settings(q: u64) -> u64 {
    (q.wrapping_sub(0x41) ^ 0x961012CEB770F3B6).wrapping_add(0x8037D080A6DDADD2)
}

fn y8s2_dec_settings_count(q: u64) -> u64 {
    (q.wrapping_sub(0x491A5BD694BE410C) ^ 0xE19310E5C75B305F).wrapping_add(0xC164841C13B3BADA)
}

fn y8s2_enc_end(v: u32) -> u32 {
    v.rotate_left(29).wrapping_add(0x586CDD48) ^ 0x76
}

fn y8s2_dec_end(x: u32) -> u32 {
    (x ^ 0x76).wrapping_add(0xA79322B8).rotate_left(3)
}

fn y8s2_enc_duration(v: u32) -> u32 {
    (v ^ 0x997ACC59).rotate_left(19).wrapping_add(0xB9AD1116)
}

fn y8s2_dec_duration(x: u32) -> u32 {
    x.wrapping_add(0x4652EEEA).rotate_left(13) ^ 0x997ACC59
}

fn y8s2_dec_list(q: u64) -> u64 {
    (q ^ 0xAE9138B5A3AE338E).wrapping_sub(0x4E) ^ 0x8C042D12BE38F332
}

fn y8s2_dec_count(q: u64) -> u64 {
    (q ^ 0x3550CE76D26A5841).wrapping_sub(0xFDB597B7C9B9DDBA) ^ 0x04EE305B49CF77D5
}

fn y8s2_dec_handle(q: u64) -> u64 {
    q.wrapping_sub(0x5C).rotate_left(30) ^ 0x808BC6D358AB9DAA
}

fn y8s2_dec_loadout(q: u64) -> u64 {
    (q.rotate_left(12) ^ 0x56).wrapping_sub(0x61)
}

fn y8s2_dec_slot(x: u64) -> u64 {
    x.rotate_left(44).wrapping_add(0x454C9224844B3E89)
}

fn y8s2_enc_item(v: u64) -> u64 {
    v.rotate_left(21)
        .wrapping_add(0x0A3AE558DADC7D8C)
        .rotate_left(21)
}

struct Y5Phase {
    start: u32,
    duration: u32,
    end: u32,
    at: u32,
}

static Y5_MULTIPLAYER: &[(&str, u64)] = &[
    ("Bomb", 0x138BDF6E),
    ("Bomb - No Prep Phase", 0x25A11B0495),
    ("Hostage", 0xA934BDF6),
    ("Secure Area", 0x76337AA1),
    ("Warmup", 0x7A956D02F),
];
static Y5_TERRORIST_HUNT: &[(&str, u64)] = &[
    ("Disarm Bomb", 0x2036D0C6E),
    ("Elimination", 0x58A02A93),
    ("Extract Hostage", 0x67FDB2DB),
    ("Protect Hostage", 0x58A02A91),
];
static Y5_DIFFICULTIES: &[(&str, u64)] = &[
    ("Normal", 0xCDE271EC),
    ("Hard", 0xCDE271ED),
    ("Realistic", 0xCDE271EE),
];
const Y5_GYM: u64 = 0x72AFC423;

type Y5Event = (&'static str, u64, u64, &'static [usize]);
type Y5HeadCodec = [fn(u64) -> u64; 2];

enum Y5Roster {
    Plain {
        list: i32,
        handle: i32,
    },
    Encoded {
        lists: [u64; 2],
        dec_head: Y5HeadCodec,
        handles: &'static [u64],
        dec_handle: fn(u64) -> u64,
        session_handles: Option<(u64, [u64; 2], &'static [u64])>,
    },
}

struct Y5Round {
    live: (u64, u32),
    running: (u64, u8),
    countdown: (u64, u8),
    match_over: (u64, u8),
    now: u64,
    start: u64,
    end: u64,
    duration: u64,
    infinite_display: Option<(u64, u8, u8)>,
    enc_end: fn(u32) -> u32,
    dec_end: fn(u32) -> u32,
    enc_duration: fn(u32) -> u32,
    dec_duration: fn(u32) -> u32,
    roster: Y5Roster,
    loadout: i32,
    dec_loadout: Option<fn(u64) -> u64>,
    slots: [i32; 4],
    slot: Option<Y5Slot>,
}

struct Y5Slot {
    dec: fn(u64) -> u64,
    field: u64,
    item: u64,
    enc_item: fn(u64) -> u64,
}

struct Y5MatchLayout {
    difficulty: u64,
    time_of_day: u64,
    settings: u64,
}

const Y5_MATCH_LAYOUT: Y5MatchLayout = Y5MatchLayout {
    difficulty: 0x30,
    time_of_day: 0x38,
    settings: 0x48,
};

const Y7_MATCH_LAYOUT: Y5MatchLayout = Y5MatchLayout {
    difficulty: 0x38,
    time_of_day: 0x40,
    settings: 0x50,
};

struct Y5Game {
    tag: &'static str,
    game_manager: u64,
    dec_gm: Option<fn(u64) -> u64>,
    state: u64,
    dec_state: Option<fn(u32) -> u32>,
    role: u64,
    match_state: &'static [i32],
    match_layout: Y5MatchLayout,
    gamemode: u64,
    settings: u64,
    dec_settings: Option<Y5HeadCodec>,
    time_of_day: u64,
    enc_handle_uid: Option<fn(u64) -> u64>,
    round: Option<Y5Round>,
    maps: &'static [(&'static str, u64)],
    standard_maps: usize,
    thunt_maps: &'static [usize],
    events: &'static [Y5Event],
}

static Y5S1_GAME: Y5Game = Y5Game {
    tag: "y5s1",
    game_manager: 0x53A6868,
    dec_gm: None,
    state: 0xF0,
    dec_state: None,
    role: 0x230,
    match_state: &[0x90],
    match_layout: Y5_MATCH_LAYOUT,
    gamemode: 0x3A94168,
    settings: 0x70,
    dec_settings: None,
    time_of_day: 0x20,
    enc_handle_uid: None,
    round: Some(Y5Round {
        live: (0x1F4, 1),
        running: (0x998, 1),
        countdown: (0x999, 0),
        match_over: (0x99F, 1),
        now: 0x1678,
        start: 0x944,
        end: 0x948,
        duration: 0x950,
        infinite_display: None,
        enc_end: y5_plain,
        dec_end: y5_plain,
        enc_duration: y5_plain,
        dec_duration: y5_plain,
        roster: Y5Roster::Encoded {
            lists: [0x300, 0x4D8],
            dec_head: [y5_plain_ptr; 2],
            handles: &[0x98],
            dec_handle: y5_plain_ptr,
            session_handles: None,
        },
        loadout: 0x40,
        dec_loadout: None,
        slots: Y5_SLOTS,
        slot: None,
    }),
    maps: &[
        ("House", 0x331CFEA6),
        ("Oregon", 0x35F2901CF4),
        ("Hereford Base", 0x2A3A99CB),
        ("Hereford Base - Rework", 0x1DCA7A2258),
        ("Club House", 0x31E6DF85),
        ("Presidential Plane", 0x9B858528),
        ("Yacht", 0x6961015C),
        ("Consulate", 0x9B858E7A),
        ("Bank", 0x9B858706),
        ("Kanal", 0x570932C9),
        ("Chalet", 0x9B859057),
        ("Bartlett University", 0xA0C50B9A),
        ("Kafe Dostoyevsky", 0x522587EA),
        ("Border", 0xCE4BB240),
        ("Favela", 0x77B43597E),
        ("Skyscraper", 0x8AF15096F),
        ("Coastline", 0x9CCC3D997),
        ("Theme Park", 0x2E8679C826),
        ("Tower", 0xC7C6E5654),
        ("Villa", 0x14839B2B18),
        ("Fortress", 0x1D61EAFB8F),
        ("Outback", 0x2296804EC5),
        ("Headquarters", 0x3E005881AF),
    ],
    standard_maps: 22,
    thunt_maps: &[
        0, 1, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17, 18, 19, 20, 21,
    ],
    events: &[
        ("Grand Larceny", 0x39E871FC8A, 0x4024D5CD3C, &[22]),
        ("Golden Gun", 0x3EA695B452, 0x3F331EF4E2, &[1]),
    ],
};

static Y5S2_GAME: Y5Game = Y5Game {
    tag: "y5s2",
    game_manager: 0x603DD10,
    dec_gm: None,
    state: 0x114,
    dec_state: None,
    role: 0x278,
    match_state: &[0xF8],
    match_layout: Y5_MATCH_LAYOUT,
    gamemode: 0x4C8AD70,
    settings: 0x70,
    dec_settings: None,
    time_of_day: 0x20,
    enc_handle_uid: None,
    round: Some(Y5Round {
        live: (0x234, 1),
        running: (0x5E8, 0),
        countdown: (0x670, 0),
        match_over: (0x97C, 1),
        now: 0x16C8,
        start: 0xA60,
        end: 0x600,
        duration: 0x970,
        infinite_display: None,
        enc_end: y5s2_end,
        dec_end: y5s2_end,
        enc_duration: y5s2_duration,
        dec_duration: y5s2_duration,
        roster: Y5Roster::Plain {
            list: 0x3C0,
            handle: 0xA8,
        },
        loadout: 0x40,
        dec_loadout: None,
        slots: Y5_SLOTS,
        slot: None,
    }),
    maps: &[
        ("House", 0x37625C3900),
        ("Oregon", 0x35F2901CF4),
        ("Hereford Base", 0x2A3A99CB),
        ("Hereford Base - Rework", 0x1DCA7A2258),
        ("Club House", 0x31E6DF85),
        ("Presidential Plane", 0x9B858528),
        ("Yacht", 0x6961015C),
        ("Consulate", 0x9B858E7A),
        ("Bank", 0x9B858706),
        ("Kanal", 0x570932C9),
        ("Chalet", 0x9B859057),
        ("Bartlett University", 0xA0C50B9A),
        ("Kafe Dostoyevsky", 0x522587EA),
        ("Border", 0xCE4BB240),
        ("Favela", 0x77B43597E),
        ("Skyscraper", 0x8AF15096F),
        ("Coastline", 0x9CCC3D997),
        ("Theme Park", 0x2E8679C826),
        ("Tower", 0xC7C6E5654),
        ("Villa", 0x14839B2B18),
        ("Fortress", 0x1D61EAFB8F),
        ("Outback", 0x2296804EC5),
        ("M.U.T.E. Protocol", 0x40DC810C48),
    ],
    standard_maps: 22,
    thunt_maps: &[
        0, 1, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17, 18, 19, 20, 21,
    ],
    events: &[("M.U.T.E. Protocol", 0x41E157DDE5, 0x41E15761D1, &[22])],
};

static Y5S3_GAME: Y5Game = Y5Game {
    tag: "y5s3",
    game_manager: 0x6E3FF30,
    dec_gm: None,
    state: 0x50,
    dec_state: None,
    role: 0x308,
    match_state: &[0x68],
    match_layout: Y5_MATCH_LAYOUT,
    gamemode: 0x534D4B0,
    settings: 0x68,
    dec_settings: None,
    time_of_day: 0x40,
    enc_handle_uid: None,
    round: Some(Y5Round {
        live: (0x2CC, 1),
        running: (0x418, 2),
        countdown: (0x874, 0x5E),
        match_over: (0xB68, 0x4C),
        now: 0x5118,
        start: 0x6BC,
        end: 0x7A4,
        duration: 0x7F0,
        infinite_display: None,
        enc_end: y5s3_enc_end,
        dec_end: y5s3_dec_end,
        enc_duration: y5s3_enc_duration,
        dec_duration: y5s3_dec_duration,
        roster: Y5Roster::Plain {
            list: 0x8C0,
            handle: 0xA8,
        },
        loadout: 0x38,
        dec_loadout: None,
        slots: Y5_SLOTS,
        slot: None,
    }),
    maps: &[
        ("Oregon", 0x35F2901CF4),
        ("Hereford Base", 0x2A3A99CB),
        ("Hereford Base - Rework", 0x1DCA7A2258),
        ("Club House", 0x31E6DF85),
        ("Presidential Plane", 0x9B858528),
        ("Yacht", 0x6961015C),
        ("Consulate", 0x9B858E7A),
        ("Bank", 0x9B858706),
        ("Kanal", 0x570932C9),
        ("Chalet", 0x3C7E4A5A5D),
        ("Bartlett University", 0xA0C50B9A),
        ("Kafe Dostoyevsky", 0x522587EA),
        ("Border", 0xCE4BB240),
        ("Favela", 0x77B43597E),
        ("Skyscraper", 0x8AF15096F),
        ("Coastline", 0x9CCC3D997),
        ("Theme Park", 0x2E8679C826),
        ("Tower", 0xC7C6E5654),
        ("Villa", 0x14839B2B18),
        ("Fortress", 0x1D61EAFB8F),
        ("Outback", 0x2296804EC5),
        ("House", 0x37625C3900),
        ("Neighborhood", 0x431BA741C3),
    ],
    standard_maps: 22,
    thunt_maps: &[
        0, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17, 18, 19, 20, 21,
    ],
    events: &[
        ("Golden Gun", 0x3EA695B452, 0x3F331EF4E2, &[12, 21, 15]),
        ("Sugar Fright", 0x45840C11F0, 0x45840EDB69, &[22]),
    ],
};

static Y5S4_GAME: Y5Game = Y5Game {
    tag: "y5s4",
    game_manager: 0x6E85930,
    dec_gm: Some(y5s4_dec_gm),
    state: 0xB8,
    dec_state: Some(y5s4_dec_state),
    role: 0x348,
    match_state: &[0xC8],
    match_layout: Y5_MATCH_LAYOUT,
    gamemode: 0x53A7BA0,
    settings: 0x68,
    dec_settings: None,
    time_of_day: 0x40,
    enc_handle_uid: None,
    round: Some(Y5Round {
        live: (0x30C, 1),
        running: (0x438, 0x12),
        countdown: (0x9F0, 0x8A),
        match_over: (0x478, 0x3F),
        now: 0x51E8,
        start: 0xB88,
        end: 0xA08,
        duration: 0xAE4,
        infinite_display: None,
        enc_end: y5s4_enc_end,
        dec_end: y5s4_dec_end,
        enc_duration: y5s4_enc_duration,
        dec_duration: y5s4_dec_duration,
        roster: Y5Roster::Encoded {
            lists: [0x4F8, 0x780],
            dec_head: [y5s4_dec_list; 2],
            handles: &[0x158],
            dec_handle: y5s4_dec_handle,
            session_handles: None,
        },
        loadout: 0x38,
        dec_loadout: None,
        slots: Y5_SLOTS,
        slot: None,
    }),
    maps: &[
        ("Oregon", 0x35F2901CF4),
        ("Hereford Base", 0x2A3A99CB),
        ("Hereford Base - Rework", 0x1DCA7A2258),
        ("Club House", 0x31E6DF85),
        ("Presidential Plane", 0x9B858528),
        ("Yacht", 0x6961015C),
        ("Consulate", 0x9B858E7A),
        ("Bank", 0x9B858706),
        ("Kanal", 0x570932C9),
        ("Chalet", 0x3C7E4A5A5D),
        ("Bartlett University", 0xA0C50B9A),
        ("Kafe Dostoyevsky", 0x522587EA),
        ("Border", 0xCE4BB240),
        ("Favela", 0x77B43597E),
        ("Skyscraper", 0x4053835E1E),
        ("Coastline", 0x9CCC3D997),
        ("Theme Park", 0x2E8679C826),
        ("Tower", 0xC7C6E5654),
        ("Villa", 0x14839B2B18),
        ("Fortress", 0x1D61EAFB8F),
        ("Outback", 0x2296804EC5),
        ("House", 0x37625C3900),
        ("Stadium", 0x3EE107746E),
    ],
    standard_maps: 22,
    thunt_maps: &[
        0, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17, 18, 19, 20, 21,
    ],
    events: &[("Road To S.I. 2021", 0x483AB3E121, 0x4707B87CF1, &[22])],
};

static Y6S1_GAME: Y5Game = Y5Game {
    tag: "y6s1",
    game_manager: 0x70D88C8,
    dec_gm: Some(y6s1_dec_gm),
    state: 0x184,
    dec_state: Some(y6s1_dec_state),
    role: 0x340,
    match_state: &[0xA0],
    match_layout: Y5_MATCH_LAYOUT,
    gamemode: 0x55444F0,
    settings: 0x68,
    dec_settings: None,
    time_of_day: 0x40,
    enc_handle_uid: None,
    round: Some(Y5Round {
        live: (0x304, 1),
        running: (0x738, 0x96),
        countdown: (0x9A0, 0x36),
        match_over: (0xC18, 0xB7),
        now: 0x51C8,
        start: 0xADC,
        end: 0xAC0,
        duration: 0xB70,
        infinite_display: Some((0xA81, 0x36, 0x2E)),
        enc_end: y6s1_enc_end,
        dec_end: y6s1_dec_end,
        enc_duration: y6s1_enc_duration,
        dec_duration: y6s1_dec_duration,
        roster: Y5Roster::Encoded {
            lists: [0x420, 0x770],
            dec_head: [y6s1_dec_list; 2],
            handles: &[0xB0],
            dec_handle: y6s1_dec_handle,
            session_handles: None,
        },
        loadout: 0x38,
        dec_loadout: None,
        slots: Y5_SLOTS,
        slot: None,
    }),
    maps: &[
        ("Oregon", 0x35F2901CF4),
        ("Hereford Base", 0x2A3A99CB),
        ("Hereford Base - Rework", 0x1DCA7A2258),
        ("Club House", 0x31E6DF85),
        ("Presidential Plane", 0x9B858528),
        ("Yacht", 0x6961015C),
        ("Consulate", 0x9B858E7A),
        ("Bank", 0x9B858706),
        ("Kanal", 0x570932C9),
        ("Chalet", 0x3C7E4A5A5D),
        ("Bartlett University", 0xA0C50B9A),
        ("Kafe Dostoyevsky", 0x522587EA),
        ("Border", 0x473DCA77EF),
        ("Favela", 0x77B43597E),
        ("Skyscraper", 0x4053835E1E),
        ("Coastline", 0x9CCC3D997),
        ("Theme Park", 0x2E8679C826),
        ("Tower", 0xC7C6E5654),
        ("Villa", 0x14839B2B18),
        ("Fortress", 0x1D61EAFB8F),
        ("Outback", 0x2296804EC5),
        ("House", 0x37625C3900),
        ("Rainbow is Magic", 0x2BCB159A9C),
        ("Apocalypse", 0x50183207D0),
    ],
    standard_maps: 22,
    thunt_maps: &[
        0, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17, 18, 19, 20, 21,
    ],
    events: &[
        ("Rainbow is Magic", 0x4DB61E967D, 0x4DB61E9851, &[22]),
        ("Apocalypse", 0x530F7BDC93, 0x530F7BED20, &[23]),
    ],
};

static Y6S1_15500403_GAME: Y5Game = Y5Game {
    tag: "y6s1",
    game_manager: 0x7106998,
    dec_gm: Some(y6s1_15500403_dec_gm),
    state: 0x170,
    dec_state: Some(y6s1_15500403_dec_state),
    role: 0x338,
    match_state: &[0x100],
    match_layout: Y5_MATCH_LAYOUT,
    gamemode: 0x5520760,
    settings: 0x68,
    dec_settings: None,
    time_of_day: 0x40,
    enc_handle_uid: None,
    round: Some(Y5Round {
        live: (0x2FC, 1),
        running: (0xB10, 2),
        countdown: (0x3E8, 0x95),
        match_over: (0xC24, 0xF4),
        now: 0x51A8,
        start: 0xA70,
        end: 0xAE4,
        duration: 0x9A0,
        infinite_display: Some((0x3E9, 0x9F, 0x8F)),
        enc_end: y6s1_15500403_end,
        dec_end: y6s1_15500403_end,
        enc_duration: y6s1_15500403_duration,
        dec_duration: y6s1_15500403_duration,
        roster: Y5Roster::Encoded {
            lists: [0x668, 0x418],
            dec_head: [y6s1_15500403_dec_list; 2],
            handles: &[0x180],
            dec_handle: y6s1_15500403_dec_handle,
            session_handles: None,
        },
        loadout: 0x38,
        dec_loadout: None,
        slots: Y5_SLOTS,
        slot: None,
    }),
    ..Y6S1_GAME
};

static Y6S2_GAME: Y5Game = Y5Game {
    tag: "y6s2",
    game_manager: 0x729A598,
    dec_gm: Some(y6s2_dec_gm),
    state: 0x150,
    dec_state: Some(y6s2_dec_state),
    role: 0x358,
    match_state: &[0xD0],
    match_layout: Y5_MATCH_LAYOUT,
    gamemode: 0x56FCF90,
    settings: 0x68,
    dec_settings: None,
    time_of_day: 0x40,
    enc_handle_uid: None,
    round: Some(Y5Round {
        live: (0x31C, 1),
        running: (0x488, 0xEE),
        countdown: (0xA61, 0x42),
        match_over: (0x800, 0xFF),
        now: 0x6050,
        start: 0x928,
        end: 0x490,
        duration: 0x7D8,
        infinite_display: Some((0x7F0, 0xD8, 0xD5)),
        enc_end: y6s2_enc_end,
        dec_end: y6s2_dec_end,
        enc_duration: y6s2_enc_duration,
        dec_duration: y6s2_dec_duration,
        roster: Y5Roster::Encoded {
            lists: [0x500, 0xB00],
            dec_head: [y6s2_dec_list; 2],
            handles: &[0x48],
            dec_handle: y6s2_dec_handle,
            session_handles: None,
        },
        loadout: 0x38,
        dec_loadout: None,
        slots: Y5_SLOTS,
        slot: None,
    }),
    maps: &[
        ("Oregon", 0x35F2901CF4),
        ("Hereford Base", 0x2A3A99CB),
        ("Hereford Base - Rework", 0x1DCA7A2258),
        ("Club House", 0x31E6DF85),
        ("Presidential Plane", 0x9B858528),
        ("Yacht", 0x6961015C),
        ("Consulate", 0x9B858E7A),
        ("Bank", 0x9B858706),
        ("Kanal", 0x570932C9),
        ("Chalet", 0x3C7E4A5A5D),
        ("Bartlett University", 0xA0C50B9A),
        ("Kafe Dostoyevsky", 0x522587EA),
        ("Border", 0x473DCA77EF),
        ("Favela", 0x4CCD9FE066),
        ("Skyscraper", 0x4053835E1E),
        ("Coastline", 0x9CCC3D997),
        ("Theme Park", 0x2E8679C826),
        ("Tower", 0xC7C6E5654),
        ("Villa", 0x14839B2B18),
        ("Fortress", 0x1D61EAFB8F),
        ("Outback", 0x2296804EC5),
        ("House", 0x37625C3900),
        ("Containment", 0x5464BBF809),
    ],
    standard_maps: 22,
    thunt_maps: &[
        0, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17, 18, 19, 20, 21,
    ],
    events: &[("Containment", 0x547CD662FD, 0x54A95068E8, &[22])],
};

static Y6S2_15701375_GAME: Y5Game = Y5Game {
    tag: "y6s2",
    game_manager: 0x72A43C8,
    dec_gm: Some(y6s2_15701375_dec_gm),
    state: 0xE0,
    dec_state: Some(y6s2_15701375_dec_state),
    role: 0x358,
    match_state: &[0x118],
    match_layout: Y5_MATCH_LAYOUT,
    gamemode: 0x5643B20,
    settings: 0x68,
    dec_settings: None,
    time_of_day: 0x40,
    enc_handle_uid: None,
    round: Some(Y5Round {
        live: (0x31C, 1),
        running: (0xC82, 0xA8),
        countdown: (0x4EC, 0xA4),
        match_over: (0xC38, 0x91),
        now: 0x6038,
        start: 0x848,
        end: 0x87C,
        duration: 0xCE0,
        infinite_display: Some((0x490, 0xA3, 0x83)),
        enc_end: y6s2_15701375_enc_end,
        dec_end: y6s2_15701375_dec_end,
        enc_duration: y6s2_15701375_enc_duration,
        dec_duration: y6s2_15701375_dec_duration,
        roster: Y5Roster::Encoded {
            lists: [0x928, 0x5B8],
            dec_head: [y6s2_15701375_dec_list; 2],
            handles: &[0x1B0],
            dec_handle: y6s2_15701375_dec_handle,
            session_handles: None,
        },
        loadout: 0x38,
        dec_loadout: None,
        slots: Y5_SLOTS,
        slot: None,
    }),
    events: &[],
    ..Y6S2_GAME
};

static Y6S3_GAME: Y5Game = Y5Game {
    tag: "y6s3",
    game_manager: 0x75AE938,
    dec_gm: Some(y6s3_dec_gm),
    state: 0x158,
    dec_state: Some(y6s3_dec_state),
    role: 0x338,
    match_state: &[0xD0],
    match_layout: Y5_MATCH_LAYOUT,
    gamemode: 0x5B9F930,
    settings: 0x68,
    dec_settings: None,
    time_of_day: 0x40,
    enc_handle_uid: None,
    round: Some(Y5Round {
        live: (0x2FC, 1),
        running: (0xC30, 0xF2),
        countdown: (0xC33, 0x29),
        match_over: (0xC2B, 0x8F),
        now: 0x5B28,
        start: 0xBC4,
        end: 0xBFC,
        duration: 0xBDC,
        infinite_display: Some((0xC2C, 0x99, 0x19)),
        enc_end: y6s3_enc_end,
        dec_end: y6s3_dec_end,
        enc_duration: y6s3_enc_duration,
        dec_duration: y6s3_dec_duration,
        roster: Y5Roster::Encoded {
            lists: [0x660, 0x9A8],
            dec_head: [y6s3_dec_list; 2],
            handles: &[0x70],
            dec_handle: y6s3_dec_handle,
            session_handles: None,
        },
        loadout: 0x38,
        dec_loadout: None,
        slots: Y5_SLOTS,
        slot: None,
    }),
    maps: &[
        ("Oregon", 0x35F2901CF4),
        ("Hereford Base", 0x2A3A99CB),
        ("Hereford Base - Rework", 0x1DCA7A2258),
        ("Club House", 0x31E6DF85),
        ("Presidential Plane", 0x9B858528),
        ("Yacht", 0x6961015C),
        ("Consulate", 0x9B858E7A),
        ("Bank", 0x52C53F0506),
        ("Kanal", 0x570932C9),
        ("Chalet", 0x3C7E4A5A5D),
        ("Bartlett University", 0xA0C50B9A),
        ("Kafe Dostoyevsky", 0x522587EA),
        ("Border", 0x473DCA77EF),
        ("Favela", 0x4CCD9FE066),
        ("Skyscraper", 0x4053835E1E),
        ("Coastline", 0x9CCC3D997),
        ("Theme Park", 0x2E8679C826),
        ("Tower", 0xC7C6E5654),
        ("Villa", 0x14839B2B18),
        ("Fortress", 0x1D61EAFB8F),
        ("Outback", 0x2296804EC5),
        ("House", 0x37625C3900),
        ("Fort Truth", 0x2F83769722),
        ("Doktor's Curse", 0xB3555E074),
    ],
    standard_maps: 22,
    thunt_maps: &[
        0, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17, 18, 19, 20, 21,
    ],
    events: &[
        ("Showdown", 0x30B6346015, 0x4DB61E9852, &[22]),
        ("Doktor's Curse", 0x55CDE1F7AE, 0x55CDE1FAB7, &[23]),
    ],
};

static Y6S3_32627923_GAME: Y5Game = Y5Game {
    tag: "y6s3",
    game_manager: 0x72C3B28,
    dec_gm: Some(y6s3_32627923_dec_gm),
    state: 0x15C,
    dec_state: Some(y6s3_32627923_dec_state),
    role: 0x338,
    match_state: &[0xD0],
    match_layout: Y5_MATCH_LAYOUT,
    gamemode: 0x5959F00,
    settings: 0x68,
    dec_settings: None,
    time_of_day: 0x40,
    enc_handle_uid: None,
    round: Some(Y5Round {
        live: (0x2FC, 1),
        running: (0xC28, 0x82),
        countdown: (0xC2A, 0x4B),
        match_over: (0xC35, 0x41),
        now: 0x5B28,
        start: 0xBB0,
        end: 0xC08,
        duration: 0xBD0,
        infinite_display: Some((0xC2F, 0x18, 0x19)),
        enc_end: y6s3_32627923_end,
        dec_end: y6s3_32627923_end,
        enc_duration: y6s3_32627923_enc_duration,
        dec_duration: y6s3_32627923_dec_duration,
        roster: Y5Roster::Encoded {
            lists: [0x428, 0x858],
            dec_head: [y6s3_32627923_dec_list; 2],
            handles: &[0x50],
            dec_handle: y6s3_32627923_dec_handle,
            session_handles: None,
        },
        loadout: 0x38,
        dec_loadout: None,
        slots: Y5_SLOTS,
        slot: None,
    }),
    events: &[("Showdown", 0x30B6346015, 0x4DB61E9852, &[22])],
    ..Y6S3_GAME
};

static Y6S4_GAME: Y5Game = Y5Game {
    tag: "y6s4",
    game_manager: 0x790A308,
    dec_gm: Some(y6s4_dec_gm),
    state: 0x158,
    dec_state: Some(y6s4_dec_state),
    role: 0x380,
    match_state: &[0x28, 0x50, 0x48],
    match_layout: Y5_MATCH_LAYOUT,
    gamemode: 0x56C33E0,
    settings: 0x68,
    dec_settings: None,
    time_of_day: 0x40,
    enc_handle_uid: None,
    round: Some(Y5Round {
        live: (0x344, 1),
        running: (0xD5C, 0x54),
        countdown: (0xD5D, 0x48),
        match_over: (0xD54, 0x75),
        now: 0x5D58,
        start: 0xCF0,
        end: 0xCF4,
        duration: 0xCFC,
        infinite_display: Some((0xD52, 0x2C, 0x28)),
        enc_end: y6s4_enc_end,
        dec_end: y6s4_dec_end,
        enc_duration: y6s4_enc_duration,
        dec_duration: y6s4_dec_duration,
        roster: Y5Roster::Encoded {
            lists: [0x540, 0x8E8],
            dec_head: [y6s4_dec_list; 2],
            handles: &[0x908],
            dec_handle: y6s4_dec_handle,
            session_handles: Some((0x63B71E8, [0x68, 0x2EC], &[0x438, 0x150])),
        },
        loadout: 0x38,
        dec_loadout: None,
        slots: Y5_SLOTS,
        slot: None,
    }),
    maps: &[
        ("Oregon", 0x35F2901CF4),
        ("Hereford Base", 0x2A3A99CB),
        ("Hereford Base - Rework", 0x1DCA7A2258),
        ("Club House", 0x31E6DF85),
        ("Presidential Plane", 0x9B858528),
        ("Yacht", 0x6961015C),
        ("Consulate", 0x9B858E7A),
        ("Bank", 0x52C53F0506),
        ("Kanal", 0x570932C9),
        ("Chalet", 0x3C7E4A5A5D),
        ("Bartlett University", 0xA0C50B9A),
        ("Kafe Dostoyevsky", 0x522587EA),
        ("Border", 0x473DCA77EF),
        ("Favela", 0x4CCD9FE066),
        ("Skyscraper", 0x4053835E1E),
        ("Coastline", 0x9CCC3D997),
        ("Theme Park", 0x2E8679C826),
        ("Tower", 0xC7C6E5654),
        ("Villa", 0x14839B2B18),
        ("Fortress", 0x1D61EAFB8F),
        ("Outback", 0x546CF2DD4F),
        ("House", 0x37625C3900),
        ("Snow Brawl", 0x5617C235CE),
        ("Stadium Alpha", 0x2D1D494EF2),
        ("Stadium Bravo", 0x3EE107746E),
    ],
    standard_maps: 22,
    thunt_maps: &[
        0, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17, 18, 19, 20, 21,
    ],
    events: &[
        ("Snow Brawl", 0x5797ECFED9, 0x4A9198FBF7, &[22]),
        ("Road To S.I. 2022", 0x57490FFCEA, 0x4DA02B54A4, &[23]),
        ("Road To S.I. 2022", 0x57490FFCEA, 0x5763873C39, &[24]),
    ],
};

static Y7S1_GAME: Y5Game = Y5Game {
    tag: "y7s1",
    game_manager: 0x7C91C48,
    dec_gm: Some(y7s1_dec_gm),
    state: 0x15C,
    dec_state: Some(y7s1_dec_state),
    role: 0x380,
    match_state: &[0x70],
    match_layout: Y7_MATCH_LAYOUT,
    gamemode: 0x64CB688,
    settings: 0x68,
    dec_settings: None,
    time_of_day: 0x40,
    enc_handle_uid: None,
    round: Some(Y5Round {
        live: (0x344, 1),
        running: (0xD64, 0xF3),
        countdown: (0xD67, 0x95),
        match_over: (0xD5C, 0xE4),
        now: 0x6040,
        start: 0xD14,
        end: 0xCD8,
        duration: 0xD0C,
        infinite_display: Some((0xD62, 0xA9, 0xA8)),
        enc_end: y7s1_enc_end,
        dec_end: y7s1_dec_end,
        enc_duration: y7s1_enc_duration,
        dec_duration: y7s1_dec_duration,
        roster: Y5Roster::Encoded {
            lists: [0x650, 0xA40],
            dec_head: [y7s1_dec_list; 2],
            handles: &[0x910],
            dec_handle: y7s1_dec_handle,
            session_handles: Some((0x6C8E3B8, [0x68, 0x2E2], &[0xD8, 0x3C0])),
        },
        loadout: 0x38,
        dec_loadout: None,
        slots: Y5_SLOTS,
        slot: None,
    }),
    maps: &[
        ("Oregon", 0x35F2901CF4),
        ("Hereford Base", 0x2A3A99CB),
        ("Hereford Base - Rework", 0x1DCA7A2258),
        ("Club House", 0x31E6DF85),
        ("Presidential Plane", 0x9B858528),
        ("Yacht", 0x6961015C),
        ("Consulate", 0x9B858E7A),
        ("Bank", 0x52C53F0506),
        ("Kanal", 0x570932C9),
        ("Chalet", 0x3C7E4A5A5D),
        ("Bartlett University", 0xA0C50B9A),
        ("Kafe Dostoyevsky", 0x522587EA),
        ("Border", 0x473DCA77EF),
        ("Favela", 0x4CCD9FE066),
        ("Skyscraper", 0x4053835E1E),
        ("Coastline", 0x9CCC3D997),
        ("Theme Park", 0x2E8679C826),
        ("Tower", 0xC7C6E5654),
        ("Villa", 0x14839B2B18),
        ("Fortress", 0x1D61EAFB8F),
        ("Outback", 0x546CF2DD4F),
        ("House", 0x37625C3900),
        ("Emerald Plains", 0x550CA6FED4),
        ("Rengoku", 0x585E25818B),
    ],
    standard_maps: 23,
    thunt_maps: &[
        0, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17, 18, 19, 20, 21, 22,
    ],
    events: &[("Rengoku", 0x585B44DA5C, 0x585B4583CD, &[23])],
};

static Y7S2_GAME: Y5Game = Y5Game {
    tag: "y7s2",
    game_manager: 0x81F7A60,
    dec_gm: Some(y7s2_dec_gm),
    state: 0x15C,
    dec_state: Some(y7s2_dec_state),
    role: 0x380,
    match_state: &[0xE0],
    match_layout: Y7_MATCH_LAYOUT,
    gamemode: 0x61C55C8,
    settings: 0x68,
    dec_settings: None,
    time_of_day: 0x40,
    enc_handle_uid: None,
    round: Some(Y5Round {
        live: (0x344, 1),
        running: (0xD7D, 0x68),
        countdown: (0xD83, 0x75),
        match_over: (0xD76, 0x2A),
        now: 0x64C8,
        start: 0xD14,
        end: 0xD00,
        duration: 0xD08,
        infinite_display: Some((0xD7B, 0x0B, 0x6B)),
        enc_end: y7s2_enc_end,
        dec_end: y7s2_dec_end,
        enc_duration: y7s2_enc_duration,
        dec_duration: y7s2_dec_duration,
        roster: Y5Roster::Encoded {
            lists: [0x500, 0xA08],
            dec_head: [y7s2_dec_list; 2],
            handles: &[0x3F8, 0x110],
            dec_handle: y7s2_dec_handle,
            session_handles: None,
        },
        loadout: 0x38,
        dec_loadout: None,
        slots: Y5_SLOTS,
        slot: None,
    }),
    maps: &[
        ("Oregon", 0x35F2901CF4),
        ("Hereford Base", 0x2A3A99CB),
        ("Hereford Base - Rework", 0x1DCA7A2258),
        ("Club House", 0x31E6DF85),
        ("Presidential Plane", 0x9B858528),
        ("Yacht", 0x6961015C),
        ("Consulate", 0x9B858E7A),
        ("Bank", 0x52C53F0506),
        ("Kanal", 0x570932C9),
        ("Chalet", 0x3C7E4A5A5D),
        ("Bartlett University", 0xA0C50B9A),
        ("Kafe Dostoyevsky", 0x522587EA),
        ("Border", 0x473DCA77EF),
        ("Favela", 0x4CCD9FE066),
        ("Skyscraper", 0x4053835E1E),
        ("Coastline", 0x9CCC3D997),
        ("Theme Park", 0x2E8679C826),
        ("Tower", 0xC7C6E5654),
        ("Villa", 0x14839B2B18),
        ("Fortress", 0x1D61EAFB8F),
        ("Outback", 0x546CF2DD4F),
        ("House", 0x37625C3900),
        ("Emerald Plains", 0x550CA6FED4),
        ("M.U.T.E. Protocol", 0x40DC810C48),
    ],
    standard_maps: 23,
    thunt_maps: &[
        0, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17, 18, 19, 20, 21, 22,
    ],
    events: &[("M.U.T.E. Protocol", 0x59E5A4411F, 0x58CE8D5DB2, &[23])],
};

static Y7S3_GAME: Y5Game = Y5Game {
    tag: "y7s3",
    game_manager: 0x7E00080,
    dec_gm: Some(y7s3_dec_gm),
    state: 0x158,
    dec_state: Some(y7s3_dec_state),
    role: 0x390,
    match_state: &[0xE8],
    match_layout: Y7_MATCH_LAYOUT,
    gamemode: 0x655DAC8,
    settings: 0x68,
    dec_settings: None,
    time_of_day: 0x40,
    enc_handle_uid: Some(y7s3_enc_handle_uid),
    round: Some(Y5Round {
        live: (0x344, 1),
        running: (0xED1, 0x36),
        countdown: (0xEC4, 0xFB),
        match_over: (0xEC9, 0xEA),
        now: 0x66F8,
        start: 0xE48,
        end: 0xE84,
        duration: 0xE4C,
        infinite_display: Some((0xEC7, 0xFC, 0xF4)),
        enc_end: y7s3_enc_end,
        dec_end: y7s3_dec_end,
        enc_duration: y7s3_enc_duration,
        dec_duration: y7s3_dec_duration,
        roster: Y5Roster::Encoded {
            lists: [0x5F0, 0x958],
            dec_head: [y7s3_dec_list; 2],
            handles: &[0x370, 0x88],
            dec_handle: y7s3_dec_handle,
            session_handles: None,
        },
        loadout: 0x38,
        dec_loadout: None,
        slots: Y5_SLOTS,
        slot: Some(Y5Slot {
            dec: y7s3_dec_slot,
            field: 0x20,
            item: 0x20,
            enc_item: y7s3_enc_item,
        }),
    }),
    maps: &[
        ("Oregon", 0x35F2901CF4),
        ("Hereford Base", 0x2A3A99CB),
        ("Hereford Base - Rework", 0x1DCA7A2258),
        ("Club House", 0x31E6DF85),
        ("Presidential Plane", 0x9B858528),
        ("Yacht", 0x6961015C),
        ("Consulate", 0x9B858E7A),
        ("Bank", 0x52C53F0506),
        ("Kanal", 0x570932C9),
        ("Chalet", 0x3C7E4A5A5D),
        ("Bartlett University", 0xA0C50B9A),
        ("Kafe Dostoyevsky", 0x522587EA),
        ("Border", 0x473DCA77EF),
        ("Favela", 0x4CCD9FE066),
        ("Skyscraper", 0x4053835E1E),
        ("Coastline", 0x9CCC3D997),
        ("Theme Park", 0x2E8679C826),
        ("Tower", 0xC7C6E5654),
        ("Villa", 0x14839B2B18),
        ("Fortress", 0x1D61EAFB8F),
        ("Outback", 0x546CF2DD4F),
        ("House", 0x37625C3900),
        ("Emerald Plains", 0x550CA6FED4),
        ("Stadium Bravo", 0x3EE107746E),
        ("Fort Truth", 0x2F83769722),
        ("Doktor's Curse", 0xB3555E074),
    ],
    standard_maps: 24,
    thunt_maps: &[
        0, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17, 18, 19, 20, 21, 22,
    ],
    events: &[
        ("Snipers", 0x5A4EE6FC73, 0x5A4EE6FC65, &[2, 24, 9]),
        ("Doktor's Curse", 0x55CDE1F7AE, 0x55CDE1FAB7, &[25]),
    ],
};

static Y7S4_GAME: Y5Game = Y5Game {
    tag: "y7s4",
    game_manager: 0x7D00580,
    dec_gm: Some(y7s4_dec_gm),
    state: 0x15C,
    dec_state: Some(y7s4_dec_state),
    role: 0x390,
    match_state: &[0x100],
    match_layout: Y7_MATCH_LAYOUT,
    gamemode: 0x68EB5E0,
    settings: 0x68,
    dec_settings: None,
    time_of_day: 0x40,
    enc_handle_uid: Some(y7s4_enc_handle_uid),
    round: Some(Y5Round {
        live: (0x344, 1),
        running: (0xEC8, 0x2A),
        countdown: (0xECF, 0xCF),
        match_over: (0xEC9, 0x9D),
        now: 0x66F8,
        start: 0xEAC,
        end: 0xEA4,
        duration: 0xEA8,
        infinite_display: Some((0xED2, 0xCE, 0xAD)),
        enc_end: y7s4_enc_end,
        dec_end: y7s4_dec_end,
        enc_duration: y7s4_enc_duration,
        dec_duration: y7s4_dec_duration,
        roster: Y5Roster::Encoded {
            lists: [0x678, 0xA40],
            dec_head: [y7s4_dec_list, y7s4_dec_count],
            handles: &[0x110, 0x450],
            dec_handle: y7s4_dec_handle,
            session_handles: None,
        },
        loadout: 0x38,
        dec_loadout: None,
        slots: [0x88, 0x70, 0x58, 0x40],
        slot: Some(Y5Slot {
            dec: y7s4_dec_slot,
            field: 0x20,
            item: 0x18,
            enc_item: y7s4_enc_item,
        }),
    }),
    maps: &[
        ("Oregon", 0x35F2901CF4),
        ("Hereford Base", 0x2A3A99CB),
        ("Hereford Base - Rework", 0x1DCA7A2258),
        ("Club House", 0x31E6DF85),
        ("Presidential Plane", 0x9B858528),
        ("Yacht", 0x6961015C),
        ("Consulate", 0x9B858E7A),
        ("Bank", 0x52C53F0506),
        ("Kanal", 0x570932C9),
        ("Chalet", 0x3C7E4A5A5D),
        ("Bartlett University", 0xA0C50B9A),
        ("Kafe Dostoyevsky", 0x522587EA),
        ("Border", 0x473DCA77EF),
        ("Favela", 0x4CCD9FE066),
        ("Skyscraper", 0x4053835E1E),
        ("Coastline", 0x9CCC3D997),
        ("Theme Park", 0x2E8679C826),
        ("Tower", 0xC7C6E5654),
        ("Villa", 0x14839B2B18),
        ("Fortress", 0x1D61EAFB8F),
        ("Outback", 0x546CF2DD4F),
        ("House", 0x37625C3900),
        ("Emerald Plains", 0x550CA6FED4),
        ("Stadium Bravo", 0x3EE107746E),
        ("Nighthaven Labs", 0x58260EEFB3),
        ("Snow Brawl", 0x5617C235CE),
    ],
    standard_maps: 25,
    thunt_maps: &[
        0, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17, 18, 19, 20, 21, 22, 24,
    ],
    events: &[("Snow Brawl", 0x5797ECFED9, 0x4A9198FBF7, &[25])],
};

static Y8S1_GAME: Y5Game = Y5Game {
    tag: "y8s1",
    game_manager: 0x7927E10,
    dec_gm: Some(y8s1_dec_gm),
    state: 0x1C4,
    dec_state: Some(y8s1_dec_state),
    role: 0x3B8,
    match_state: &[0x60],
    match_layout: Y7_MATCH_LAYOUT,
    gamemode: 0x6585BB0,
    settings: 0x70,
    dec_settings: Some([y8s1_dec_settings, y8s1_dec_settings_count]),
    time_of_day: 0x40,
    enc_handle_uid: Some(y8s1_enc_handle_uid),
    round: Some(Y5Round {
        live: (0x1B8, 0xCFBCDB01),
        running: (0xF57, 0x17),
        countdown: (0xF4E, 0x86),
        match_over: (0xF4F, 0x76),
        now: 0x5F08,
        start: 0xEEC,
        end: 0xEF4,
        duration: 0xF48,
        infinite_display: Some((0xF4C, 0xA3, 0x93)),
        enc_end: y8s1_enc_end,
        dec_end: y8s1_dec_end,
        enc_duration: y8s1_enc_duration,
        dec_duration: y8s1_dec_duration,
        roster: Y5Roster::Encoded {
            lists: [0x440, 0x908],
            dec_head: [y8s1_dec_list, y8s1_dec_count],
            handles: &[0x4D0, 0x1A0],
            dec_handle: y8s1_dec_handle,
            session_handles: None,
        },
        loadout: 0x18,
        dec_loadout: Some(y8s1_dec_loadout),
        slots: [0x08, 0x48, 0x20, 0x80],
        slot: Some(Y5Slot {
            dec: y8s1_dec_slot,
            field: 0x28,
            item: 0x30,
            enc_item: y8s1_enc_item,
        }),
    }),
    maps: &[
        ("Oregon", 0x35F2901CF4),
        ("Hereford Base", 0x2A3A99CB),
        ("Hereford Base - Rework", 0x1DCA7A2258),
        ("Club House", 0x31E6DF85),
        ("Presidential Plane", 0x9B858528),
        ("Yacht", 0x6961015C),
        ("Consulate", 0x9B858E7A),
        ("Bank", 0x52C53F0506),
        ("Kanal", 0x570932C9),
        ("Chalet", 0x3C7E4A5A5D),
        ("Bartlett University", 0xA0C50B9A),
        ("Kafe Dostoyevsky", 0x522587EA),
        ("Border", 0x473DCA77EF),
        ("Favela", 0x4CCD9FE066),
        ("Skyscraper", 0x4053835E1E),
        ("Coastline", 0x9CCC3D997),
        ("Theme Park", 0x2E8679C826),
        ("Tower", 0xC7C6E5654),
        ("Villa", 0x14839B2B18),
        ("Fortress", 0x1D61EAFB8F),
        ("Outback", 0x546CF2DD4F),
        ("House", 0x37625C3900),
        ("Emerald Plains", 0x550CA6FED4),
        ("Stadium Bravo", 0x3EE107746E),
        ("Nighthaven Labs", 0x58260EEFB3),
        ("Rainbow is Magic", 0x2BCB159A9C),
    ],
    standard_maps: 25,
    thunt_maps: &[
        0, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17, 18, 19, 20, 21, 22, 24,
    ],
    events: &[("Rainbow is Magic", 0x5B55A9D54B, 0x4DB61E9851, &[25])],
};

static Y8S2_GAME: Y5Game = Y5Game {
    tag: "y8s2",
    game_manager: 0x70BD170,
    dec_gm: Some(y8s2_dec_gm),
    state: 0x1C0,
    dec_state: Some(y8s2_dec_state),
    role: 0x3B8,
    match_state: &[0x70],
    match_layout: Y7_MATCH_LAYOUT,
    gamemode: 0x6745B80,
    settings: 0x60,
    dec_settings: Some([y8s2_dec_settings, y8s2_dec_settings_count]),
    time_of_day: 0x20,
    enc_handle_uid: Some(y8s2_enc_handle_uid),
    round: Some(Y5Round {
        live: (0x1C4, 0x7A43955D),
        running: (0xF60, 0x21),
        countdown: (0xF69, 0x59),
        match_over: (0xF62, 0x9A),
        now: 0x5F30,
        start: 0xEE0,
        end: 0xEE4,
        duration: 0xF14,
        infinite_display: Some((0xF65, 0xEE, 0x0E)),
        enc_end: y8s2_enc_end,
        dec_end: y8s2_dec_end,
        enc_duration: y8s2_enc_duration,
        dec_duration: y8s2_dec_duration,
        roster: Y5Roster::Encoded {
            lists: [0x9F0, 0x428],
            dec_head: [y8s2_dec_list, y8s2_dec_count],
            handles: &[0xA0, 0x3D8],
            dec_handle: y8s2_dec_handle,
            session_handles: None,
        },
        loadout: 0x28,
        dec_loadout: Some(y8s2_dec_loadout),
        slots: [0x40, 0x80, 0x60, 0x70],
        slot: Some(Y5Slot {
            dec: y8s2_dec_slot,
            field: 0x10,
            item: 0x18,
            enc_item: y8s2_enc_item,
        }),
    }),
    maps: &[
        ("Oregon", 0x35F2901CF4),
        ("Hereford Base", 0x2A3A99CB),
        ("Hereford Base - Rework", 0x1DCA7A2258),
        ("Club House", 0x31E6DF85),
        ("Presidential Plane", 0x9B858528),
        ("Yacht", 0x6961015C),
        ("Consulate", 0x584B31FC7D),
        ("Bank", 0x52C53F0506),
        ("Kanal", 0x570932C9),
        ("Chalet", 0x3C7E4A5A5D),
        ("Bartlett University", 0xA0C50B9A),
        ("Kafe Dostoyevsky", 0x522587EA),
        ("Border", 0x473DCA77EF),
        ("Favela", 0x4CCD9FE066),
        ("Skyscraper", 0x4053835E1E),
        ("Coastline", 0x9CCC3D997),
        ("Theme Park", 0x2E8679C826),
        ("Tower", 0xC7C6E5654),
        ("Villa", 0x14839B2B18),
        ("Fortress", 0x1D61EAFB8F),
        ("Outback", 0x546CF2DD4F),
        ("House", 0x37625C3900),
        ("Emerald Plains", 0x550CA6FED4),
        ("Stadium Bravo", 0x3EE107746E),
        ("Nighthaven Labs", 0x58260EEFB3),
        ("Rengoku", 0x585E25818B),
    ],
    standard_maps: 25,
    thunt_maps: &[
        0, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17, 18, 19, 20, 21, 22, 24,
    ],
    events: &[("Rengoku", 0x585B44DA5C, 0x585B4583CD, &[25])],
};

fn y5_game(build: &str) -> Option<&'static Y5Game> {
    match build {
        "Y5S1_14303219" => Some(&Y5S1_GAME),
        "Y5S2_14664131" => Some(&Y5S2_GAME),
        "Y5S3_15018155" => Some(&Y5S3_GAME),
        "Y5S4_15241382" => Some(&Y5S4_GAME),
        "Y6S1_15500403" => Some(&Y6S1_15500403_GAME),
        "Y6S1_15680437" => Some(&Y6S1_GAME),
        "Y6S2_15701375" => Some(&Y6S2_15701375_GAME),
        "Y6S2_15772114" => Some(&Y6S2_GAME),
        "Y6S3_32627923" => Some(&Y6S3_32627923_GAME),
        "Y6S3_35130040" => Some(&Y6S3_GAME),
        "Y6S4_37046065" => Some(&Y6S4_GAME),
        "Y7S1_39785137" => Some(&Y7S1_GAME),
        "Y7S2_43489433" => Some(&Y7S2_GAME),
        "Y7S3_47165842" => Some(&Y7S3_GAME),
        "Y7S4_50497889" => Some(&Y7S4_GAME),
        "Y8S1_55217154" => Some(&Y8S1_GAME),
        "Y8S2_58222837" => Some(&Y8S2_GAME),
        _ => None,
    }
}

#[derive(Clone, Copy)]
struct Y5Pick {
    mode: u64,
    map: u64,
    difficulty: u64,
    playlist: u64,
}

struct Y5Lobby {
    m: u64,
    playlist: u64,
    tod_at: u64,
    tod: u64,
    entries: Vec<u64>,
}

fn parse_y5_pick(game: &Y5Game, id: &str) -> Option<Y5Pick> {
    let parts: Vec<&str> = id.split(':').collect();
    if parts.first() != Some(&game.tag) {
        return None;
    }
    let n = |i: usize| parts.get(i)?.parse::<usize>().ok();
    let map = |i: usize| Some(game.maps.get(n(i)?)?.1);
    let plain = |modes: &[(&str, u64)], difficulty: u64| {
        Some(Y5Pick {
            mode: modes.get(n(2)?)?.1,
            map: map(3)?,
            difficulty,
            playlist: 0,
        })
    };
    match *parts.get(1)? {
        "mp" => plain(Y5_MULTIPLAYER, 0),
        "dev" => Some(Y5Pick {
            mode: Y5_GYM,
            map: map(2)?,
            difficulty: 0,
            playlist: 0,
        }),
        "th" => plain(Y5_TERRORIST_HUNT, Y5_DIFFICULTIES.get(n(4)?)?.1),
        "ev" => {
            let (_, mode, playlist, _) = game.events.get(n(2)?)?;
            Some(Y5Pick {
                mode: *mode,
                map: map(3)?,
                difficulty: 0,
                playlist: *playlist,
            })
        }
        _ => None,
    }
}

fn build_y5_tree_json(game: &Y5Game) -> String {
    let tag = game.tag;
    let maps = |prefix: String| {
        (0..game.standard_maps)
            .map(|map| leaf(game.maps[map].0, format!("{prefix}:{map}")))
            .collect::<Vec<_>>()
    };
    let multiplayer = Y5_MULTIPLAYER
        .iter()
        .enumerate()
        .map(|(m, (name, _))| branch(name, maps(format!("{tag}:mp:{m}"))))
        .collect();
    let gym = maps(format!("{tag}:dev"));
    let hunt = Y5_TERRORIST_HUNT
        .iter()
        .enumerate()
        .map(|(m, (name, _))| {
            let maps = game
                .thunt_maps
                .iter()
                .map(|&map| {
                    let levels = Y5_DIFFICULTIES
                        .iter()
                        .enumerate()
                        .map(|(d, (level, _))| leaf(level, format!("{tag}:th:{m}:{map}:{d}")))
                        .collect();
                    branch(game.maps[map].0, levels)
                })
                .collect();
            branch(name, maps)
        })
        .collect();
    let mut events: Vec<TNode> = Vec::new();
    for (e, (name, _, _, maps)) in game.events.iter().enumerate() {
        let leaves = maps
            .iter()
            .map(|&map| leaf(game.maps[map].0, format!("{tag}:ev:{e}:{map}")));
        match events.iter_mut().find(|node| node.text == *name) {
            Some(group) => group.children.extend(leaves),
            None => events.push(branch(name, leaves.collect())),
        }
    }
    for node in &mut events {
        if node.children.len() == 1 {
            node.id = node.children.remove(0).id;
        }
    }
    let mut roots = vec![
        branch("Multiplayer", multiplayer),
        branch("Terrorist Hunt", hunt),
        branch("Development", gym),
    ];
    if !events.is_empty() {
        roots.push(branch("Events", events));
    }
    tree_event(&mut roots)
}

pub struct Runner {
    eng: Engine,
    client: TcpStream,
    build: &'static str,
    patched: bool,
    pending: bool,
    countdown: i32,
    unlocked: bool,
    unsupported: bool,
    season: i32,
    disable_primary: bool,
    disable_secondary: bool,
    gametype: u64,
    y5_disable: [bool; Y5_DISABLE_SLOTS.len()],
    y5_infinite: bool,
    y5_phase: Option<Y5Phase>,
    y5_match_end: bool,
    y5_pick: Option<Y5Pick>,
    y5_handles: HashMap<u64, u64>,
    y5_playlists: HashMap<u64, u64>,
    y5_scanned: u64,
    y5_lobby: Option<Y5Lobby>,
    lastsig: String,
    tree_sent: bool,
    scanned: String,
    available: [bool; CAPS.len()],
    ownership: Ownership,
    onboarding: Onboarding,
    badges: Badges,
}

impl Runner {
    fn new(client: TcpStream) -> Runner {
        Runner {
            eng: Engine::default(),
            client,
            build: "",
            patched: false,
            pending: true,
            countdown: 0,
            unlocked: false,
            unsupported: false,
            season: -1,
            disable_primary: false,
            disable_secondary: false,
            gametype: 0,
            y5_disable: [false; Y5_DISABLE_SLOTS.len()],
            y5_infinite: false,
            y5_phase: None,
            y5_match_end: false,
            y5_pick: None,
            y5_handles: HashMap::new(),
            y5_playlists: HashMap::new(),
            y5_scanned: 0,
            y5_lobby: None,
            lastsig: String::new(),
            tree_sent: false,
            scanned: String::new(),
            available: [false; CAPS.len()],
            ownership: Ownership::default(),
            onboarding: Onboarding::default(),
            badges: Badges::default(),
        }
    }

    fn send_line(&self, s: &str) {
        let _ = (&self.client).write_all(format!("{s}\n").as_bytes());
    }

    fn apply_static(&self, feature: &str, branch: &str) {
        for p in PATCHES {
            if p.feature != feature || p.build != self.build || p.branch != branch {
                continue;
            }
            self.eng
                .write_mem(self.eng.base.wrapping_add(p.addr), p.bytes);
        }
    }

    fn is_full_feature(&self) -> bool {
        (0..SEASON_Y5S1).contains(&self.season) || y5_game(self.build).is_some()
    }

    fn y5_deref(&self, start: u64, offsets: &[i32]) -> u64 {
        let mut a = start;
        for off in offsets {
            if a == 0 {
                return 0;
            }
            a = self.eng.read_ptr(a.wrapping_add(*off as i64 as u64));
        }
        a
    }

    fn y5_container(&self, round: &Y5Round, loadout: u64) -> u64 {
        match round.dec_loadout {
            Some(dec) if loadout != 0 => dec(self
                .eng
                .read_u64(loadout.wrapping_add(round.loadout as u64))),
            Some(_) => 0,
            None => self.y5_deref(loadout, &[round.loadout]),
        }
    }

    fn apply_y5_disable(&self) {
        if !self.y5_disable.iter().any(|on| *on) {
            return;
        }
        let round = self.y5_round();
        let containers: Vec<u64> = match (self.season, round) {
            (_, Some((game, round))) => {
                let gm = self.y5_gm(game);
                if !(5..=7).contains(&self.y5_state(game, gm)) {
                    return;
                }
                match round.roster {
                    Y5Roster::Plain { list, handle } => (-5..5)
                        .map(|player| {
                            self.y5_container(
                                round,
                                self.y5_deref(gm, &[list, player * 8, 0x00, handle, 0x00]),
                            )
                        })
                        .collect(),
                    Y5Roster::Encoded {
                        lists,
                        dec_head,
                        handles,
                        dec_handle,
                        session_handles,
                    } => {
                        let handles = session_handles
                            .filter(|(config, flags, _)| {
                                let host = self.eng.read_ptr(self.eng.base + config);
                                host != 0 && flags.iter().all(|f| self.eng.read1(host + f) != 0)
                            })
                            .map_or(handles, |(_, _, session)| session);
                        lists
                            .iter()
                            .flat_map(|&at| {
                                let mut head = [0u8; 16];
                                self.eng.read_mem(gm + at, &mut head);
                                let (words, _) = head.as_chunks::<8>();
                                let data = dec_head[0](u64::from_le_bytes(words[0]));
                                let count = dec_head[1](u64::from_le_bytes(words[1])) & 0x3FFF_FFFF;
                                (0..count.min(16)).map(move |i| data + i * 8)
                            })
                            .map(|entry| self.y5_deref(entry, &[0x00, 0x00]))
                            .filter(|&pawn| pawn != 0)
                            .flat_map(|pawn| {
                                handles.iter().map(move |at| {
                                    self.y5_container(
                                        round,
                                        self.y5_deref(
                                            dec_handle(self.eng.read_u64(pawn + at)),
                                            &[0x00],
                                        ),
                                    )
                                })
                            })
                            .collect()
                    }
                }
            }
            _ => return,
        };
        let round = round.map(|(_, round)| round);
        let slots = round.map_or(Y5_SLOTS, |round| round.slots);
        let slot = round.and_then(|round| round.slot.as_ref());
        let (item, empty) = slot.map_or((0x18, 0), |slot| (slot.item, (slot.enc_item)(0)));
        for container in containers {
            if container == 0 {
                continue;
            }
            for (i, at) in slots.iter().enumerate() {
                if !self.y5_disable[i] {
                    continue;
                }
                let child = self.y5_deref(container, &[*at]);
                let entry = match slot {
                    Some(slot) if child != 0 => {
                        self.y5_deref((slot.dec)(self.eng.read_u64(child + slot.field)), &[0x00])
                    }
                    Some(_) => 0,
                    None => self.y5_deref(child, &[0x20, 0x00]),
                };
                if entry != 0 {
                    self.eng.write_mem(entry + item, &empty.to_le_bytes());
                }
            }
        }
    }

    fn shadow_toggle(&self, mod_: &str, enabled: bool) -> bool {
        let mut matched = false;
        for (name, off, on) in shadow_toggles(self.build) {
            if mod_ != *name {
                continue;
            }
            matched = true;
            if enabled {
                self.eng.shadow_write(*off, on);
            } else if let Some(regions) = shadow_regions_for_build(self.build) {
                if let Some(r) = regions.iter().find(|r| r.offset == *off) {
                    self.eng.shadow_write(*off, r.patch);
                }
            }
        }
        matched
    }

    fn tree_json(&self) -> String {
        match (self.season, y5_game(self.build)) {
            (_, Some(game)) => build_y5_tree_json(game),
            _ => self.build_tree_json(),
        }
    }

    fn write_chain(&self, feature: &str, bytes: &[u8]) {
        if let Some(fc) = find_chain(feature, self.build) {
            let a = self.eng.resolve_ptr_chain(fc.base_offset, fc.offsets);
            self.eng.write_mem(a, bytes);
        }
    }

    fn end_round(&self) {
        if !self.patched {
            return;
        }
        if let Some((game, round)) = self.y5_round() {
            let gm = self.y5_host(game);
            if self.y5_live(game, round, gm) && self.y5_flag(gm, round.running) {
                self.y5_time_out(round, gm);
            }
        } else if !self.write_state("EndRound") {
            self.write_chain("EndRound", &[1]);
        }
    }

    fn end_match(&mut self) {
        if !self.patched {
            return;
        }
        if let Some((game, round)) = self.y5_round() {
            if self.y5_live(game, round, self.y5_host(game)) {
                self.end_match_patch(true);
                self.y5_match_end = true;
                self.apply_y5_match_end();
            }
        } else if !self.write_state("EndMatch") {
            self.write_chain("EndMatch", &[1]);
            self.write_chain("EndMatchTrigger", &[1]);
        }
    }

    fn end_match_patch(&self, enabled: bool) {
        if !self.shadow_toggle("endMatch", enabled) {
            self.apply_static("EndMatch", if enabled { "enable" } else { "disable" });
        }
    }

    fn y5_round(&self) -> Option<(&'static Y5Game, &'static Y5Round)> {
        y5_game(self.build).and_then(|game| game.round.as_ref().map(|round| (game, round)))
    }

    fn y5_gm(&self, game: &Y5Game) -> u64 {
        let at = self.eng.base + game.game_manager;
        match game.dec_gm {
            Some(decode) => decode(self.eng.read_u64(at)),
            None => self.eng.read_ptr(at),
        }
    }

    fn y5_host(&self, game: &Y5Game) -> u64 {
        let gm = self.y5_gm(game);
        if gm != 0 && self.eng.read_u32(gm + game.role) <= 1 {
            gm
        } else {
            0
        }
    }

    fn y5_state(&self, game: &Y5Game, gm: u64) -> u32 {
        let raw = self.eng.read_u32(gm + game.state);
        game.dec_state.map_or(raw, |decode| decode(raw))
    }

    fn y5_flag(&self, gm: u64, (offset, value): (u64, u8)) -> bool {
        self.eng.read1(gm + offset) == value
    }

    fn y5_live(&self, game: &Y5Game, round: &Y5Round, gm: u64) -> bool {
        gm != 0
            && self.y5_state(game, gm) == 7
            && self.eng.read_u32(gm + round.live.0) == round.live.1
    }

    fn apply_y5_match_end(&mut self) {
        let Some((game, round)) = self.y5_round() else {
            return;
        };
        let gm = self.y5_host(game);
        if !self.y5_live(game, round, gm) || self.y5_flag(gm, round.match_over) {
            self.end_match_patch(false);
            self.y5_match_end = false;
        } else if self.y5_flag(gm, round.running) {
            self.y5_time_out(round, gm);
        }
    }

    fn y5_time_out(&self, round: &Y5Round, gm: u64) {
        let now = self.eng.read_u32(gm + round.now);
        if self.eng.read_u32(gm + round.duration) == (round.enc_duration)(0) {
            self.eng.write_mem(
                gm + round.duration,
                &(round.enc_duration)(1000).to_le_bytes(),
            );
        }
        self.eng
            .write_mem(gm + round.end, &(round.enc_end)(now).to_le_bytes());
    }

    fn apply_y5_infinite(&mut self) {
        let Some((game, round)) = self.y5_round() else {
            return;
        };
        let gm = self.y5_host(game);
        if gm == 0 {
            return;
        }
        let infinite = (round.enc_duration)(0);
        let start = self.eng.read_u32(gm + round.start);
        let duration = self.eng.read_u32(gm + round.duration);
        let now = self.eng.read_u32(gm + round.now);
        if !self.y5_infinite {
            if let Some(phase) = self.y5_phase.take() {
                if phase.start == start && duration == infinite {
                    let end = (round.dec_end)(phase.end).wrapping_add(now.wrapping_sub(phase.at));
                    self.eng
                        .write_mem(gm + round.end, &(round.enc_end)(end).to_le_bytes());
                    self.eng
                        .write_mem(gm + round.duration, &phase.duration.to_le_bytes());
                    self.y5_infinite_display(round, gm, false);
                }
            }
            return;
        }
        let state = self.y5_state(game, gm);
        if (state == 5 || state == 7)
            && self.y5_flag(gm, round.running)
            && self.y5_flag(gm, round.countdown)
            && duration != infinite
            && (round.dec_duration)(duration) >= Y5_MIN_PHASE_MS
        {
            self.y5_phase = Some(Y5Phase {
                start,
                duration,
                end: self.eng.read_u32(gm + round.end),
                at: now,
            });
            self.eng
                .write_mem(gm + round.duration, &infinite.to_le_bytes());
            self.eng
                .write_mem(gm + round.end, &(round.enc_end)(now).to_le_bytes());
            self.y5_infinite_display(round, gm, true);
        }
    }

    fn y5_infinite_display(&self, round: &Y5Round, gm: u64, on: bool) {
        if let Some((offset, enabled, disabled)) = round.infinite_display {
            self.eng
                .write_mem(gm + offset, &[if on { enabled } else { disabled }]);
        }
    }

    fn y5_scan(&mut self, game: &Y5Game) {
        let modes: Vec<u64> = Y5_MULTIPLAYER
            .iter()
            .chain(Y5_TERRORIST_HUNT)
            .map(|m| m.1)
            .chain([Y5_GYM])
            .chain(game.events.iter().map(|e| e.1))
            .collect();
        let playlists: Vec<u64> = game.events.iter().map(|e| e.2).collect();
        let stored: HashMap<u64, u64> = game
            .maps
            .iter()
            .chain(Y5_DIFFICULTIES)
            .map(|t| t.1)
            .chain(modes.iter().chain(&playlists).copied())
            .map(|uid| (game.enc_handle_uid.map_or(uid, |enc| enc(uid)), uid))
            .collect();
        let mut targets: Vec<u64> = stored.keys().chain(&playlists).copied().collect();
        targets.sort_unstable();
        targets.dedup();
        let mut hits = Vec::new();
        self.eng.for_each_region(|base, b, owned| {
            for (j, q) in b[..owned].as_chunks::<8>().0.iter().enumerate() {
                if targets.binary_search(&u64::from_le_bytes(*q)).is_ok() {
                    hits.push(base + j as u64 * 8);
                }
            }
        });
        let image = self.eng.base..self.eng.base + self.eng.modsize as u64;
        let gamemode = self.eng.base + game.gamemode;
        self.y5_handles.clear();
        self.y5_playlists.clear();
        for at in hits {
            let raw = self.eng.read_u64(at);
            let obj = at - 0x10;
            let head = self.eng.read_u64(obj);
            if image.contains(&head) {
                if playlists.contains(&raw) {
                    self.y5_playlists.entry(raw).or_insert(obj);
                }
            } else if let Some(&uid) = stored.get(&raw) {
                if obj % 0x20 == 0
                    && self.eng.read_u64(obj + 0x18) == 0
                    && self.eng.read_u64(obj + 8) >> 32 != 0
                    && (head == 0 || (0x10000..0x7FF0_0000_0000).contains(&head))
                    && (!modes.contains(&uid) || self.eng.read_u64(head) == gamemode)
                {
                    self.y5_handles.entry(uid).or_insert(obj);
                }
            }
        }
    }

    fn y5_handle(&self, game: &Y5Game, uid: u64) -> u64 {
        match self.y5_handles.get(&uid) {
            Some(&handle)
                if self.eng.read_u64(handle + 0x10)
                    == game.enc_handle_uid.map_or(uid, |enc| enc(uid)) =>
            {
                handle
            }
            _ => 0,
        }
    }

    fn y5_array(&self, at: u64) -> Vec<u64> {
        self.y5_entries(self.eng.read_u64(at), self.eng.read_u64(at + 8))
    }

    fn y5_entries(&self, ptr: u64, packed: u64) -> Vec<u64> {
        if ptr == 0 || packed as u32 & 0xC000_0000 != 0xC000_0000 {
            return Vec::new();
        }
        (0..(packed & 0x3FFF_FFFF).min(4096))
            .map(|i| self.eng.read_u64(ptr + i * 8))
            .collect()
    }

    fn y5_write_settings(&self, at: u64, entries: &[u64]) {
        let ptr = self.eng.read_u64(at);
        let packed = self.eng.read_u64(at + 8);
        let cap = packed >> 32;
        let count = entries.len() as u64;
        if ptr == 0 || packed as u32 & 0xC000_0000 != 0xC000_0000 || count > cap {
            return;
        }
        for (i, entry) in entries.iter().enumerate() {
            self.eng.write_mem(ptr + i as u64 * 8, &entry.to_le_bytes());
        }
        self.eng
            .write_mem(at + 8, &(cap << 32 | count | 0xC000_0000).to_le_bytes());
        for i in count..(packed & 0x3FFF_FFFF).min(cap) {
            self.eng.write_mem(ptr + i * 8, &0u64.to_le_bytes());
        }
    }

    fn apply_y5_playlist(&mut self) {
        let Some(pick) = self.y5_pick else {
            return;
        };
        let Some(game) = y5_game(self.build) else {
            return;
        };
        let gm = self.y5_host(game);
        if gm == 0 || self.y5_state(game, gm) != 3 {
            return;
        }
        let m = self.y5_deref(gm, game.match_state);
        if m == 0 {
            return;
        }
        let ready = |runner: &Self| {
            [pick.mode, pick.map, pick.difficulty]
                .iter()
                .all(|&uid| uid == 0 || runner.y5_handle(game, uid) != 0)
                && (pick.playlist == 0 || runner.y5_playlists.contains_key(&pick.playlist))
        };
        if !ready(self) {
            if self.y5_scanned == m {
                return;
            }
            self.y5_scanned = m;
            self.y5_scan(game);
            if !ready(self) {
                return;
            }
        }
        self.eng
            .write_mem(m + 0x08, &self.y5_handle(game, pick.mode).to_le_bytes());
        self.eng
            .write_mem(m + 0x10, &self.y5_handle(game, pick.map).to_le_bytes());
        if pick.difficulty != 0 {
            self.eng.write_mem(
                m + game.match_layout.difficulty,
                &self.y5_handle(game, pick.difficulty).to_le_bytes(),
            );
        }
        if pick.playlist == 0 {
            self.y5_restore(game, m);
        } else {
            self.y5_event(game, m, pick.playlist);
        }
    }

    fn y5_event(&mut self, game: &Y5Game, m: u64, playlist: u64) {
        let tod_at = self.eng.read_ptr(m + game.match_layout.time_of_day);
        let lobby = match self.y5_lobby.take() {
            Some(lobby) if lobby.m == m => lobby,
            _ => Y5Lobby {
                m,
                playlist: self.eng.read_u64(m),
                tod_at,
                tod: self.eng.read_u64(tod_at),
                entries: self.y5_array(m + game.match_layout.settings),
            },
        };
        let obj = self.y5_playlists[&playlist];
        let at = obj + game.settings;
        let settings = match game.dec_settings {
            Some([ptr, count]) => {
                self.y5_entries(ptr(self.eng.read_u64(at)), count(self.eng.read_u64(at + 8)))
            }
            None => self.y5_array(at),
        };
        self.y5_write_settings(
            m + game.match_layout.settings,
            if settings.is_empty() {
                &lobby.entries
            } else {
                &settings
            },
        );
        if tod_at != 0 {
            let tod = self
                .y5_array(obj + game.time_of_day)
                .first()
                .copied()
                .unwrap_or(lobby.tod);
            self.eng.write_mem(tod_at, &tod.to_le_bytes());
        }
        self.eng.write_mem(m, &playlist.to_le_bytes());
        self.y5_lobby = Some(lobby);
    }

    fn y5_restore(&mut self, game: &Y5Game, m: u64) {
        let Some(lobby) = self.y5_lobby.take() else {
            return;
        };
        if lobby.m != m {
            return;
        }
        self.eng.write_mem(m, &lobby.playlist.to_le_bytes());
        if lobby.tod_at != 0 && self.eng.read_ptr(m + game.match_layout.time_of_day) == lobby.tod_at
        {
            self.eng.write_mem(lobby.tod_at, &lobby.tod.to_le_bytes());
        }
        self.y5_write_settings(m + game.match_layout.settings, &lobby.entries);
    }

    fn set_watermark(&self, enabled: bool) -> bool {
        let Some(&(_, flag, manager, layer, dirty)) =
            WATERMARKS.iter().find(|(build, ..)| *build == self.build)
        else {
            return self.set_watermark_overlay(enabled);
        };
        self.eng
            .write_mem(self.eng.base + flag, &[u8::from(enabled)]);
        let ui = self.eng.read_ptr(self.eng.base + manager);
        if ui != 0 {
            self.eng.write_mem(ui + layer, &[u8::from(enabled)]);
            self.eng.write_mem(ui + dirty, &[1]);
        }
        true
    }

    fn set_watermark_overlay(&self, enabled: bool) -> bool {
        let Some(&(_, flag, manager, counter)) = WATERMARK_OVERLAYS
            .iter()
            .find(|(build, ..)| *build == self.build)
        else {
            return false;
        };
        self.eng
            .write_mem(self.eng.base + flag, &[if enabled { 0x5B } else { 0x59 }]);
        let ui = self.eng.read_ptr(self.eng.base + manager);
        let list = self.y5_deref(ui, &[0x1598, 0]);
        let data = self.y5_deref(list, &[0x28]);
        if data == 0 {
            return true;
        }
        let head = self.eng.read_u32(list + 0x30);
        let size = head & 0x3FFF_FFFF;
        let id = self.eng.read_u32(ui + 0x1584);
        let at = |k: u32| data + u64::from(k) * 0x18;
        if enabled {
            let object = self.eng.read_ptr(ui + 0x950);
            if id != 0 || object == 0 || size >= self.eng.read_u32(list + 0x34) & 0x3FFF_FFFF {
                return true;
            }
            let id = self.eng.read_u32(self.eng.base + counter).wrapping_add(1);
            self.eng
                .write_mem(self.eng.base + counter, &id.to_le_bytes());
            self.eng.write_mem(at(size), &id.to_le_bytes());
            self.eng.write_mem(at(size) + 8, &object.to_le_bytes());
            self.eng.write_mem(at(size) + 0x10, &[0; 4]);
            let refs = self.eng.read_u32(object + 0xC).wrapping_add(1);
            self.eng.write_mem(object + 0xC, &refs.to_le_bytes());
            self.eng.write_mem(ui + 0x1584, &id.to_le_bytes());
            self.eng.write_mem(
                list + 0x30,
                &((head & 0xC000_0000) | (size + 1)).to_le_bytes(),
            );
        } else {
            if id == 0 {
                return true;
            }
            if let Some(k) = (0..size).find(|&k| self.eng.read_u32(at(k)) == id) {
                let object = self.eng.read_ptr(at(k) + 8);
                let last = size - 1;
                if k != last {
                    let mut moved = [0u8; 0x18];
                    self.eng.read_mem(at(last), &mut moved);
                    self.eng.write_mem(at(k), &moved);
                }
                self.eng
                    .write_mem(list + 0x30, &((head & 0xC000_0000) | last).to_le_bytes());
                if object != 0 {
                    let refs = self.eng.read_u32(object + 0xC).wrapping_sub(1);
                    self.eng.write_mem(object + 0xC, &refs.to_le_bytes());
                }
            }
            self.eng.write_mem(ui + 0x1584, &[0; 4]);
        }
        self.eng.write_mem(list + 0x38, &[1]);
        true
    }

    fn write_state(&self, feature: &str) -> bool {
        for (build, name, global, off, value) in STATE_WRITES {
            if *build != self.build || *name != feature {
                continue;
            }
            let obj = self.eng.read_ptr(self.eng.base + global);
            if obj != 0 {
                self.eng
                    .write_mem(obj.wrapping_add(*off as i64 as u64), &[*value]);
            }
            return true;
        }
        false
    }

    fn apply_weapons(&self) {
        if self.disable_primary && self.disable_secondary && self.season < SEASON_Y2S3 {
            self.apply_static("SetEmptySecondary", "enable");
            self.apply_static("SetDisablePrimaryWeapon", "enable");
        } else {
            self.apply_static("SetEmptySecondary", "disable");
            self.apply_static(
                "SetDisablePrimaryWeapon",
                if self.disable_primary {
                    "enable"
                } else {
                    "disable"
                },
            );
            self.apply_static(
                "SetDisableSecondaryWeapon",
                if self.disable_secondary {
                    "enable"
                } else {
                    "disable"
                },
            );
        }
    }

    fn disable_self_terminate(&self) {
        for (module, export) in [
            (s!("kernel32.dll"), s!("TerminateProcess")),
            (s!("ntdll.dll"), s!("NtTerminateProcess")),
        ] {
            if let Some(f) = unsafe { GetProcAddress(GetModuleHandleA(module), export) } {
                self.eng.write_mem(f as usize as u64, &[0xC3]);
            }
        }
    }

    fn set_mod(&mut self, mod_: &str, enabled: bool) {
        if !self.patched {
            return;
        }
        if self.shadow_toggle(mod_, enabled) {
            return;
        }
        if mod_ == "displayBuild" && self.set_watermark(enabled) {
            return;
        }
        if self.y5_round().is_some() {
            if let Some(i) = Y5_DISABLE_SLOTS.iter().position(|n| *n == mod_) {
                self.y5_disable[i] = enabled;
                self.apply_y5_disable();
                return;
            }
        }
        match mod_ {
            "infiniteTime" if self.y5_round().is_some() => {
                self.y5_infinite = enabled;
                self.apply_y5_infinite();
            }
            "infiniteTime" => self.write_chain("SetInfiniteTime", &[u8::from(!enabled)]),
            "disablePrimary" => {
                self.disable_primary = enabled;
                self.apply_weapons();
            }
            "disableSecondary" => {
                self.disable_secondary = enabled;
                self.apply_weapons();
            }
            _ => {
                if let Some((_, feature)) = CAPS.iter().find(|(key, _)| *key == mod_) {
                    self.apply_static(feature, if enabled { "enable" } else { "disable" });
                }
            }
        }
    }

    fn set_playlist(&mut self, id_str: &str) {
        if !self.patched {
            return;
        }
        if let Some(game) = y5_game(self.build) {
            if let Some(pick) = parse_y5_pick(game, id_str) {
                self.y5_pick = Some(pick);
                self.y5_scanned = 0;
                self.apply_y5_playlist();
            }
            return;
        }
        let raw = if let Some(rest) = id_str.strip_prefix("hereford1:") {
            self.set_old_hereford(true);
            rest
        } else if let Some(rest) = id_str.strip_prefix("hereford0:") {
            self.set_old_hereford(false);
            rest
        } else {
            id_str
        };
        self.gametype = raw.parse().unwrap_or(0);
        self.apply_gametype();
    }

    fn apply_gametype(&self) {
        if self.gametype == 0 {
            return;
        }
        if let Some(fc) = find_chain("SetGametype", self.build) {
            let a = self.eng.resolve_ptr_chain(fc.base_offset, fc.offsets);
            if a != 0 && self.eng.read_u64(a) != self.gametype {
                self.eng.write_mem(a, &self.gametype.to_le_bytes());
            }
        }
    }

    fn is_idle(&self) -> bool {
        IDLE.iter()
            .find(|(b, _)| *b == self.build)
            .is_none_or(|(_, addr)| self.eng.read1(self.eng.base.wrapping_add(*addr)) != 0)
    }

    fn apply_event_mode(&self, root: &mut TNode, em: &str) {
        if root.children.is_empty() {
            return;
        }
        match em {
            "Mad_House" => {
                relabel_event(&mut root.children[0], &[2, 0, 2], &[2, 0], 2, "Mad House");
            }
            "Rainbow_Is_Magic" => {
                relabel_event(
                    &mut root.children[0],
                    &[0, 21, 0],
                    &[0],
                    21,
                    "Rainbow is Magic",
                );
            }
            "Showdown" => {
                relabel_event(&mut root.children[0], &[1, 21, 0], &[1], 21, "Showdown");
            }
            "Doktors_Curse_MoneyHeist" => {
                relabel_event(&mut root.children[0], &[5, 0, 0], &[], 5, "Money Heist");
                let idv = find_chain("DoktorsCurse", self.build).map_or(0, |fc| {
                    self.eng
                        .read_ptr(self.eng.resolve_ptr_chain(fc.base_offset, fc.offsets))
                });
                root.children[0].children.push(TNode {
                    text: "Doktor's Curse".to_string(),
                    id: format!("{idv}"),
                    children: Vec::new(),
                });
            }
            "Stadium" => {
                relabel_event(
                    &mut root.children[0],
                    &[5, 0, 0],
                    &[],
                    5,
                    "Road To S.I. 2020",
                );
            }
            _ => {}
        }
    }

    fn build_tree_json(&self) -> String {
        let Some(tp) = TREE_PARAMS.iter().find(|t| t.build == self.build) else {
            return TREE_NULL.to_string();
        };
        let root_addr = self.eng.resolve_ptr_chain(tp.root_base, tp.root_offs);
        if root_addr == 0 {
            return TREE_NULL.to_string();
        }
        let mut rh = TNode::default();
        for i in 0..tp.root_count {
            let a = self.eng.read_ptr(root_addr + i * (tp.stride as u64));
            let child = self
                .eng
                .build_nodes(a, true, tp.name_off, tp.children_off, 0);
            rh.children.push(child);
        }
        self.apply_event_mode(&mut rh, tp.event_mode);
        for &r in tp.remove.iter().rev() {
            if r < rh.children.len() {
                rh.children.remove(r);
            }
        }
        if let Some(t) = rh.children.get_mut(0) {
            label_multiplayer(t, self.season);
        }
        if let Some(t) = rh.children.get_mut(1) {
            label_terrorist_hunt(t, self.season);
        }
        if let Some(t) = rh.children.get_mut(3) {
            label_matchmaking(t);
        }
        if let Some(t) = rh.children.get_mut(2) {
            label_situation(t, tp.situ_adv);
        }
        if tp.i_vr >= 0 {
            if let Some(t) = rh.children.get_mut(tp.i_vr as usize) {
                label_video_review(t);
            }
        }
        if tp.i_gym >= 0 {
            if let Some(t) = rh.children.get_mut(tp.i_gym as usize) {
                label_gym(t, self.season);
            }
        }
        if tp.i_ob >= 0 {
            if let Some(t) = rh.children.get_mut(tp.i_ob as usize) {
                label_outbreak(t);
            }
        }
        if find_chain("SetOldHereford", self.build).is_some() {
            add_original_hereford(&mut rh);
        }
        group_development_nodes(&mut rh, tp.i_gym, tp.i_vr);
        group_event_nodes(&mut rh, tp.event_mode);
        tree_event(&mut rh.children)
    }

    fn set_old_hereford(&self, on: bool) {
        let value: i64 = if on { 708483531 } else { 127951053400 };
        self.write_chain("SetOldHereford", &value.to_le_bytes());
    }

    fn scan_features(&mut self) {
        let round = self.y5_round().is_some();
        for (on, (key, feature)) in self.available.iter_mut().zip(CAPS) {
            *on = find_chain(feature, self.build).is_some()
                || patches_have(feature, self.build)
                || shadow_toggles(self.build)
                    .iter()
                    .any(|(name, ..)| *name == key)
                || (round && matches!(key, "infiniteTime" | "endRound"))
                || (round && Y5_DISABLE_SLOTS.contains(&key))
                || (key == "displayBuild"
                    && (WATERMARKS.iter().any(|(build, ..)| *build == self.build)
                        || WATERMARK_OVERLAYS
                            .iter()
                            .any(|(build, ..)| *build == self.build)))
                || STATE_WRITES
                    .iter()
                    .any(|(b, f, ..)| *b == self.build && *f == feature);
        }
    }

    fn build_state(&self) -> String {
        let full = self.is_full_feature();
        let caps: String = CAPS
            .iter()
            .zip(self.available)
            .map(|((key, _), on)| format!("\"{key}\":{},", full && on))
            .collect();
        format!(
            "{{\"event\":\"state\",\"attached\":{},\"unlocked\":{},\"unsupported\":{},\"capabilities\":{{{caps}\"fullFeature\":{full}}}}}",
            !self.eng.proc.is_null(),
            self.unlocked,
            self.unsupported
        )
    }

    fn push_state(&mut self) {
        let s = self.build_state();
        if s != self.lastsig {
            self.send_line(&s);
            self.lastsig = s;
        }
    }

    fn tick(&mut self) {
        if !self.eng.proc.is_null() && !self.eng.process_alive() {
            std::process::exit(0);
        }
        if self.eng.proc.is_null() && !self.eng.attach() {
            return;
        }
        if self.scanned.is_empty() {
            let Some(s) = self.eng.detect_build() else {
                return;
            };
            self.scanned = s;
        }
        if self.build.is_empty() {
            let num = self.scanned.rsplit_once('_').map_or("", |(_, n)| n);
            let Some(&(build, season)) = BUILD_SEASONS
                .iter()
                .find(|(b, _)| b.strip_suffix(num).is_some_and(|p| p.ends_with('_')))
            else {
                self.unsupported = true;
                return;
            };
            self.build = build;
            self.season = season;
            self.scan_features();
        }
        if self.pending {
            let idle = match shadow_regions_for_build(self.build) {
                Some(regions) => self.eng.shadow_delta != 0 || self.eng.shadow_run(regions),
                None => self.is_idle(),
            };
            if !idle {
                return;
            }
            self.pending = false;
            self.countdown = if patches_have("ApplyCorePatch", self.build) {
                4
            } else {
                0
            };
            if self.is_full_feature() {
                let tj = self.tree_json();
                self.tree_sent = tj != TREE_NULL;
                self.send_line(&tj);
            }
            return;
        }
        if self.countdown > 0 {
            self.countdown -= 1;
            return;
        }
        if !self.patched {
            self.disable_self_terminate();
            self.eng.shadow_arm_pages();
            self.apply_static("ApplyCorePatch", "always");
            self.end_match_patch(false);
            self.patched = true;
        }
        for (build, offset, value) in INIT_BYTES {
            if *build == self.build && self.eng.read1(self.eng.base + offset) != *value {
                self.eng.write_mem(self.eng.base + offset, &[*value]);
            }
        }
        if !unlock(
            &self.eng,
            self.build,
            &mut self.ownership,
            &mut self.onboarding,
            &mut self.badges,
        ) {
            self.unlocked = false;
            return;
        }
        if !self.unlocked {
            self.apply_static("ApplyOfflinePatch", "always");
        }
        self.apply_gametype();
        self.apply_y5_disable();
        self.apply_y5_playlist();
        if self.y5_infinite {
            self.apply_y5_infinite();
        }
        if self.y5_match_end {
            self.apply_y5_match_end();
        }
        if !self.tree_sent && self.is_full_feature() {
            let tj = self.tree_json();
            if tj != TREE_NULL {
                self.tree_sent = true;
                self.send_line(&tj);
            }
        }
        self.unlocked = true;
    }

    fn handle_command(&mut self, line: &str) {
        match json_str(line, "cmd") {
            Some("setMod") => {
                if let Some(m) = json_str(line, "mod") {
                    self.set_mod(m, json_bool(line, "enabled"));
                }
            }
            Some("setPlaylist") => {
                if let Some(id) = json_str(line, "playlistId") {
                    self.set_playlist(id);
                }
            }
            Some("endRound") => self.end_round(),
            Some("endMatch") => self.end_match(),
            _ => {}
        }
    }

    fn run(&mut self) -> i32 {
        let Ok(reader) = self.client.try_clone() else {
            return 1;
        };
        let (tx, rx) = mpsc::channel();
        std::thread::spawn(move || {
            for line in BufReader::new(reader).split(b'\n') {
                let Ok(line) = line else {
                    break;
                };
                if tx
                    .send(String::from_utf8_lossy(&line).into_owned())
                    .is_err()
                {
                    break;
                }
            }
        });
        let mut next = Instant::now();
        loop {
            match rx.recv_timeout(next.saturating_duration_since(Instant::now())) {
                Ok(line) if !line.is_empty() => self.handle_command(&line),
                Ok(_) | Err(RecvTimeoutError::Timeout) => {}
                Err(RecvTimeoutError::Disconnected) => return 0,
            }
            if Instant::now() >= next {
                next = Instant::now() + Duration::from_secs(1);
                self.tick();
                self.push_state();
            }
        }
    }
}

pub fn main_entry() -> i32 {
    let args: Vec<String> = std::env::args().collect();
    if args.len() > 2 && args[1] == "--runtime" {
        if let Ok(port) = args[2].parse::<u16>() {
            let Ok(client) = TcpStream::connect(("127.0.0.1", port)) else {
                return 1;
            };
            return Runner::new(client).run();
        }
    }
    2
}
