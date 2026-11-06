use core::ffi::c_void;
use std::collections::HashMap;
use std::ffi::{CStr, OsString};
use std::io::{BufRead, BufReader, Write};
use std::mem::{size_of, zeroed};
use std::net::TcpStream;
use std::os::windows::ffi::OsStringExt;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{self, RecvTimeoutError};
use std::time::{Duration, Instant};

use windows_sys::s;
use windows_sys::Wdk::System::Threading::{NtQueryInformationProcess, ProcessBasicInformation};
use windows_sys::Win32::Foundation::{CloseHandle, HANDLE, INVALID_HANDLE_VALUE, STILL_ACTIVE};
use windows_sys::Win32::System::Diagnostics::Debug::{ReadProcessMemory, WriteProcessMemory};
use windows_sys::Win32::System::Diagnostics::ToolHelp::{
    CreateToolhelp32Snapshot, Module32FirstW, Process32First, Process32Next, MODULEENTRY32W,
    PROCESSENTRY32, TH32CS_SNAPMODULE, TH32CS_SNAPMODULE32, TH32CS_SNAPPROCESS,
};
use windows_sys::Win32::System::LibraryLoader::{GetModuleHandleA, GetProcAddress};
use windows_sys::Win32::System::Memory::{VirtualAllocEx, MEM_COMMIT, MEM_RESERVE};
use windows_sys::Win32::System::Threading::{
    GetExitCodeProcess, OpenProcess, PROCESS_BASIC_INFORMATION, PROCESS_CREATE_THREAD,
    PROCESS_QUERY_INFORMATION, PROCESS_VM_OPERATION, PROCESS_VM_READ, PROCESS_VM_WRITE,
};

use crate::shadow::{playlist_sync, shadow_regions_for_build, warmup_gate};
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
    let mut found: Option<(usize, u32)> = None;
    if unsafe { Process32First(snap, &mut pe) } != 0 {
        loop {
            let exe = pe.szExeFile.map(|c| c as u8);
            let exe = CStr::from_bytes_until_nul(&exe).map_or(&[][..], CStr::to_bytes);
            if let Some(rank) = names
                .iter()
                .position(|n| exe.eq_ignore_ascii_case(n.as_bytes()))
            {
                if found.is_none_or(|(best, _)| rank < best) {
                    found = Some((rank, pe.th32ProcessID));
                }
            }
            if unsafe { Process32Next(snap, &mut pe) } == 0 {
                break;
            }
        }
    }
    unsafe { CloseHandle(snap) };
    found.map(|(rank, pid)| (pid, names[rank]))
}

fn module_info(pid: u32) -> Option<(u64, u32, PathBuf)> {
    let snap = unsafe { CreateToolhelp32Snapshot(TH32CS_SNAPMODULE | TH32CS_SNAPMODULE32, pid) };
    if snap == INVALID_HANDLE_VALUE {
        return None;
    }
    let mut me: MODULEENTRY32W = unsafe { zeroed() };
    me.dwSize = size_of::<MODULEENTRY32W>() as u32;
    let found = unsafe { Module32FirstW(snap, &mut me) } != 0;
    unsafe { CloseHandle(snap) };
    if !found {
        return None;
    }
    let path = &me.szExePath;
    let len = path.iter().position(|&c| c == 0).unwrap_or(path.len());
    let exe = PathBuf::from(OsString::from_wide(&path[..len]));
    let dir = match exe.parent() {
        Some(dir) if exe.is_file() => dir.to_path_buf(),
        _ => PathBuf::new(),
    };
    Some((me.modBaseAddr as u64, me.modBaseSize, dir))
}

#[derive(Default)]
pub struct Engine {
    pub proc: HANDLE,
    pub base: u64,
    pub modsize: u32,
    pub exe: &'static str,
    pub dir: PathBuf,
    pub(crate) renderer: Option<&'static RendererBuild>,
    pub(crate) shadow_pages: Vec<u64>,
    pub(crate) shadow_delta: i64,
    pub(crate) shadow_probe: u64,
}

impl Engine {
    pub fn attach(&mut self) -> bool {
        let Some((pid, name)) = find_process(&[
            "RainbowSixGame.exe",
            "RainbowSix.exe",
            "RainbowSix_DX11.exe",
            "RainbowSix_Vulkan.exe",
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
        let Some((base, size, dir)) = module_info(pid) else {
            unsafe { CloseHandle(h) };
            return false;
        };
        let mut info: PROCESS_BASIC_INFORMATION = unsafe { zeroed() };
        unsafe {
            NtQueryInformationProcess(
                h,
                ProcessBasicInformation,
                &mut info as *mut PROCESS_BASIC_INFORMATION as *mut c_void,
                size_of::<PROCESS_BASIC_INFORMATION>() as u32,
                core::ptr::null_mut(),
            )
        };
        self.proc = h;
        self.base = base;
        IMAGE_KEY.store(base >> 33, Ordering::Relaxed);
        PEB_KEY.store(info.PebBaseAddress as u64 >> 12, Ordering::Relaxed);
        self.modsize = size;
        self.exe = name;
        self.dir = dir;
        true
    }

    pub(crate) fn at(&self, rva: u64) -> u64 {
        let Some(renderer) = self.renderer else {
            return self.base.wrapping_add(rva);
        };
        renderer
            .rvas
            .iter()
            .find(|&&(dx, _)| dx == rva)
            .map_or(0, |&(_, alt)| self.base.wrapping_add(alt))
    }

    pub(crate) fn bytes<'a>(&self, rva: u64, dx: &'a [u8]) -> &'a [u8] {
        self.renderer
            .and_then(|renderer| renderer.patches.iter().find(|p| p.rva == rva && p.dx == dx))
            .map_or(dx, |p| p.alt)
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

    fn read_u8(&self, addr: u64) -> u8 {
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

    fn resolve_ptr_chain(&self, base_offset: u64, offsets: &[u64]) -> u64 {
        let Some((last, path)) = offsets.split_last() else {
            return 0;
        };
        let ptr = path
            .iter()
            .try_fold(self.read_u64(self.at(base_offset)), |p, off| {
                (p != 0).then(|| self.read_u64(p.wrapping_add(*off)))
            });
        match ptr {
            Some(p) if p != 0 => p.wrapping_add(*last),
            _ => 0,
        }
    }
}

fn find_chain(feature: &str, build: &str) -> Option<&'static FeatureChain> {
    FEATURE_CHAINS
        .iter()
        .find(|fc| fc.feature == feature && fc.build == build)
}

fn relabel_event(mp: &mut TNode, extract: &[usize], rm_path: &[usize], rm_idx: usize, label: &str) {
    if let Some(c) = get_path_mut(mp, extract).cloned() {
        mp.children.push(c);
    }
    rm(mp, rm_path, rm_idx);
    set_text(mp, &[5], label);
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
    ("skip", "Skip"),
    ("endMatch", "EndMatch"),
];

static STATE_WRITES: &[(&str, &str, u64, i32, u8)] =
    &[("Y1S0_8194013", "EndMatch", 0x5880AA8, 0x66C, 2)];

static INIT_BYTES: &[(&str, u64, u8)] = &[("Y8S3_62486471", 0x7FCA406, 0)];

static IMAGE_KEY: AtomicU64 = AtomicU64::new(0);
static PEB_KEY: AtomicU64 = AtomicU64::new(0);

fn image_key() -> u64 {
    IMAGE_KEY.load(Ordering::Relaxed)
}

fn peb_key() -> u64 {
    PEB_KEY.load(Ordering::Relaxed)
}

#[derive(Clone, Copy)]
enum Raw<T> {
    Fixed(T),
    Keyed(fn(u64) -> T),
}

impl<T> Raw<T> {
    fn get(self) -> T {
        match self {
            Raw::Fixed(value) => value,
            Raw::Keyed(value) => value(image_key()),
        }
    }
}

static WATERMARKS: &[(&str, u64, u64, u64, u64)] = &[
    ("Y6S4_37046065", 0x60D353A, 0x8111FF8, 0x13E3, 0x13E9),
    ("Y7S1_39785137", 0x699F56A, 0x8B90F80, 0x1423, 0x1428),
    ("Y7S2_43489433", 0x676C429, 0x895D618, 0x1483, 0x1488),
    ("Y7S3_47165842", 0x6D39679, 0x8ED3F40, 0x14CB, 0x14D0),
    ("Y7S4_50497889", 0x6E508D8, 0x8852A78, 0x147B, 0x1480),
    ("Y8S1_55217154", 0x6B2C4F8, 0x8D75F20, 0x14BB, 0x14C0),
];

struct WatermarkOverlay {
    build: &'static str,
    flag: u64,
    flag_bytes: Raw<[u8; 2]>,
    manager: u64,
    counter: u64,
    entries: i32,
    slot: u64,
    persistent: u64,
    dirty: u64,
}

static WATERMARK_OVERLAYS: &[WatermarkOverlay] = &[
    WatermarkOverlay {
        build: "Y8S2_58222837",
        flag: 0x6DF1EA0,
        flag_bytes: Raw::Fixed([0x5B, 0x59]),
        manager: 0x755A398,
        counter: 0x6D73D98,
        entries: 0x1598,
        slot: 0x1584,
        persistent: 0x950,
        dirty: 0x38,
    },
    WatermarkOverlay {
        build: "Y8S3_62486471",
        flag: 0x6BCFF28,
        flag_bytes: Raw::Fixed([0x30, 0x2F]),
        manager: 0x742EBC0,
        counter: 0x69D7848,
        entries: 0x1660,
        slot: 0x164C,
        persistent: 0x968,
        dirty: 0x38,
    },
    WatermarkOverlay {
        build: "Y8S4_67466841",
        flag: 0x6DB1E38,
        flag_bytes: Raw::Keyed(y8s4_watermark),
        manager: 0x761CC38,
        counter: 0x6CA2608,
        entries: 0x1708,
        slot: 0x16EC,
        persistent: 0x978,
        dirty: 0x38,
    },
    WatermarkOverlay {
        build: "Y9S1_69866602",
        flag: 0x79CC988,
        flag_bytes: Raw::Keyed(y9s1_watermark),
        manager: 0x809C168,
        counter: 0x792A3C8,
        entries: 0x16F0,
        slot: 0x16D4,
        persistent: 0x998,
        dirty: 0x38,
    },
    WatermarkOverlay {
        build: "Y9S2_72730050",
        flag: 0x7D662D8,
        flag_bytes: Raw::Keyed(y9s2_watermark),
        manager: 0x840F348,
        counter: 0x7C0D9B8,
        entries: 0x1700,
        slot: 0x16E4,
        persistent: 0x998,
        dirty: 0x60,
    },
    WatermarkOverlay {
        build: "Y9S3_79429095",
        flag: 0xBA6E208,
        flag_bytes: Raw::Keyed(y9s3_watermark),
        manager: 0xBB7C6E8,
        counter: 0xB8A8058,
        entries: 0x17B8,
        slot: 0x179C,
        persistent: 0x9B8,
        dirty: 0x60,
    },
    WatermarkOverlay {
        build: "Y9S4_82873685",
        flag: 0xD482958,
        flag_bytes: Raw::Fixed([0x1D, 0x1D]),
        manager: 0xD51D7A0,
        counter: 0xD24DED8,
        entries: 0x1980,
        slot: 0x1964,
        persistent: 0xA58,
        dirty: 0x60,
    },
    WatermarkOverlay {
        build: "Y10S1_87835624",
        flag: 0xE336418,
        flag_bytes: Raw::Keyed(y10s1_watermark),
        manager: 0xE3D47A8,
        counter: 0xE0F1968,
        entries: 0x19A0,
        slot: 0x1984,
        persistent: 0xB60,
        dirty: 0x60,
    },
];

fn patches_have(feature: &str, build: &str) -> bool {
    PATCHES
        .iter()
        .any(|p| p.feature == feature && p.build == build)
}

const TREE_NULL: &str = "{\"event\":\"tree\",\"tree\":null}";
const PLAYLIST_POLL: Duration = Duration::from_millis(100);

static DISABLE_MODS: [&str; 4] = [
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
        &[0xE9, 0xC8, 0x10, 0x8F, 0xFE, 0x90, 0x90, 0x90],
    ),
    (
        "unlimitedEquip",
        0x12984AB,
        &[
            0x3B, 0xD6, 0x0F, 0x4C, 0xD6, 0x8B, 0xDA, 0x89, 0x51, 0x54, 0x66, 0x90, 0x48, 0x85,
            0xFF, 0x74,
        ],
    ),
    (
        "unlimitedEquip",
        0x17022D4,
        &[
            0x48, 0x8B, 0x81, 0x68, 0x03, 0x00, 0x00, 0x46, 0x8D, 0x0C, 0x85, 0x00, 0x00, 0x00,
            0x00, 0x41, 0x3B, 0x2C, 0x01, 0x41, 0x0F, 0x4C, 0x2C, 0x01, 0x41, 0x3B, 0x2C, 0x01,
        ],
    ),
    ("disableAI", 0x1081B80, &[0xC3]),
    ("unlimitedAmmo", 0x1ABB148, &[0x0F, 0x1F, 0x00]),
    ("unlimitedAmmo", 0x13DF7DB, &[0x00]),
    ("unlimitedAmmo", 0x13E1EC0, &[0x00]),
    ("unlimitedAmmo", 0x1AB7BF6, &[0x0F, 0x1F, 0x00]),
    ("unlimitedAmmo", 0x1AC58B4, &[0xEB]),
    ("unlimitedAmmo", 0x1AC5A17, &[0x83]),
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
        0x27363E,
        &[0xE9, 0x4D, 0xF7, 0x1D, 0x00, 0x90, 0x90],
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
        &[
            0x48, 0x8B, 0x8F, 0x78, 0x03, 0x00, 0x00, 0x39, 0x34, 0x01, 0x0F, 0x4F, 0x34, 0x01,
            0x39, 0x34, 0x01, 0x66, 0x90,
        ],
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
            0x85, 0xF6, 0x0F, 0x84, 0x5B, 0x01, 0x00, 0x00, 0x48, 0x89, 0xCB, 0x0F, 0x1F, 0x00,
        ],
    ),
    (
        "unlimitedEquip",
        0x238DFBD,
        &[
            0x8B, 0x71, 0x6C, 0xBD, 0xFF, 0xFF, 0xFF, 0xFF, 0x0F, 0x1F, 0x44, 0x00, 0x00,
        ],
    ),
    (
        "unlimitedEquip",
        0x4387D42,
        &[
            0x8B, 0x71, 0x64, 0xBD, 0xFF, 0xFF, 0xFF, 0xFF, 0x0F, 0x1F, 0x40, 0x00,
        ],
    ),
    (
        "unlimitedEquip",
        0x438A39A,
        &[
            0x8B, 0xB9, 0xC4, 0x00, 0x00, 0x00, 0xBD, 0xFF, 0xFF, 0xFF, 0xFF, 0x0F, 0x1F, 0x40,
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
        0x43D8E27,
        &[0xE9, 0xD4, 0xA1, 0xB4, 0xFC, 0x90, 0x90],
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
        &[
            0x48, 0x8B, 0x8F, 0x78, 0x03, 0x00, 0x00, 0x39, 0x34, 0x01, 0x0F, 0x4F, 0x34, 0x01,
            0x39, 0x34, 0x01, 0x66, 0x90,
        ],
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
            0x8B, 0xB1, 0xC4, 0x00, 0x00, 0x00, 0xBD, 0xFF, 0xFF, 0xFF, 0xFF, 0x0F, 0x1F, 0x44,
            0x00, 0x00,
        ],
    ),
    (
        "unlimitedEquip",
        0x2DA976D,
        &[
            0x8B, 0x71, 0x6C, 0xBD, 0xFF, 0xFF, 0xFF, 0xFF, 0x0F, 0x1F, 0x44, 0x00, 0x00,
        ],
    ),
    (
        "unlimitedEquip",
        0x1536592,
        &[
            0x8B, 0x71, 0x64, 0xBD, 0xFF, 0xFF, 0xFF, 0xFF, 0x0F, 0x1F, 0x40, 0x00,
        ],
    ),
    (
        "unlimitedEquip",
        0x1538A1A,
        &[
            0x8B, 0xB9, 0xC4, 0x00, 0x00, 0x00, 0xBD, 0xFF, 0xFF, 0xFF, 0xFF, 0x0F, 0x1F, 0x40,
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
        0xEEAC67,
        &[0xE9, 0x94, 0x53, 0x4B, 0x03, 0x90, 0x90],
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
        &[
            0x48, 0x8B, 0x8F, 0x78, 0x03, 0x00, 0x00, 0x39, 0x34, 0x01, 0x0F, 0x4F, 0x34, 0x01,
            0x39, 0x34, 0x01, 0x66, 0x90,
        ],
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
            0x8B, 0xB1, 0xC4, 0x00, 0x00, 0x00, 0xBD, 0xFF, 0xFF, 0xFF, 0xFF, 0x0F, 0x1F, 0x44,
            0x00, 0x00,
        ],
    ),
    (
        "unlimitedEquip",
        0x28E910D,
        &[
            0x8B, 0x71, 0x6C, 0xBD, 0xFF, 0xFF, 0xFF, 0xFF, 0x0F, 0x1F, 0x44, 0x00, 0x00,
        ],
    ),
    (
        "unlimitedEquip",
        0xBA9412,
        &[
            0x8B, 0x71, 0x64, 0xBD, 0xFF, 0xFF, 0xFF, 0xFF, 0x0F, 0x1F, 0x40, 0x00,
        ],
    ),
    (
        "unlimitedEquip",
        0xBAB7AA,
        &[
            0x8B, 0xB9, 0xC4, 0x00, 0x00, 0x00, 0xBD, 0xFF, 0xFF, 0xFF, 0xFF, 0x0F, 0x1F, 0x40,
            0x00,
        ],
    ),
];

static Y6S4_TOGGLES: &[(&str, u64, &[u8])] = &[
    (
        "deathless",
        0x2A164A7,
        &[0xE9, 0x54, 0xCB, 0x69, 0x00, 0x90, 0x90],
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
        &[
            0x48, 0x8B, 0x8F, 0x78, 0x03, 0x00, 0x00, 0x39, 0x34, 0x01, 0x0F, 0x4F, 0x34, 0x01,
            0x39, 0x34, 0x01, 0x66, 0x90,
        ],
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
            0x8B, 0xB1, 0xC4, 0x00, 0x00, 0x00, 0xBD, 0xFF, 0xFF, 0xFF, 0xFF, 0x0F, 0x1F, 0x44,
            0x00, 0x00,
        ],
    ),
    (
        "unlimitedEquip",
        0x2FC77CD,
        &[
            0x8B, 0x71, 0x6C, 0xBD, 0xFF, 0xFF, 0xFF, 0xFF, 0x0F, 0x1F, 0x44, 0x00, 0x00,
        ],
    ),
    (
        "unlimitedEquip",
        0x49058D2,
        &[
            0x8B, 0x71, 0x64, 0xBD, 0xFF, 0xFF, 0xFF, 0xFF, 0x0F, 0x1F, 0x40, 0x00,
        ],
    ),
    (
        "unlimitedEquip",
        0x49078FA,
        &[
            0x8B, 0xB9, 0xC4, 0x00, 0x00, 0x00, 0xBD, 0xFF, 0xFF, 0xFF, 0xFF, 0x0F, 0x1F, 0x40,
            0x00,
        ],
    ),
];

static Y7S1_TOGGLES: &[(&str, u64, &[u8])] = &[
    (
        "deathless",
        0x4D9A367,
        &[0xE9, 0x94, 0x3C, 0x2B, 0x00, 0x90, 0x90],
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
        &[
            0x48, 0x8B, 0x8F, 0x68, 0x03, 0x00, 0x00, 0x39, 0x34, 0x01, 0x0F, 0x4F, 0x34, 0x01,
            0x39, 0x34, 0x01, 0x66, 0x90,
        ],
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
            0x8B, 0x91, 0xC4, 0x00, 0x00, 0x00, 0x0F, 0x1F, 0x84, 0x00, 0x00, 0x00, 0x00, 0x00,
        ],
    ),
    (
        "unlimitedEquip",
        0x1573212,
        &[
            0x8B, 0x91, 0xC4, 0x00, 0x00, 0x00, 0x0F, 0x1F, 0x84, 0x00, 0x00, 0x00, 0x00, 0x00,
        ],
    ),
    (
        "unlimitedEquip",
        0x1ADBAB4,
        &[
            0x8B, 0x91, 0xC4, 0x00, 0x00, 0x00, 0x0F, 0x1F, 0x84, 0x00, 0x00, 0x00, 0x00, 0x00,
        ],
    ),
    (
        "unlimitedEquip",
        0x2A4AC9B,
        &[
            0x8B, 0x91, 0xC4, 0x00, 0x00, 0x00, 0x0F, 0x1F, 0x84, 0x00, 0x00, 0x00, 0x00, 0x00,
        ],
    ),
    (
        "unlimitedEquip",
        0x2A50FE6,
        &[
            0x8B, 0x91, 0xC4, 0x00, 0x00, 0x00, 0x0F, 0x1F, 0x84, 0x00, 0x00, 0x00, 0x00, 0x00,
        ],
    ),
    (
        "unlimitedEquip",
        0x2A53806,
        &[
            0x8B, 0x91, 0xC4, 0x00, 0x00, 0x00, 0x0F, 0x1F, 0x84, 0x00, 0x00, 0x00, 0x00, 0x00,
        ],
    ),
    (
        "unlimitedEquip",
        0x2A69269,
        &[
            0x8B, 0x91, 0xC4, 0x00, 0x00, 0x00, 0x0F, 0x1F, 0x84, 0x00, 0x00, 0x00, 0x00, 0x00,
        ],
    ),
    (
        "unlimitedEquip",
        0x24D7340,
        &[
            0x8B, 0x51, 0x6C, 0x0F, 0x1F, 0x84, 0x00, 0x00, 0x00, 0x00, 0x00,
        ],
    ),
    (
        "unlimitedEquip",
        0x40020B0,
        &[
            0x8B, 0x41, 0x64, 0x0F, 0x1F, 0x84, 0x00, 0x00, 0x00, 0x00, 0x00, 0x90,
        ],
    ),
    (
        "unlimitedEquip",
        0x4004C4A,
        &[
            0x8B, 0xB1, 0xC4, 0x00, 0x00, 0x00, 0xBD, 0xFF, 0xFF, 0xFF, 0xFF, 0x0F, 0x1F, 0x40,
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
        0x1F2E449,
        &[0xE9, 0xB2, 0x6B, 0xF8, 0xFF, 0x90, 0x90, 0x90],
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
        &[
            0x48, 0x8B, 0x8F, 0x68, 0x03, 0x00, 0x00, 0x39, 0x34, 0x01, 0x0F, 0x4F, 0x34, 0x01,
            0x39, 0x34, 0x01, 0x66, 0x90,
        ],
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
            0x8B, 0x91, 0xC4, 0x00, 0x00, 0x00, 0x0F, 0x1F, 0x84, 0x00, 0x00, 0x00, 0x00, 0x00,
        ],
    ),
    (
        "unlimitedEquip",
        0x42E7B52,
        &[
            0x8B, 0x91, 0xC4, 0x00, 0x00, 0x00, 0x0F, 0x1F, 0x84, 0x00, 0x00, 0x00, 0x00, 0x00,
        ],
    ),
    (
        "unlimitedEquip",
        0x1D8BA94,
        &[
            0x8B, 0x91, 0xC4, 0x00, 0x00, 0x00, 0x0F, 0x1F, 0x84, 0x00, 0x00, 0x00, 0x00, 0x00,
        ],
    ),
    (
        "unlimitedEquip",
        0x407F9D3,
        &[
            0x8B, 0x91, 0xC4, 0x00, 0x00, 0x00, 0x0F, 0x1F, 0x84, 0x00, 0x00, 0x00, 0x00, 0x00,
        ],
    ),
    (
        "unlimitedEquip",
        0x4085CF6,
        &[
            0x8B, 0x91, 0xC4, 0x00, 0x00, 0x00, 0x0F, 0x1F, 0x84, 0x00, 0x00, 0x00, 0x00, 0x00,
        ],
    ),
    (
        "unlimitedEquip",
        0x4088676,
        &[
            0x8B, 0x91, 0xC4, 0x00, 0x00, 0x00, 0x0F, 0x1F, 0x84, 0x00, 0x00, 0x00, 0x00, 0x00,
        ],
    ),
    (
        "unlimitedEquip",
        0x38B8319,
        &[
            0x8B, 0x91, 0xC4, 0x00, 0x00, 0x00, 0x0F, 0x1F, 0x84, 0x00, 0x00, 0x00, 0x00, 0x00,
        ],
    ),
    (
        "unlimitedEquip",
        0x22763B0,
        &[
            0x8B, 0x51, 0x6C, 0x0F, 0x1F, 0x84, 0x00, 0x00, 0x00, 0x00, 0x00,
        ],
    ),
    (
        "unlimitedEquip",
        0x30A41D0,
        &[
            0x8B, 0x41, 0x64, 0x0F, 0x1F, 0x84, 0x00, 0x00, 0x00, 0x00, 0x00, 0x90,
        ],
    ),
    (
        "unlimitedEquip",
        0x30A6D4A,
        &[
            0x8B, 0xB1, 0xC4, 0x00, 0x00, 0x00, 0xBD, 0xFF, 0xFF, 0xFF, 0xFF, 0x0F, 0x1F, 0x40,
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
        0x3CDD5D7,
        &[0xE9, 0x24, 0xEA, 0xB5, 0x00, 0x90],
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
        &[
            0x48, 0x8B, 0x89, 0x68, 0x03, 0x00, 0x00, 0x39, 0x34, 0x01, 0x0F, 0x4F, 0x34, 0x01,
            0x39, 0x34, 0x01, 0x66, 0x90,
        ],
    ),
    ("unlimitedEquip", 0x54888A9, &[0x8D, 0x47, 0x00]),
    ("unlimitedEquip", 0x28BF170, &[0x83, 0x43, 0x44, 0x00]),
    ("unlimitedEquip", 0x5488920, &[0x83, 0x43, 0x44, 0x00]),
    (
        "unlimitedEquip",
        0x3E23D3C,
        &[
            0x8B, 0x91, 0xC4, 0x00, 0x00, 0x00, 0x0F, 0x1F, 0x84, 0x00, 0x00, 0x00, 0x00, 0x00,
        ],
    ),
    (
        "unlimitedEquip",
        0x3E25806,
        &[
            0x8B, 0x91, 0xC4, 0x00, 0x00, 0x00, 0x0F, 0x1F, 0x84, 0x00, 0x00, 0x00, 0x00, 0x00,
        ],
    ),
    (
        "unlimitedEquip",
        0x161EC54,
        &[
            0x8B, 0x91, 0xC4, 0x00, 0x00, 0x00, 0x0F, 0x1F, 0x84, 0x00, 0x00, 0x00, 0x00, 0x00,
        ],
    ),
    (
        "unlimitedEquip",
        0x580E004,
        &[0x0F, 0x1F, 0x84, 0x00, 0x00, 0x00, 0x00, 0x00],
    ),
    (
        "unlimitedEquip",
        0x26505F6,
        &[
            0x8B, 0x91, 0xC4, 0x00, 0x00, 0x00, 0x0F, 0x1F, 0x84, 0x00, 0x00, 0x00, 0x00, 0x00,
        ],
    ),
    (
        "unlimitedEquip",
        0x2652F06,
        &[
            0x8B, 0x91, 0xC4, 0x00, 0x00, 0x00, 0x0F, 0x1F, 0x84, 0x00, 0x00, 0x00, 0x00, 0x00,
        ],
    ),
    (
        "unlimitedEquip",
        0x2669FC9,
        &[
            0x8B, 0x91, 0xC4, 0x00, 0x00, 0x00, 0x0F, 0x1F, 0x84, 0x00, 0x00, 0x00, 0x00, 0x00,
        ],
    ),
    (
        "unlimitedEquip",
        0x2685340,
        &[
            0x8B, 0x51, 0x6C, 0x0F, 0x1F, 0x84, 0x00, 0x00, 0x00, 0x00, 0x00,
        ],
    ),
    (
        "unlimitedEquip",
        0x54806C0,
        &[
            0x8B, 0x41, 0x64, 0x0F, 0x1F, 0x84, 0x00, 0x00, 0x00, 0x00, 0x00, 0x90,
        ],
    ),
    (
        "unlimitedEquip",
        0x54821B0,
        &[
            0x8B, 0x81, 0xC4, 0x00, 0x00, 0x00, 0x83, 0xA1, 0xC4, 0x00, 0x00, 0x00, 0x00, 0x66,
            0x90,
        ],
    ),
    (
        "unlimitedEquip",
        0x3E261AA,
        &[0xBD, 0xFF, 0xFF, 0xFF, 0xFF, 0x90],
    ),
];

static Y7S4_TOGGLES: &[(&str, u64, &[u8])] = &[
    ("deathless", 0x159490, &[0xE9, 0x6B, 0x0B, 0x75, 0x01, 0x90]),
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
        &[
            0x48, 0x8B, 0x89, 0x68, 0x03, 0x00, 0x00, 0x39, 0x34, 0x01, 0x0F, 0x4F, 0x34, 0x01,
            0x39, 0x34, 0x01, 0x66, 0x90,
        ],
    ),
    ("unlimitedEquip", 0x504D5C9, &[0x8D, 0x47, 0x00]),
    ("unlimitedEquip", 0x521B880, &[0x83, 0x43, 0x44, 0x00]),
    ("unlimitedEquip", 0x504D640, &[0x83, 0x43, 0x44, 0x00]),
    (
        "unlimitedEquip",
        0x4561219,
        &[
            0x8B, 0x91, 0xC4, 0x00, 0x00, 0x00, 0x0F, 0x1F, 0x84, 0x00, 0x00, 0x00, 0x00, 0x00,
        ],
    ),
    (
        "unlimitedEquip",
        0x456260E,
        &[
            0x8B, 0x91, 0xC4, 0x00, 0x00, 0x00, 0x0F, 0x1F, 0x84, 0x00, 0x00, 0x00, 0x00, 0x00,
        ],
    ),
    (
        "unlimitedEquip",
        0xE294E1,
        &[
            0x8B, 0x91, 0xC4, 0x00, 0x00, 0x00, 0x0F, 0x1F, 0x84, 0x00, 0x00, 0x00, 0x00, 0x00,
        ],
    ),
    (
        "unlimitedEquip",
        0x5063BBB,
        &[
            0x8B, 0x91, 0xC4, 0x00, 0x00, 0x00, 0x0F, 0x1F, 0x84, 0x00, 0x00, 0x00, 0x00, 0x00,
        ],
    ),
    (
        "unlimitedEquip",
        0x506AC86,
        &[
            0x8B, 0x91, 0xC4, 0x00, 0x00, 0x00, 0x0F, 0x1F, 0x84, 0x00, 0x00, 0x00, 0x00, 0x00,
        ],
    ),
    (
        "unlimitedEquip",
        0x506CC96,
        &[
            0x8B, 0x91, 0xC4, 0x00, 0x00, 0x00, 0x0F, 0x1F, 0x84, 0x00, 0x00, 0x00, 0x00, 0x00,
        ],
    ),
    (
        "unlimitedEquip",
        0x5084FC9,
        &[
            0x8B, 0x91, 0xC4, 0x00, 0x00, 0x00, 0x0F, 0x1F, 0x84, 0x00, 0x00, 0x00, 0x00, 0x00,
        ],
    ),
    (
        "unlimitedEquip",
        0x3A931A0,
        &[
            0x8B, 0x51, 0x6C, 0x0F, 0x1F, 0x84, 0x00, 0x00, 0x00, 0x00, 0x00,
        ],
    ),
    (
        "unlimitedEquip",
        0x42AC6E0,
        &[
            0x8B, 0x41, 0x64, 0x0F, 0x1F, 0x84, 0x00, 0x00, 0x00, 0x00, 0x00, 0x90,
        ],
    ),
    (
        "unlimitedEquip",
        0x5046550,
        &[
            0x8B, 0x81, 0xC4, 0x00, 0x00, 0x00, 0x83, 0xA1, 0xC4, 0x00, 0x00, 0x00, 0x00, 0x66,
            0x90,
        ],
    ),
    (
        "unlimitedEquip",
        0x456396A,
        &[0xBD, 0xFF, 0xFF, 0xFF, 0xFF, 0x90],
    ),
];

static Y8S1_TOGGLES: &[(&str, u64, &[u8])] = &[
    ("deathless", 0x130B5CC, &[0xE9, 0x2F, 0xEA, 0x8E, 0x02]),
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
        &[
            0x48, 0x8B, 0x81, 0x50, 0x03, 0x00, 0x00, 0x39, 0x34, 0x28, 0x0F, 0x4F, 0x34, 0x28,
            0x39, 0x34, 0x28, 0x66, 0x90,
        ],
    ),
    ("unlimitedEquip", 0x1A42009, &[0x8D, 0x47, 0x00]),
    ("unlimitedEquip", 0x13E51A0, &[0x0F, 0x1F, 0x00]),
    ("unlimitedEquip", 0x1A420A0, &[0x0F, 0x1F, 0x00]),
    (
        "unlimitedEquip",
        0x430CE5A,
        &[
            0x8B, 0xBD, 0xC4, 0x00, 0x00, 0x00, 0xBE, 0xFF, 0xFF, 0xFF, 0xFF, 0x31, 0xC0, 0x0F,
            0x1F, 0x00,
        ],
    ),
    (
        "unlimitedEquip",
        0x430EB30,
        &[
            0x8B, 0xBB, 0xC4, 0x00, 0x00, 0x00, 0x41, 0xBF, 0xFF, 0xFF, 0xFF, 0xFF, 0x31, 0xC0,
            0x0F, 0x1F, 0x40, 0x00,
        ],
    ),
    (
        "unlimitedEquip",
        0x2688F39,
        &[
            0x8B, 0xB5, 0xC4, 0x00, 0x00, 0x00, 0xBB, 0xFF, 0xFF, 0xFF, 0xFF, 0x31, 0xC0, 0x0F,
            0x1F, 0x00,
        ],
    ),
    (
        "unlimitedEquip",
        0x2AD8CBB,
        &[
            0x8B, 0xB5, 0xC4, 0x00, 0x00, 0x00, 0xBB, 0xFF, 0xFF, 0xFF, 0xFF, 0x31, 0xC0, 0x0F,
            0x1F, 0x00,
        ],
    ),
    (
        "unlimitedEquip",
        0x2ADFC23,
        &[
            0x8B, 0xBD, 0xC4, 0x00, 0x00, 0x00, 0x41, 0xBF, 0xFF, 0xFF, 0xFF, 0xFF, 0x31, 0xC0,
            0x0F, 0x1F, 0x40, 0x00,
        ],
    ),
    (
        "unlimitedEquip",
        0x2ED2283,
        &[
            0x8B, 0xBD, 0xC4, 0x00, 0x00, 0x00, 0x41, 0xBF, 0xFF, 0xFF, 0xFF, 0xFF, 0x31, 0xC0,
            0x0F, 0x1F, 0x40, 0x00,
        ],
    ),
    (
        "unlimitedEquip",
        0x37AE362,
        &[
            0x8B, 0xB5, 0xC4, 0x00, 0x00, 0x00, 0x41, 0xBF, 0xFF, 0xFF, 0xFF, 0xFF, 0x31, 0xC0,
            0x0F, 0x1F, 0x40, 0x00,
        ],
    ),
    (
        "unlimitedEquip",
        0x2E99D5D,
        &[
            0x8B, 0x71, 0x6C, 0xBD, 0xFF, 0xFF, 0xFF, 0xFF, 0x31, 0xC0, 0x0F, 0x1F, 0x00,
        ],
    ),
    (
        "unlimitedEquip",
        0x889E62,
        &[
            0x8B, 0x71, 0x64, 0xBD, 0xFF, 0xFF, 0xFF, 0xFF, 0x31, 0xC0, 0x66, 0x90,
        ],
    ),
    (
        "unlimitedEquip",
        0x88BCE0,
        &[
            0x8B, 0x81, 0xC4, 0x00, 0x00, 0x00, 0x83, 0xA1, 0xC4, 0x00, 0x00, 0x00, 0x00, 0x66,
            0x90,
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
        0x3B9AB4E,
        &[0xE9, 0xAD, 0x54, 0xEA, 0x01, 0x90, 0x90],
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
    ("unlimitedAmmo", 0x2C9E046, &[0x82]),
    ("unlimitedAmmo", 0x2C9E225, &[0x0F, 0x1F, 0x00]),
    ("unlimitedAmmo", 0x5688750, &[0xC3]),
    ("disableAI", 0x17DADA0, &[0xC3]),
    ("endMatch", 0x26A8942, &[0x74, 0x3E, 0xB1, 0x01]),
    (
        "unlimitedEquip",
        0x1E339D3,
        &[
            0x48, 0x8B, 0x81, 0x50, 0x03, 0x00, 0x00, 0x42, 0x39, 0x3C, 0x38, 0x42, 0x0F, 0x4F,
            0x3C, 0x38, 0x42, 0x39, 0x3C, 0x38,
        ],
    ),
    ("unlimitedEquip", 0x55454B9, &[0x8D, 0x47, 0x00]),
    ("unlimitedEquip", 0x3312173, &[0x0F, 0x1F, 0x00]),
    (
        "unlimitedEquip",
        0x483A48A,
        &[
            0x8B, 0xBD, 0xC8, 0x00, 0x00, 0x00, 0xBE, 0xFF, 0xFF, 0xFF, 0xFF, 0x31, 0xC0, 0x0F,
            0x1F, 0x00,
        ],
    ),
    (
        "unlimitedEquip",
        0x483C14C,
        &[
            0x8B, 0xBB, 0xC8, 0x00, 0x00, 0x00, 0x41, 0xBF, 0xFF, 0xFF, 0xFF, 0xFF, 0x31, 0xC0,
            0x0F, 0x1F, 0x40, 0x00,
        ],
    ),
    (
        "unlimitedEquip",
        0x4806C29,
        &[
            0x8B, 0xB5, 0xC8, 0x00, 0x00, 0x00, 0xBB, 0xFF, 0xFF, 0xFF, 0xFF, 0x31, 0xC0, 0x0F,
            0x1F, 0x00,
        ],
    ),
    (
        "unlimitedEquip",
        0x555C3FB,
        &[
            0x8B, 0xB5, 0xC8, 0x00, 0x00, 0x00, 0xBB, 0xFF, 0xFF, 0xFF, 0xFF, 0x31, 0xC0, 0x0F,
            0x1F, 0x00,
        ],
    ),
    (
        "unlimitedEquip",
        0x5562D53,
        &[
            0x8B, 0xBD, 0xC8, 0x00, 0x00, 0x00, 0x41, 0xBF, 0xFF, 0xFF, 0xFF, 0xFF, 0x31, 0xC0,
            0x0F, 0x1F, 0x40, 0x00,
        ],
    ),
    (
        "unlimitedEquip",
        0x55658C3,
        &[
            0x8B, 0xBD, 0xC8, 0x00, 0x00, 0x00, 0x41, 0xBF, 0xFF, 0xFF, 0xFF, 0xFF, 0x31, 0xC0,
            0x0F, 0x1F, 0x40, 0x00,
        ],
    ),
    (
        "unlimitedEquip",
        0x557DCD2,
        &[
            0x8B, 0xB5, 0xC8, 0x00, 0x00, 0x00, 0x41, 0xBF, 0xFF, 0xFF, 0xFF, 0xFF, 0x31, 0xC0,
            0x0F, 0x1F, 0x40, 0x00,
        ],
    ),
    (
        "unlimitedEquip",
        0x288F99D,
        &[
            0x8B, 0x71, 0x70, 0xBD, 0xFF, 0xFF, 0xFF, 0xFF, 0x31, 0xC0, 0x0F, 0x1F, 0x00,
        ],
    ),
    (
        "unlimitedEquip",
        0x553B5E2,
        &[
            0x8B, 0x71, 0x68, 0xBD, 0xFF, 0xFF, 0xFF, 0xFF, 0x31, 0xC0, 0x66, 0x90,
        ],
    ),
    (
        "unlimitedEquip",
        0x553D310,
        &[
            0x8B, 0x81, 0xC8, 0x00, 0x00, 0x00, 0x83, 0xA1, 0xC8, 0x00, 0x00, 0x00, 0x00, 0x66,
            0x90,
        ],
    ),
];

static Y8S3_TOGGLES: &[(&str, u64, &[u8])] = &[
    (
        "deathless",
        0x164F2C,
        &[0xE9, 0xCF, 0xB0, 0xDC, 0x04, 0x90, 0x90],
    ),
    ("unlimitedAmmo", 0x379136F, &[0x0F, 0x1F, 0x00]),
    (
        "unlimitedAmmo",
        0x37A18DD,
        &[0x66, 0x0F, 0x1F, 0x44, 0x00, 0x00],
    ),
    (
        "unlimitedAmmo",
        0x37A1A41,
        &[0x66, 0x0F, 0x1F, 0x44, 0x00, 0x00],
    ),
    ("unlimitedAmmo", 0x37B801A, &[0x66, 0x90]),
    ("unlimitedAmmo", 0x37B2337, &[0x82]),
    ("unlimitedAmmo", 0x37B2524, &[0x66, 0x90]),
    ("unlimitedAmmo", 0x37C7740, &[0xC3]),
    ("disableAI", 0x2941270, &[0xC3]),
    ("endMatch", 0xC99C63, &[0x74, 0x3D, 0xB1, 0x01]),
    (
        "unlimitedEquip",
        0x3E86152,
        &[
            0x48, 0x8B, 0x83, 0xD8, 0x01, 0x00, 0x00, 0x42, 0x39, 0x2C, 0x30, 0x42, 0x0F, 0x4F,
            0x2C, 0x30, 0x42, 0x39, 0x2C, 0x30,
        ],
    ),
    ("unlimitedEquip", 0x3ECFC49, &[0x8D, 0x47, 0x00]),
    ("unlimitedEquip", 0x3ECFCE0, &[0x0F, 0x1F, 0x00]),
    ("unlimitedEquip", 0x2EEEBE0, &[0x0F, 0x1F, 0x00]),
    (
        "unlimitedEquip",
        0x411355E,
        &[
            0x8B, 0xBD, 0xD0, 0x00, 0x00, 0x00, 0xBE, 0xFF, 0xFF, 0xFF, 0xFF, 0x31, 0xC0, 0x0F,
            0x1F, 0x00,
        ],
    ),
    (
        "unlimitedEquip",
        0x41156E7,
        &[
            0x8B, 0xBB, 0xD0, 0x00, 0x00, 0x00, 0x41, 0xBF, 0xFF, 0xFF, 0xFF, 0xFF, 0x31, 0xC0,
            0x0F, 0x1F, 0x40, 0x00,
        ],
    ),
    (
        "unlimitedEquip",
        0x9B200C,
        &[
            0x8B, 0xB5, 0xD0, 0x00, 0x00, 0x00, 0xBF, 0xFF, 0xFF, 0xFF, 0xFF, 0x31, 0xC0, 0x0F,
            0x1F, 0x00,
        ],
    ),
    (
        "unlimitedEquip",
        0x51FEDDE,
        &[
            0x8B, 0xBD, 0xD0, 0x00, 0x00, 0x00, 0xBE, 0xFF, 0xFF, 0xFF, 0xFF, 0x31, 0xC0, 0x0F,
            0x1F, 0x00,
        ],
    ),
    (
        "unlimitedEquip",
        0x5205873,
        &[
            0x8B, 0xBB, 0xD0, 0x00, 0x00, 0x00, 0x41, 0xBF, 0xFF, 0xFF, 0xFF, 0xFF, 0x31, 0xC0,
            0x0F, 0x1F, 0x40, 0x00,
        ],
    ),
    (
        "unlimitedEquip",
        0x5207AE3,
        &[
            0x8B, 0xBB, 0xD0, 0x00, 0x00, 0x00, 0x41, 0xBF, 0xFF, 0xFF, 0xFF, 0xFF, 0x31, 0xC0,
            0x0F, 0x1F, 0x40, 0x00,
        ],
    ),
    (
        "unlimitedEquip",
        0x517E13D,
        &[
            0x8B, 0x71, 0x78, 0xBD, 0xFF, 0xFF, 0xFF, 0xFF, 0x31, 0xC0, 0x0F, 0x1F, 0x00,
        ],
    ),
    (
        "unlimitedEquip",
        0x3EC58E2,
        &[
            0x8B, 0x71, 0x70, 0xBD, 0xFF, 0xFF, 0xFF, 0xFF, 0x31, 0xC0, 0x66, 0x90,
        ],
    ),
    (
        "unlimitedEquip",
        0x3EC7160,
        &[
            0x8B, 0x81, 0xC8, 0x00, 0x00, 0x00, 0x83, 0xA1, 0xC8, 0x00, 0x00, 0x00, 0x00, 0x66,
            0x90,
        ],
    ),
    ("unlimitedEquip", 0x2EEECD9, &[0x8D, 0x47, 0x00]),
];

static Y8S4_TOGGLES: &[(&str, u64, &[u8])] = &[
    (
        "deathless",
        0x32AF107,
        &[0xE9, 0xF4, 0x6E, 0xAC, 0xFF, 0x90],
    ),
    ("unlimitedAmmo", 0x1247B00, &[0x0F, 0x1F, 0x44, 0x00, 0x00]),
    (
        "unlimitedAmmo",
        0x1257F17,
        &[0x66, 0x0F, 0x1F, 0x44, 0x00, 0x00],
    ),
    (
        "unlimitedAmmo",
        0x12580F9,
        &[0x66, 0x0F, 0x1F, 0x44, 0x00, 0x00],
    ),
    ("unlimitedAmmo", 0x126C8D8, &[0x66, 0x90]),
    ("unlimitedAmmo", 0x1266CC7, &[0x82]),
    ("unlimitedAmmo", 0x1266E2B, &[0x66, 0x90]),
    ("unlimitedAmmo", 0x127D8D0, &[0xC3]),
    ("endMatch", 0x1427F0C, &[0x75, 0x44, 0xB1, 0x01]),
    (
        "unlimitedEquip",
        0x4DE40D0,
        &[
            0x48, 0x8B, 0x81, 0xD8, 0x01, 0x00, 0x00, 0x42, 0x39, 0x2C, 0x30, 0x42, 0x0F, 0x4F,
            0x2C, 0x30, 0x42, 0x39, 0x2C, 0x30,
        ],
    ),
    ("unlimitedEquip", 0x3B154E9, &[0x8D, 0x47, 0x00]),
    ("unlimitedEquip", 0x3B15580, &[0x0F, 0x1F, 0x00]),
    ("unlimitedEquip", 0x1330F90, &[0x0F, 0x1F, 0x00]),
    (
        "unlimitedEquip",
        0x2DA10FE,
        &[
            0x8B, 0xBD, 0xD0, 0x00, 0x00, 0x00, 0xBE, 0xFF, 0xFF, 0xFF, 0xFF, 0x31, 0xC0, 0x0F,
            0x1F, 0x00,
        ],
    ),
    (
        "unlimitedEquip",
        0x2DA2EBD,
        &[
            0x8B, 0xBB, 0xD0, 0x00, 0x00, 0x00, 0x41, 0xBF, 0xFF, 0xFF, 0xFF, 0xFF, 0x31, 0xC0,
            0x0F, 0x1F, 0x40, 0x00,
        ],
    ),
    (
        "unlimitedEquip",
        0x29C5439,
        &[
            0x8B, 0xB5, 0xD0, 0x00, 0x00, 0x00, 0xBB, 0xFF, 0xFF, 0xFF, 0xFF, 0x31, 0xC0, 0x0F,
            0x1F, 0x00,
        ],
    ),
    (
        "unlimitedEquip",
        0x3B2AE0C,
        &[
            0x8B, 0xBD, 0xD0, 0x00, 0x00, 0x00, 0x41, 0xBF, 0xFF, 0xFF, 0xFF, 0xFF, 0x31, 0xC0,
            0x0F, 0x1F, 0x40, 0x00,
        ],
    ),
    (
        "unlimitedEquip",
        0x580D653,
        &[
            0x8B, 0xBD, 0xD0, 0x00, 0x00, 0x00, 0x41, 0xBF, 0xFF, 0xFF, 0xFF, 0xFF, 0x31, 0xC0,
            0x0F, 0x1F, 0x40, 0x00,
        ],
    ),
    (
        "unlimitedEquip",
        0x580F9E3,
        &[
            0x8B, 0xBD, 0xD0, 0x00, 0x00, 0x00, 0x41, 0xBF, 0xFF, 0xFF, 0xFF, 0xFF, 0x31, 0xC0,
            0x0F, 0x1F, 0x40, 0x00,
        ],
    ),
    (
        "unlimitedEquip",
        0x34C9B3D,
        &[
            0x8B, 0x71, 0x78, 0xBD, 0xFF, 0xFF, 0xFF, 0xFF, 0x31, 0xC0, 0x0F, 0x1F, 0x00,
        ],
    ),
    (
        "unlimitedEquip",
        0xE0B3B2,
        &[
            0x8B, 0x71, 0x70, 0xBD, 0xFF, 0xFF, 0xFF, 0xFF, 0x31, 0xC0, 0x66, 0x90,
        ],
    ),
    (
        "unlimitedEquip",
        0xE0D530,
        &[
            0x8B, 0x81, 0xC8, 0x00, 0x00, 0x00, 0x83, 0xA1, 0xC8, 0x00, 0x00, 0x00, 0x00, 0x66,
            0x90,
        ],
    ),
    ("unlimitedEquip", 0x1331089, &[0x8D, 0x47, 0x00]),
];

static Y9S1_TOGGLES: &[(&str, u64, &[u8])] = &[
    ("deathless", 0x5367EAF, &[0xE9, 0x4C, 0x21, 0x7B, 0xFE]),
    ("unlimitedAmmo", 0x65FE593, &[0x0F, 0x1F, 0x00]),
    (
        "unlimitedAmmo",
        0x661B6B4,
        &[0x66, 0x0F, 0x1F, 0x44, 0x00, 0x00],
    ),
    (
        "unlimitedAmmo",
        0x661B728,
        &[0x66, 0x0F, 0x1F, 0x44, 0x00, 0x00],
    ),
    ("unlimitedAmmo", 0x6645DBC, &[0x0F, 0x1F, 0x00]),
    ("unlimitedAmmo", 0x663D087, &[0x82]),
    ("unlimitedAmmo", 0x663D12B, &[0x66, 0x90]),
    ("unlimitedAmmo", 0x6667420, &[0xC3]),
    ("endMatch", 0x36E91A5, &[0x75, 0x3A, 0xB0, 0x01]),
    (
        "unlimitedEquip",
        0x1E61D92,
        &[
            0x48, 0x8B, 0x83, 0xD8, 0x01, 0x00, 0x00, 0x42, 0x39, 0x2C, 0x30, 0x42, 0x0F, 0x4F,
            0x2C, 0x30, 0x42, 0x39, 0x2C, 0x30,
        ],
    ),
    ("unlimitedEquip", 0x52970F9, &[0x8D, 0x47, 0x00]),
    ("unlimitedEquip", 0x5297190, &[0x0F, 0x1F, 0x00]),
    ("unlimitedEquip", 0x377D6D0, &[0x0F, 0x1F, 0x00]),
    (
        "unlimitedEquip",
        0x493029,
        &[
            0x8B, 0xB5, 0xD0, 0x00, 0x00, 0x00, 0xBF, 0xFF, 0xFF, 0xFF, 0xFF, 0x31, 0xC0, 0x0F,
            0x1F, 0x00,
        ],
    ),
    (
        "unlimitedEquip",
        0x481E1F3,
        &[
            0x8B, 0xBB, 0xD0, 0x00, 0x00, 0x00, 0x41, 0xBF, 0xFF, 0xFF, 0xFF, 0xFF, 0x31, 0xC0,
            0x0F, 0x1F, 0x40, 0x00,
        ],
    ),
    (
        "unlimitedEquip",
        0x52B1CB0,
        &[
            0x8B, 0xBE, 0xD0, 0x00, 0x00, 0x00, 0xBB, 0xFF, 0xFF, 0xFF, 0xFF, 0x31, 0xC0, 0x0F,
            0x1F, 0x00,
        ],
    ),
    (
        "unlimitedEquip",
        0x52B88A3,
        &[
            0x8B, 0xBB, 0xD0, 0x00, 0x00, 0x00, 0x41, 0xBF, 0xFF, 0xFF, 0xFF, 0xFF, 0x31, 0xC0,
            0x0F, 0x1F, 0x40, 0x00,
        ],
    ),
    (
        "unlimitedEquip",
        0x66BF041,
        &[
            0x8B, 0xB5, 0xD0, 0x00, 0x00, 0x00, 0xBF, 0xFF, 0xFF, 0xFF, 0xFF, 0x31, 0xC0, 0x0F,
            0x1F, 0x00,
        ],
    ),
    (
        "unlimitedEquip",
        0x66C119D,
        &[
            0x8B, 0xBB, 0xD0, 0x00, 0x00, 0x00, 0x41, 0xBF, 0xFF, 0xFF, 0xFF, 0xFF, 0x31, 0xC0,
            0x0F, 0x1F, 0x40, 0x00,
        ],
    ),
    (
        "unlimitedEquip",
        0x13F83CD,
        &[
            0x8B, 0x71, 0x78, 0xBD, 0xFF, 0xFF, 0xFF, 0xFF, 0x31, 0xC0, 0x0F, 0x1F, 0x00,
        ],
    ),
    (
        "unlimitedEquip",
        0x528C532,
        &[
            0x8B, 0x71, 0x70, 0xBD, 0xFF, 0xFF, 0xFF, 0xFF, 0x31, 0xC0, 0x66, 0x90,
        ],
    ),
    (
        "unlimitedEquip",
        0x528EA40,
        &[
            0x8B, 0x81, 0xC8, 0x00, 0x00, 0x00, 0x83, 0xA1, 0xC8, 0x00, 0x00, 0x00, 0x00, 0x66,
            0x90,
        ],
    ),
    ("unlimitedEquip", 0x377D7C9, &[0x8D, 0x47, 0x00]),
    ("unlimitedEquip", 0x5D519DA, &[0x66, 0x90]),
    ("unlimitedEquip", 0x5D5356C, &[0x66, 0x90]),
];

static Y9S2_TOGGLES: &[(&str, u64, &[u8])] = &[
    (
        "deathless",
        0x520E13A,
        &[
            0x48, 0x89, 0xF1, 0xE8, 0x2E, 0xB9, 0x00, 0x00, 0x84, 0xC0, 0x49, 0xBA, 0x00, 0x00,
            0x00, 0x00, 0xFF, 0x00, 0x00, 0x00, 0x0F, 0x85, 0xAC, 0x3E, 0x88, 0x00, 0x0F, 0x1F,
            0x40, 0x00,
        ],
    ),
    ("unlimitedAmmo", 0x4C11050, &[0x0F, 0x1F, 0x00]),
    (
        "unlimitedAmmo",
        0x4BFC6B4,
        &[0x66, 0x0F, 0x1F, 0x44, 0x00, 0x00],
    ),
    (
        "unlimitedAmmo",
        0x4BFC728,
        &[0x66, 0x0F, 0x1F, 0x44, 0x00, 0x00],
    ),
    ("unlimitedAmmo", 0x4C2414B, &[0x66, 0x90]),
    ("unlimitedAmmo", 0x4C1CE36, &[0x82]),
    ("unlimitedAmmo", 0x4C1D34F, &[0x0F, 0x1F, 0x00]),
    ("unlimitedAmmo", 0x50E7F60, &[0xC3]),
    ("endMatch", 0x529EAE8, &[0xB1, 0x01]),
];

fn shadow_toggles(build: &str) -> &'static [(&'static str, u64, &'static [u8])] {
    match build {
        "Y5S1_14303219" => Y5S1_TOGGLES,
        "Y6S1_15680437" => Y6S1_TOGGLES,
        "Y6S2_15772114" => Y6S2_TOGGLES,
        "Y6S3_35130040" => Y6S3_TOGGLES,
        "Y6S4_37046065" => Y6S4_TOGGLES,
        "Y7S1_39785137" => Y7S1_TOGGLES,
        "Y7S2_43489433" => Y7S2_TOGGLES,
        "Y7S3_47165842" => Y7S3_TOGGLES,
        "Y7S4_50497889" => Y7S4_TOGGLES,
        "Y8S1_55217154" => Y8S1_TOGGLES,
        "Y8S2_58222837" => Y8S2_TOGGLES,
        "Y8S3_62486471" => Y8S3_TOGGLES,
        "Y8S4_67466841" => Y8S4_TOGGLES,
        "Y9S1_69866602" => Y9S1_TOGGLES,
        "Y9S2_72730050" => Y9S2_TOGGLES,
        _ => &[],
    }
}

const MIN_PHASE_MS: u32 = 15000;

fn plain_u32(v: u32) -> u32 {
    v
}

fn plain_u64(q: u64) -> u64 {
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

fn shuffle_bytes(v: u64, p: [usize; 8]) -> u64 {
    let b = v.to_le_bytes();
    u64::from_le_bytes([
        b[p[0]], b[p[1]], b[p[2]], b[p[3]], b[p[4]], b[p[5]], b[p[6]], b[p[7]],
    ])
}

fn y8s3_dec_state(x: u32) -> u32 {
    match (x.rotate_left(7) ^ 0x7B).wrapping_sub(0x73) {
        s @ 6..=8 => s - 1,
        s => s,
    }
}

fn y8s3_enc_handle_uid(uid: u64) -> u64 {
    (shuffle_bytes(uid, [1, 2, 7, 6, 3, 0, 5, 4]) ^ 0x5ADD222034B277B4).wrapping_add(0x77)
}

fn y8s3_dec_settings(q: u64) -> u64 {
    shuffle_bytes(q, [2, 7, 6, 5, 1, 0, 4, 3]).rotate_left(10) ^ 0x4C3D17E661C9DDB5
}

fn y8s3_dec_settings_count(q: u64) -> u64 {
    shuffle_bytes(q, [2, 7, 6, 5, 1, 0, 4, 3]).rotate_left(10) ^ 0x4ED7216EB1D75940
}

fn y8s3_enc_end(v: u32) -> u32 {
    v.rotate_left(30).wrapping_add(0xDC6D2AF7).rotate_left(7)
}

fn y8s3_dec_end(x: u32) -> u32 {
    x.rotate_left(25).wrapping_add(0x2392D509).rotate_left(2)
}

fn y8s3_enc_duration(v: u32) -> u32 {
    let b = v.to_le_bytes();
    u32::from_le_bytes([b[0], b[2], b[3], b[1]]).wrapping_add(0xD54BD509)
}

fn y8s3_dec_duration(x: u32) -> u32 {
    let b = x.wrapping_add(0x2AB42AF7).to_le_bytes();
    u32::from_le_bytes([b[0], b[3], b[1], b[2]])
}

fn y8s3_dec_list(q: u64) -> u64 {
    shuffle_bytes(q ^ 0xE56135039E21E2CB, [6, 0, 1, 4, 2, 7, 5, 3]).rotate_left(20)
}

fn y8s3_dec_count(q: u64) -> u64 {
    shuffle_bytes(q ^ 0x281726E51D661869, [6, 0, 1, 4, 2, 7, 5, 3]).rotate_left(20)
}

fn y8s3_dec_handle(q: u64) -> u64 {
    (q ^ 0xD051F3BC2F545794).rotate_left(19) ^ 0xC263D7F268767A6C
}

fn y8s3_dec_loadout(q: u64) -> u64 {
    shuffle_bytes(
        (q ^ 0xCDD38B44F8D4E59E).rotate_left(17),
        [1, 5, 7, 4, 0, 2, 3, 6],
    )
}

fn y8s3_dec_slot(x: u64) -> u64 {
    x.wrapping_sub(0x2B).rotate_left(40) ^ 0x0CE65B83A8A2BCFC
}

fn y8s3_enc_item(v: u64) -> u64 {
    (v ^ 0xF622D2E61F51A35B).wrapping_add(0x78).rotate_left(45)
}

fn y8s4_dec_state(x: u32) -> u32 {
    let b = x.to_le_bytes();
    match u32::from_le_bytes([b[3], b[1], b[0], b[2]]).wrapping_sub(0x73) ^ 0xA38AE78A {
        s @ 6..=8 => s - 1,
        s => s,
    }
}

fn y8s4_enc_handle_uid(uid: u64) -> u64 {
    uid.wrapping_add(0x51)
}

fn y8s4_enc_object_uid(uid: u64) -> u64 {
    uid.rotate_left(16) ^ 0xFA4E1F18A6AD1F68
}

fn y8s4_dec_settings(q: u64) -> u64 {
    q.wrapping_sub(0x2F)
}

fn y8s4_dec_settings_count(q: u64) -> u64 {
    q.wrapping_add(0x39902BAB62789411)
}

fn y8s4_enc_end(v: u32) -> u32 {
    (v ^ 0xAEA8A80C).swap_bytes().rotate_right(16)
}

fn y8s4_dec_end(x: u32) -> u32 {
    x.swap_bytes().rotate_right(16) ^ 0xAEA8A80C
}

fn y8s4_enc_duration(v: u32) -> u32 {
    let key = image_key() as u32;
    (v ^ key).rotate_left(30) ^ key
}

fn y8s4_dec_duration(x: u32) -> u32 {
    let key = image_key() as u32;
    (x ^ key).rotate_left(2) ^ key
}

fn y8s4_dec_list(q: u64) -> u64 {
    q.rotate_left(10).wrapping_add(image_key()) ^ 0x8906AF93079A1E13
}

fn y8s4_dec_count(q: u64) -> u64 {
    q.rotate_left(10)
        .wrapping_add(image_key())
        .wrapping_sub(0x4EDE4C71EEDB5A5D)
        ^ 0x09ED303757E1C43E
}

fn y8s4_dec_handle(q: u64) -> u64 {
    shuffle_bytes(q.wrapping_add(0x81E5F4A7807F6400), [1, 4, 3, 5, 2, 7, 6, 0]) ^ 0x2653895794D9A795
}

fn y8s4_dec_loadout(q: u64) -> u64 {
    shuffle_bytes(
        shuffle_bytes(q, [1, 5, 6, 3, 2, 4, 0, 7]).rotate_left(5),
        [3, 5, 6, 1, 2, 0, 4, 7],
    )
}

fn y8s4_dec_slot(x: u64) -> u64 {
    shuffle_bytes(x ^ 0x6C6149FB2C5E2202, [3, 6, 2, 7, 0, 1, 5, 4]).wrapping_sub(0x22)
}

fn y8s4_enc_item(v: u64) -> u64 {
    (v.wrapping_sub(image_key()) ^ 0xF41870301A237006).wrapping_add(0x3D)
}

fn y8s4_countdown(key: u64) -> u8 {
    0x11u8.wrapping_sub(key as u8)
}

fn y8s4_watermark(key: u64) -> [u8; 2] {
    let k = key as u8;
    [1u8.wrapping_sub(k), k.wrapping_neg()]
}

fn y9s1_dec_state(x: u32) -> u32 {
    let s = match (image_key() ^ 0x2A) % 6 {
        0 => (x ^ 0x6D).wrapping_sub(0x5C).rotate_left(5),
        1 => (x ^ 0x6D).rotate_left(5).wrapping_sub(0x5C),
        2 => x.wrapping_sub(0x5C).rotate_left(5) ^ 0x6D,
        3 => (x.wrapping_sub(0x5C) ^ 0x6D).rotate_left(5),
        4 => (x.rotate_left(5) ^ 0x6D).wrapping_sub(0x5C),
        _ => x.rotate_left(5).wrapping_sub(0x5C) ^ 0x6D,
    };
    match s {
        s @ 6..=8 => s - 1,
        s => s,
    }
}

fn y9s1_dec_match_index(b: u8) -> u8 {
    b ^ 0x96
}

fn y9s1_dec_match_data(q: u64) -> u64 {
    shuffle_bytes(q, [0, 3, 6, 2, 1, 7, 5, 4]).rotate_right(11) ^ 0xBB508334FB49A92A
}

fn y9s1_dec_match_count(q: u64) -> u64 {
    shuffle_bytes(q, [0, 3, 6, 2, 1, 7, 5, 4]).rotate_right(11) ^ 0x5071489510EEE0DA
}

fn y9s1_enc_mode(h: u64) -> u64 {
    const KEY: u64 = 0x432AA8A1228480B3;
    let base = image_key() << 33;
    match 0x1C0E18DBFF21E75C_u64.wrapping_sub(image_key()) % 6 {
        0 => (h ^ KEY).rotate_right(22).wrapping_sub(base),
        1 => (h.rotate_right(22) ^ KEY).wrapping_sub(base),
        2 => (h.wrapping_sub(base) ^ KEY).rotate_right(22),
        3 => (h ^ KEY).wrapping_sub(base).rotate_right(22),
        4 => h.rotate_right(22).wrapping_sub(base) ^ KEY,
        _ => h.wrapping_sub(base).rotate_right(22) ^ KEY,
    }
}

fn y9s1_enc_map(h: u64) -> u64 {
    h.wrapping_add(0x58)
}

fn y9s1_enc_playlist(uid: u64) -> u64 {
    (shuffle_bytes(uid, [1, 6, 3, 5, 7, 4, 0, 2]) ^ 0xD9612151BA343B04).wrapping_add(0x66)
}

fn y9s1_dec_match_tod(q: u64) -> u64 {
    q.wrapping_sub(0x53)
}

fn y9s1_dec_match_settings(q: u64) -> u64 {
    const P: [usize; 8] = [1, 5, 3, 0, 7, 4, 2, 6];
    match (image_key() + 0xB49CD5CA) % 6 {
        0 => shuffle_bytes(q.wrapping_sub(0x2D), P).wrapping_sub(0x64),
        1 | 4 => shuffle_bytes(q.wrapping_sub(0x91), P),
        2 | 3 => shuffle_bytes(q, P).wrapping_sub(0x91),
        _ => shuffle_bytes(q.wrapping_sub(0x64), P).wrapping_sub(0x2D),
    }
}

fn y9s1_dec_match_settings_count(q: u64) -> u64 {
    const P: [usize; 8] = [1, 5, 3, 0, 7, 4, 2, 6];
    match (image_key() + 0xB49CD5CA) % 6 {
        0 => shuffle_bytes(q.wrapping_add(0xFF69F333127EC532), P).wrapping_add(0x56F5E65E89C88417),
        1 | 4 => shuffle_bytes(q.wrapping_add(0x565FD9919C474949), P),
        2 | 3 => shuffle_bytes(q, P).wrapping_add(0x565FD9919C474949),
        _ => shuffle_bytes(q.wrapping_add(0x56F5E65E89C88417), P).wrapping_add(0xFF69F333127EC532),
    }
}

fn y9s1_enc_match_settings_count(v: u64) -> u64 {
    const P: [usize; 8] = [3, 0, 6, 2, 5, 1, 7, 4];
    match (image_key() + 0xB49CD5CA) % 6 {
        0 => shuffle_bytes(v.wrapping_sub(0x56F5E65E89C88417), P).wrapping_sub(0xFF69F333127EC532),
        1 | 4 => shuffle_bytes(v, P).wrapping_sub(0x565FD9919C474949),
        2 | 3 => shuffle_bytes(v.wrapping_sub(0x565FD9919C474949), P),
        _ => shuffle_bytes(v.wrapping_sub(0xFF69F333127EC532), P).wrapping_sub(0x56F5E65E89C88417),
    }
}

fn y9s1_enc_handle_uid(uid: u64) -> u64 {
    (uid ^ 0x1CBED4498BB197CF).wrapping_sub(image_key() << 33) ^ 0x2E
}

fn y9s1_enc_object_uid(uid: u64) -> u64 {
    uid.wrapping_add(0x4C)
}

fn y9s1_dec_settings(q: u64) -> u64 {
    let key = image_key();
    let high = key << 33;
    match (0x04B642D1E34B5FF0 ^ key) % 6 {
        0 => q.wrapping_sub(key).rotate_left(43).wrapping_add(high),
        1 | 4 => q.wrapping_sub(key).wrapping_add(high).rotate_left(43),
        2 | 3 => q.rotate_left(43).wrapping_add(high).wrapping_sub(key),
        _ => q.wrapping_add(high).rotate_left(43).wrapping_sub(key),
    }
}

fn y9s1_dec_settings_count(q: u64) -> u64 {
    let key = image_key();
    let high = key << 33;
    match (0x04B642D1E34B5FF0 ^ key) % 6 {
        0 => q
            .wrapping_add(0xEC05C8D37E1F3664)
            .wrapping_sub(key)
            .rotate_left(43)
            .wrapping_add(high)
            .wrapping_add(0xA5EDB65987A04533),
        1 | 4 => q
            .wrapping_add(0x91F37F2D05BF7B97)
            .wrapping_add(high)
            .wrapping_sub(key)
            .rotate_left(43),
        2 | 3 => q
            .rotate_left(43)
            .wrapping_add(0x91F37F2D05BF7B97)
            .wrapping_add(high)
            .wrapping_sub(key),
        _ => q
            .wrapping_add(high)
            .wrapping_add(0xA5EDB65987A04533)
            .rotate_left(43)
            .wrapping_add(0xEC05C8D37E1F3664)
            .wrapping_sub(key),
    }
}

fn y9s1_countdown(key: u64) -> u8 {
    [0x7E, 0xBA, 0x4B, 0xAB, 0xB4, 0x87][((0x66788E14E052ADB8 ^ key) % 6) as usize]
}

fn y9s1_unfrozen(key: u64) -> u32 {
    key as u32 ^ 0x3FEFE9B2
}

fn y9s1_enc_end(v: u32) -> u32 {
    let b = v.to_le_bytes();
    (u32::from_le_bytes([b[0], b[2], b[1], b[3]]) ^ 0x5D4E6117).wrapping_sub(image_key() as u32)
}

fn y9s1_dec_end(x: u32) -> u32 {
    let b = (x.wrapping_add(image_key() as u32) ^ 0x5D4E6117).to_le_bytes();
    u32::from_le_bytes([b[0], b[2], b[1], b[3]])
}

fn y9s1_enc_duration(v: u32) -> u32 {
    let key = image_key();
    let k = key as u32;
    match 0x086A0851D8CE4398u64.wrapping_add(key) % 6 {
        0 => v.rotate_left(9).wrapping_sub(k).rotate_left(19),
        1 | 4 => v.wrapping_sub(k).rotate_left(28),
        2 | 3 => v.rotate_left(28).wrapping_sub(k),
        _ => v.rotate_left(19).wrapping_sub(k).rotate_left(9),
    }
}

fn y9s1_dec_duration(x: u32) -> u32 {
    let key = image_key();
    let k = key as u32;
    match 0x086A0851D8CE4398u64.wrapping_add(key) % 6 {
        0 => x.rotate_left(13).wrapping_add(k).rotate_left(23),
        1 | 4 => x.rotate_left(4).wrapping_add(k),
        2 | 3 => x.wrapping_add(k).rotate_left(4),
        _ => x.rotate_left(23).wrapping_add(k).rotate_left(13),
    }
}

fn y9s1_dec_list(q: u64) -> u64 {
    let key = image_key();
    match 0x79A32FB175E8BB56u64.wrapping_sub(key) % 6 {
        1 | 4 => q.wrapping_add(key << 34).rotate_left(12),
        2 | 3 => q.rotate_left(12).wrapping_add(key << 34),
        _ => q
            .wrapping_add(key << 33)
            .rotate_left(12)
            .wrapping_add(key << 33),
    }
}

fn y9s1_dec_count(q: u64) -> u64 {
    let key = image_key();
    match 0x79A32FB175E8BB56u64.wrapping_sub(key) % 6 {
        0 => q
            .wrapping_add(key << 33)
            .wrapping_add(0x7DEA1812B9854EB0)
            .rotate_left(12)
            .wrapping_add(key << 33)
            .wrapping_add(0x3D81C43A92E41B3B),
        1 | 4 => q
            .wrapping_add(key << 34)
            .wrapping_add(0xBB6BDC4D4C6969EB)
            .rotate_left(12),
        2 | 3 => q
            .rotate_left(12)
            .wrapping_add(key << 34)
            .wrapping_add(0xBB6BDC4D4C6969EB),
        _ => q
            .wrapping_add(key << 33)
            .wrapping_add(0x3D81C43A92E41B3B)
            .rotate_left(12)
            .wrapping_add(key << 33)
            .wrapping_add(0x7DEA1812B9854EB0),
    }
}

fn y9s1_dec_handle(q: u64) -> u64 {
    q ^ (image_key() << 33) ^ 0x767DAF54C4721A3D
}

fn y9s1_dec_loadout(q: u64) -> u64 {
    const P: [usize; 8] = [6, 3, 0, 1, 2, 7, 5, 4];
    const X: u64 = 0x3D3077367397BE10;
    let key = image_key();
    match 0x179450143FA7F5F0u64.wrapping_sub(key) % 6 {
        0 => shuffle_bytes((q ^ X).wrapping_sub(key), P),
        1 => shuffle_bytes(q ^ X, P).wrapping_sub(key),
        2 => shuffle_bytes(q.wrapping_sub(key), P) ^ X,
        3 => shuffle_bytes(q.wrapping_sub(key) ^ X, P),
        4 => (shuffle_bytes(q, P) ^ X).wrapping_sub(key),
        _ => shuffle_bytes(q, P).wrapping_sub(key) ^ X,
    }
}

fn y9s1_dec_slot(x: u64) -> u64 {
    const P: [usize; 8] = [2, 6, 5, 0, 1, 3, 4, 7];
    const X: u64 = 0xC96A5A187A319B3E;
    match 0xC96A5A18CBD4F122u64.wrapping_add(image_key()) % 6 {
        0 => shuffle_bytes(x, P).rotate_left(1) ^ X,
        1 => (shuffle_bytes(x, P) ^ X).rotate_left(1),
        2 => shuffle_bytes(x.rotate_left(1) ^ X, P),
        3 => shuffle_bytes(x.rotate_left(1), P) ^ X,
        4 => shuffle_bytes(x ^ X, P).rotate_left(1),
        _ => shuffle_bytes((x ^ X).rotate_left(1), P),
    }
}

fn y9s1_enc_item(v: u64) -> u64 {
    const P: [usize; 8] = [2, 4, 3, 0, 1, 5, 6, 7];
    const X: u64 = 0x6004B93971212784;
    match (0x6004B939A9DBB76Eu64 ^ image_key()) % 6 {
        0 => (shuffle_bytes(v, P) ^ X).rotate_left(8),
        1 => shuffle_bytes(v ^ X, P).rotate_left(8),
        2 => shuffle_bytes(v.rotate_left(8), P) ^ X,
        3 => shuffle_bytes(v, P).rotate_left(8) ^ X,
        4 => shuffle_bytes((v ^ X).rotate_left(8), P),
        _ => shuffle_bytes(v.rotate_left(8) ^ X, P),
    }
}

fn y9s1_watermark(key: u64) -> [u8; 2] {
    let k = key as u8;
    if key & 1 == 1 {
        [k.wrapping_add(0x80), k]
    } else {
        [k.wrapping_add(1).rotate_right(1), k.rotate_right(1)]
    }
}

fn y9s2_dec_state(x: u32) -> u32 {
    let s = match !image_key() & 3 {
        0 => (x ^ 0x510ED45B).rotate_left(28) ^ 0x73C51DA8,
        1 => (x ^ 0x22CBC9F3).rotate_left(28),
        _ => x.rotate_left(28) ^ 0x22CBC9F3,
    };
    match s {
        s @ 6..=8 => s - 1,
        s => s,
    }
}

fn y9s2_dec_match_index(b: u8) -> u8 {
    b.wrapping_sub(0x4B) ^ 4
}

fn y9s2_dec_match_data(q: u64) -> u64 {
    let key = image_key();
    if key & 1 == 0 {
        q.wrapping_sub(key)
            .rotate_left(48)
            .wrapping_add(0xB24B0ADA19084558)
    } else {
        q.wrapping_sub(key)
            .wrapping_add(0xB24B0ADA19084558)
            .rotate_left(48)
    }
}

fn y9s2_dec_match_count(q: u64) -> u64 {
    let key = image_key();
    if key & 1 == 0 {
        q.wrapping_sub(key)
            .wrapping_add(0x90193C73ED6ADB25)
            .rotate_left(48)
            .wrapping_add(0xEB5CA2460323C8AA)
    } else {
        q.wrapping_sub(key)
            .wrapping_add(0x7B75DEB9F08EA3CF)
            .rotate_left(48)
    }
}

fn y9s2_enc_playlist(uid: u64) -> u64 {
    shuffle_bytes(
        uid.wrapping_sub(image_key() << 33),
        [2, 5, 4, 3, 7, 1, 6, 0],
    )
}

fn y9s2_enc_mode(h: u64) -> u64 {
    const KEY: u64 = 0x72B8D855BCF70F49;
    let key = image_key();
    match 0x3B43AFAF2207D0D2_u64.wrapping_sub(key) % 3 {
        0 => (h ^ KEY).wrapping_add(key).rotate_left(35),
        1 => (h.wrapping_add(key) ^ KEY).rotate_left(35),
        _ => (h.rotate_left(35) ^ KEY).wrapping_add(key),
    }
}

fn y9s2_enc_map(h: u64) -> u64 {
    const A: u64 = 0x997A438E61922FAB;
    const B: u64 = 0x6722B80A04C096F3;
    match image_key().wrapping_add(0x01A7047B9AAD46AA) % 5 {
        0 => (h ^ B).rotate_left(12).wrapping_sub(A),
        1 => (h.rotate_left(12) ^ B).wrapping_sub(A),
        2 => (h.wrapping_sub(A) ^ B).rotate_left(12),
        3 => (h ^ B).wrapping_sub(A).rotate_left(12),
        _ => h.rotate_left(12).wrapping_sub(A) ^ B,
    }
}

fn y9s2_dec_match_tod(q: u64) -> u64 {
    const P: u64 = 0xC32EB3E79DCDB02E;
    let key = image_key();
    let high = key << 33;
    match 0xD98F382F9950F1E4_u64.wrapping_add(key) % 5 {
        0 => q.wrapping_add(high).rotate_left(22).wrapping_add(P),
        1 | 4 => q.wrapping_add(high).wrapping_add(P).rotate_left(22),
        _ => q.rotate_left(22).wrapping_add(high).wrapping_add(P),
    }
}

fn y9s2_dec_match_settings(q: u64) -> u64 {
    const S: [usize; 8] = [3, 2, 7, 6, 4, 5, 0, 1];
    const X: u64 = 0x47C294D6F2502EA1;
    const A: u64 = 0xB8D54BBE97F7FA69;
    match 0x00E820971716EB48_u64.wrapping_sub(image_key()) % 3 {
        0 => (shuffle_bytes(q, S) ^ X).wrapping_add(A),
        1 => shuffle_bytes(q, S).wrapping_add(A) ^ X,
        _ => shuffle_bytes((q ^ X).wrapping_add(A), S),
    }
}

fn y9s2_dec_match_settings_count(q: u64) -> u64 {
    const S: [usize; 8] = [3, 2, 7, 6, 4, 5, 0, 1];
    const X: u64 = 0x40B523C7AE08BD14;
    const A: u64 = 0xB3092B42D7F16326;
    match 0x00E820971716EB48_u64.wrapping_sub(image_key()) % 3 {
        0 => (shuffle_bytes(q, S) ^ X).wrapping_add(A),
        1 => shuffle_bytes(q, S).wrapping_add(A) ^ X,
        _ => shuffle_bytes((q ^ X).wrapping_add(A), S),
    }
}

fn y9s2_enc_match_settings_count(v: u64) -> u64 {
    const S: [usize; 8] = [6, 7, 1, 0, 4, 5, 3, 2];
    const X: u64 = 0x40B523C7AE08BD14;
    const A: u64 = 0xB3092B42D7F16326;
    match 0x00E820971716EB48_u64.wrapping_sub(image_key()) % 3 {
        0 => shuffle_bytes(v.wrapping_sub(A) ^ X, S),
        1 => shuffle_bytes((v ^ X).wrapping_sub(A), S),
        _ => shuffle_bytes(v, S).wrapping_sub(A) ^ X,
    }
}

fn y9s2_enc_handle_uid(uid: u64) -> u64 {
    shuffle_bytes(uid, [2, 1, 3, 4, 7, 6, 5, 0])
}

fn y9s2_enc_object_uid(uid: u64) -> u64 {
    const P: [usize; 8] = [4, 5, 3, 1, 0, 7, 2, 6];
    const C: u64 = 0x35F4442EA3C85D95;
    let key = image_key();
    match 0x457E356A3366571C_u64.wrapping_add(key) % 5 {
        0 | 3 => shuffle_bytes(uid, P).wrapping_add(C).wrapping_add(key),
        1 => shuffle_bytes(uid.wrapping_add(key), P).wrapping_add(C),
        2 => shuffle_bytes(uid.wrapping_add(C), P).wrapping_add(key),
        _ => shuffle_bytes(uid.wrapping_add(key).wrapping_add(C), P),
    }
}

fn y9s2_dec_settings(q: u64) -> u64 {
    const P: [usize; 8] = [3, 4, 2, 0, 5, 1, 6, 7];
    const C: u64 = 0xC48CB2896FA84E13;
    let key = image_key();
    let ks = key << 33;
    match (key & 3) ^ 1 {
        0 => shuffle_bytes(q, P).wrapping_add(C) ^ ks,
        1 => (shuffle_bytes(q, P) ^ ks).wrapping_add(C),
        2 => shuffle_bytes(q.wrapping_add(C) ^ ks, P),
        _ => shuffle_bytes(q.wrapping_add(C), P) ^ ks,
    }
}

fn y9s2_dec_settings_count(q: u64) -> u64 {
    const P: [usize; 8] = [3, 4, 2, 0, 5, 1, 6, 7];
    const C: u64 = 0xF5B3D4328B8B38C3;
    let key = image_key();
    let ks = (key << 33).wrapping_add(0x1427BD32086A1F9F);
    match (key & 3) ^ 1 {
        0 => shuffle_bytes(q, P).wrapping_add(C) ^ ks,
        1 => (shuffle_bytes(q, P) ^ ks).wrapping_add(C),
        2 => shuffle_bytes(q.wrapping_add(C) ^ ks, P),
        _ => shuffle_bytes(q.wrapping_add(C), P) ^ ks,
    }
}

fn y9s2_live(key: u64) -> u32 {
    (key as u32).wrapping_sub(0x0CE651B7)
}

fn y9s2_countdown(key: u64) -> u8 {
    let k = key as u8;
    match 0x83BCEB0EF01D1FFAu64.wrapping_sub(key) % 3 {
        2 => 0,
        _ => k.wrapping_neg() ^ k,
    }
}

fn y9s2_match_over(key: u64) -> u8 {
    [0x89, 0xB2, 0xC6, 0x35, 0x03][(0x17883E3055DF1BF0u64.wrapping_add(key) % 5) as usize]
}

fn y9s2_unfrozen(key: u64) -> u32 {
    const X: u32 = 0x692A8140;
    let k = key as u32;
    match !key & 3 {
        0 => k.wrapping_neg() ^ X,
        1 => k.wrapping_neg().rotate_left(24) ^ X,
        2 => X.rotate_left(24).wrapping_sub(k),
        _ => X.wrapping_sub(k),
    }
}

fn y9s2_display(key: u64) -> [u8; 2] {
    let k = key as u8;
    if key & 1 == 0 {
        [
            (k ^ 1).wrapping_add(0xA8) ^ 0xD9,
            k.wrapping_add(0xA8) ^ 0xD9,
        ]
    } else {
        [k ^ 0x70, k ^ 0x71]
    }
}

fn y9s2_enc_end(v: u32) -> u32 {
    let mix = |x: u32| {
        let b = x.to_le_bytes();
        u32::from_le_bytes([b[0], b[3], b[1], b[2]])
    };
    match 0xE64A931BEBB3B8D4u64.wrapping_add(image_key()) % 5 {
        0 => mix(v).wrapping_add(0x88AD8314) ^ 0x4D,
        1 => mix(v.wrapping_add(0x88AD8314)) ^ 0x4D,
        4 => mix(v.wrapping_add(0x88AD8314) ^ 0x4D),
        _ => (mix(v) ^ 0x4D).wrapping_add(0x88AD8314),
    }
}

fn y9s2_dec_end(x: u32) -> u32 {
    let unmix = |x: u32| {
        let b = x.to_le_bytes();
        u32::from_le_bytes([b[0], b[2], b[3], b[1]])
    };
    match 0xE64A931BEBB3B8D4u64.wrapping_add(image_key()) % 5 {
        0 => unmix((x ^ 0x4D).wrapping_sub(0x88AD8314)),
        1 => unmix(x ^ 0x4D).wrapping_sub(0x88AD8314),
        4 => (unmix(x) ^ 0x4D).wrapping_sub(0x88AD8314),
        _ => unmix(x.wrapping_sub(0x88AD8314) ^ 0x4D),
    }
}

fn y9s2_enc_duration(v: u32) -> u32 {
    (v.rotate_left(8) ^ 0x27).rotate_left(2)
}

fn y9s2_dec_duration(x: u32) -> u32 {
    (x.rotate_left(30) ^ 0x27).rotate_left(24)
}

fn y9s2_dec_list(q: u64) -> u64 {
    const A: u64 = 0xC83F7D5E5EA298A6;
    const KA: u64 = 0x627706FEA2783C4F;
    const KB: u64 = 0xD75773E824A09958;
    match (image_key() & 3) ^ 1 {
        0 => (q ^ KB).wrapping_sub(A) ^ KA,
        1 => (q ^ KB ^ KA).wrapping_sub(A),
        _ => q.wrapping_sub(A) ^ KA ^ KB,
    }
}

fn y9s2_dec_count(q: u64) -> u64 {
    const A: u64 = 0x61212BFA5766DEDC;
    const KA: u64 = 0x2E74056894A5FEBC;
    const KB: u64 = 0x1ECA992BF0908650;
    match (image_key() & 3) ^ 1 {
        0 => (q ^ KB).wrapping_sub(A) ^ KA,
        1 => (q ^ KB ^ KA).wrapping_sub(A),
        _ => q.wrapping_sub(A) ^ KA ^ KB,
    }
}

fn y9s2_dec_handle(q: u64) -> u64 {
    const C: u64 = 0xA43F7333AE432FA0;
    match 0xA43F7333AE432F8Fu64.wrapping_sub(image_key()) % 6 {
        0 => q.rotate_left(29).wrapping_add(C).rotate_left(52),
        1 | 4 => q.rotate_left(17).wrapping_add(C),
        2 | 3 => q.wrapping_add(C).rotate_left(17),
        _ => q.rotate_left(52).wrapping_add(C).rotate_left(29),
    }
}

fn y9s2_dec_loadout(q: u64) -> u64 {
    const P: [usize; 8] = [5, 1, 7, 2, 3, 4, 0, 6];
    if image_key() & 1 == 0 {
        shuffle_bytes(q.rotate_left(33), P)
    } else {
        shuffle_bytes(q, P).rotate_left(33)
    }
}

fn y9s2_dec_slot(x: u64) -> u64 {
    const C: u64 = 0xF12B350ACEAF6746;
    let key = image_key();
    match !key & 3 {
        0 => (x.wrapping_add(0x15AC3822EE841AA6) ^ key).wrapping_add(0xDB7EFCE7E02B4CA0),
        1 => x.wrapping_add(C) ^ key,
        _ => (x ^ key).wrapping_add(C),
    }
}

fn y9s2_enc_item(v: u64) -> u64 {
    const C: u64 = 0xC88D39277A4C921D;
    let key = image_key();
    let high = key << 33;
    match 0xFE1BE976624FBF4Eu64.wrapping_add(key) % 5 {
        0 => (v.wrapping_add(C) ^ 0x7E).wrapping_add(high),
        1 | 4 => (v ^ 0x7E).wrapping_add(C).wrapping_add(high),
        _ => v.wrapping_add(C).wrapping_add(high) ^ 0x7E,
    }
}

fn y9s2_watermark(key: u64) -> [u8; 2] {
    let k = key as u8;
    let off = match (key & 3) ^ 1 {
        0 => k.rotate_left(5),
        1 => k.rotate_left(6),
        _ => k,
    };
    [off, off]
}

fn y9s3_dec_state(x: u32) -> u32 {
    const X: u32 = 0x9EA34EB5;
    let swap = |v: u32| {
        let b = v.to_le_bytes();
        u32::from_le_bytes([b[0], b[2], b[1], b[3]])
    };
    let s = match 0xACB9D87946E15444_u64
        .wrapping_add(peb_key())
        .wrapping_sub(image_key())
        % 3
    {
        0 => swap(x ^ X).rotate_left(26),
        1 => swap((x ^ X).rotate_left(26)),
        _ => swap(x).rotate_left(26) ^ X,
    };
    match s {
        s @ 6..=8 => s - 1,
        s => s,
    }
}

fn y9s3_dec_match_index(b: u8) -> u8 {
    b.wrapping_sub(0x0C)
}

fn y9s3_dec_match_data(q: u64) -> u64 {
    const R: u64 = 0x2AF3082A20A38C69;
    const D: u64 = 0x0CD9C4DF731EFA1B;
    match 0x262ACCF553BD7666_u64
        .wrapping_add(peb_key())
        .wrapping_sub(image_key())
        % 3
    {
        0 => (q ^ R).wrapping_add(D).rotate_left(44),
        1 => (q ^ R).rotate_left(44).wrapping_add(D),
        _ => q.wrapping_add(D).rotate_left(44) ^ R,
    }
}

fn y9s3_dec_match_count(q: u64) -> u64 {
    const R: u64 = 0x2002944F32353FDF;
    const D: u64 = 0xC43C194C51FE9020;
    match 0x262ACCF553BD7666_u64
        .wrapping_add(peb_key())
        .wrapping_sub(image_key())
        % 3
    {
        0 => (q ^ R).wrapping_add(D).rotate_left(44),
        1 => (q ^ R).rotate_left(44).wrapping_add(D),
        _ => q.wrapping_add(D).rotate_left(44) ^ R,
    }
}

fn y9s3_enc_playlist(uid: u64) -> u64 {
    const X: u64 = 0x4FEC43E2A3738E5D;
    const A: u64 = 0x8F2EAA554A08268B;
    if image_key().wrapping_add(peb_key() ^ 0xC0C2E9B7E97BA926) & 1 == 0 {
        (uid.wrapping_sub(A).wrapping_sub(0x78) ^ X).wrapping_add(0x78)
    } else {
        (uid ^ X).wrapping_sub(A)
    }
}

fn y9s3_enc_mode(h: u64) -> u64 {
    const A: u64 = 0x3D55D4A292759E72;
    const X: u64 = 0x072198F66D468884;
    match image_key().wrapping_add(peb_key() ^ 0x3A744C54FF3316D3) % 5 {
        0 => (h ^ X).wrapping_sub(A).rotate_right(27),
        1 => (h.wrapping_sub(A) ^ X).rotate_right(27),
        2 => (h.rotate_right(27) ^ X).wrapping_sub(A),
        3 => (h ^ X).rotate_right(27).wrapping_sub(A),
        _ => h.wrapping_sub(A).rotate_right(27) ^ X,
    }
}

fn y9s3_enc_map(h: u64) -> u64 {
    match image_key().wrapping_add(peb_key() ^ 0x1B7C1982) % 3 {
        0 => shuffle_bytes(
            shuffle_bytes(h, [6, 3, 0, 7, 5, 1, 2, 4]).wrapping_add(0xC),
            [0, 2, 6, 3, 4, 1, 5, 7],
        ),
        1 => shuffle_bytes(h.wrapping_add(0xC), [6, 0, 2, 7, 5, 3, 1, 4]),
        _ => shuffle_bytes(h, [5, 3, 0, 7, 1, 2, 6, 4]).wrapping_add(0xC),
    }
}

fn y9s3_dec_match_tod(q: u64) -> u64 {
    const A: u64 = 0x867632B4917A6832;
    const X: u64 = 0x83149A4789B75DCF;
    match image_key().wrapping_add(peb_key() ^ 0x0562A8F318CD3511) % 5 {
        0 => (q.wrapping_add(A).wrapping_add(0x56) ^ X).wrapping_sub(0x56),
        1 | 4 => q.wrapping_add(A) ^ X,
        _ => (q ^ X).wrapping_add(A),
    }
}

fn y9s3_dec_match_settings(q: u64) -> u64 {
    const S: [usize; 8] = [7, 6, 4, 3, 1, 0, 5, 2];
    const A: u64 = 0x83E193F85493CA8F;
    match (image_key() ^ 0x83E193F8E594ED4D_u64.wrapping_sub(peb_key())) % 4 {
        0 => shuffle_bytes(q.wrapping_add(A).rotate_left(25), S),
        1 => shuffle_bytes(q.wrapping_add(A), S).rotate_left(25),
        2 => shuffle_bytes(q.rotate_left(25), S).wrapping_add(A),
        _ => shuffle_bytes(q.rotate_left(25).wrapping_add(A), S),
    }
}

fn y9s3_dec_match_settings_count(q: u64) -> u64 {
    const S: [usize; 8] = [7, 6, 4, 3, 1, 0, 5, 2];
    const A: u64 = 0xDAE8ACA774F63A30;
    match (image_key() ^ 0x83E193F8E594ED4D_u64.wrapping_sub(peb_key())) % 4 {
        0 => shuffle_bytes(q.wrapping_add(A).rotate_left(25), S),
        1 => shuffle_bytes(q.wrapping_add(A), S).rotate_left(25),
        2 => shuffle_bytes(q.rotate_left(25), S).wrapping_add(A),
        _ => shuffle_bytes(q.rotate_left(25).wrapping_add(A), S),
    }
}

fn y9s3_enc_match_settings_count(v: u64) -> u64 {
    const S: [usize; 8] = [5, 4, 7, 3, 2, 6, 1, 0];
    const A: u64 = 0xDAE8ACA774F63A30;
    match (image_key() ^ 0x83E193F8E594ED4D_u64.wrapping_sub(peb_key())) % 4 {
        0 => shuffle_bytes(v, S).rotate_left(39).wrapping_sub(A),
        1 => shuffle_bytes(v.rotate_left(39), S).wrapping_sub(A),
        2 => shuffle_bytes(v.wrapping_sub(A), S).rotate_left(39),
        _ => shuffle_bytes(v, S).wrapping_sub(A).rotate_left(39),
    }
}

fn y9s3_enc_handle_uid(uid: u64) -> u64 {
    uid.wrapping_add(0x50)
}

fn y9s3_enc_object_uid(uid: u64) -> u64 {
    uid.wrapping_add(0x6D)
}

fn y9s3_dec_settings(q: u64) -> u64 {
    const P: [usize; 8] = [2, 6, 5, 1, 0, 3, 4, 7];
    const C: u64 = 0x8DB7E76EA98C875C;
    const X: u64 = 0x407C6B2778B9DC73;
    match 0xCDCB8C4935D03133_u64
        .wrapping_add(peb_key())
        .wrapping_sub(image_key())
        % 4
    {
        0 => shuffle_bytes(q.wrapping_add(C), P) ^ X,
        1 => shuffle_bytes(q.wrapping_add(C) ^ X, P),
        2 => (shuffle_bytes(q, P) ^ X).wrapping_add(C),
        _ => shuffle_bytes(q, P).wrapping_add(C) ^ X,
    }
}

fn y9s3_dec_settings_count(q: u64) -> u64 {
    const P: [usize; 8] = [2, 6, 5, 1, 0, 3, 4, 7];
    const C: u64 = 0x92A8C81EBA0743E7;
    const X: u64 = 0x8C3CD5D2DC929DDF;
    match 0xCDCB8C4935D03133_u64
        .wrapping_add(peb_key())
        .wrapping_sub(image_key())
        % 4
    {
        0 => shuffle_bytes(q.wrapping_add(C), P) ^ X,
        1 => shuffle_bytes(q.wrapping_add(C) ^ X, P),
        2 => (shuffle_bytes(q, P) ^ X).wrapping_add(C),
        _ => shuffle_bytes(q, P).wrapping_add(C) ^ X,
    }
}

fn y9s3_live(key: u64) -> u32 {
    match 0xDA902D5CD52A04B3_u64
        .wrapping_add(peb_key())
        .wrapping_sub(key)
        % 4
    {
        0 => 0x56B1D361,
        1 => 0xA7311B66,
        2 => 0x61D256B0,
        _ => 0x58141A03,
    }
}

fn y9s3_match_over(key: u64) -> u8 {
    match (key ^ 0x1CEAE291828B64B6_u64.wrapping_sub(peb_key())) % 6 {
        1 | 4 | 5 => 0x73,
        _ => 0x81,
    }
}

fn y9s3_unfrozen(key: u64) -> u32 {
    match 0xEB6180817682070F_u64
        .wrapping_add(peb_key())
        .wrapping_sub(key)
        % 6
    {
        0 => 0x4D4AC257,
        1 | 4 => 0x09EDC2F7,
        2 | 3 => 0xEDF7C209,
        _ => 0xAB9AC2A9,
    }
}

fn y9s3_display(key: u64) -> [u8; 2] {
    if 0x35EBCDA907D58311_u64
        .wrapping_add(peb_key())
        .wrapping_sub(key)
        & 1
        == 0
    {
        [0xE0, 0xE1]
    } else {
        [0xF0, 0xF1]
    }
}

fn y9s3_enc_end(v: u32) -> u32 {
    match image_key().wrapping_add(peb_key() ^ 0x33) % 3 {
        0 => v.rotate_left(29).wrapping_add(0x38).rotate_left(22),
        1 => v.wrapping_add(0x38).rotate_left(19),
        _ => v.rotate_left(19).wrapping_add(0x38),
    }
}

fn y9s3_dec_end(x: u32) -> u32 {
    match image_key().wrapping_add(peb_key() ^ 0x33) % 3 {
        0 => x.rotate_left(10).wrapping_sub(0x38).rotate_left(3),
        1 => x.rotate_left(13).wrapping_sub(0x38),
        _ => x.wrapping_sub(0x38).rotate_left(13),
    }
}

fn y9s3_enc_duration(v: u32) -> u32 {
    const X: u32 = 0x6BC543E2;
    let mix = |x: u32| {
        let b = x.to_le_bytes();
        u32::from_le_bytes([b[3], b[1], b[0], b[2]])
    };
    match (image_key() ^ 0xE14238B1ECB55C1C_u64.wrapping_sub(peb_key())) % 3 {
        0 => mix(v.wrapping_add(0x54) ^ X),
        1 => mix((v ^ X).wrapping_add(0x54)),
        _ => mix(v).wrapping_add(0x54) ^ X,
    }
}

fn y9s3_dec_duration(x: u32) -> u32 {
    const X: u32 = 0x6BC543E2;
    let unmix = |x: u32| {
        let b = x.to_le_bytes();
        u32::from_le_bytes([b[2], b[1], b[3], b[0]])
    };
    match (image_key() ^ 0xE14238B1ECB55C1C_u64.wrapping_sub(peb_key())) % 3 {
        0 => (unmix(x) ^ X).wrapping_sub(0x54),
        1 => unmix(x).wrapping_sub(0x54) ^ X,
        _ => unmix((x ^ X).wrapping_sub(0x54)),
    }
}

fn y9s3_dec_list(q: u64) -> u64 {
    q.wrapping_sub(0x27)
}

fn y9s3_dec_count(q: u64) -> u64 {
    q.wrapping_sub(0x0782BA9351CB8531)
}

fn y9s3_dec_handle(q: u64) -> u64 {
    const C: u64 = 0xF54BA462DD6E5637;
    match 0xF54BA462DD6E5625_u64
        .wrapping_add(peb_key())
        .wrapping_sub(image_key())
        % 3
    {
        0 => q.rotate_left(58).wrapping_add(C).rotate_left(44),
        1 => q.rotate_left(38).wrapping_add(C),
        _ => q.wrapping_add(C).rotate_left(38),
    }
}

fn y9s3_dec_loadout(q: u64) -> u64 {
    const P: [usize; 8] = [2, 7, 6, 5, 0, 4, 1, 3];
    const X: u64 = 0xC97B98A3522B3189;
    const A: u64 = 0xE1827EDCBDF76B12;
    if image_key().wrapping_add(peb_key() ^ 0xD7061980370A7B53) & 1 == 0 {
        shuffle_bytes((q ^ X).wrapping_sub(A), P)
    } else {
        shuffle_bytes(q ^ X, P).wrapping_sub(A)
    }
}

fn y9s3_dec_slot(x: u64) -> u64 {
    x.wrapping_sub(0x3B)
}

fn y9s3_enc_item(v: u64) -> u64 {
    v.wrapping_add(2)
}

fn y9s3_watermark(key: u64) -> [u8; 2] {
    let off = match 0xEDD31127168799D5_u64
        .wrapping_add(peb_key())
        .wrapping_sub(key)
        % 4
    {
        0 => 0x45,
        1 => 0x2B,
        _ => 0x4B,
    };
    [off, off]
}

fn y9s4_dec_state(x: u32) -> u32 {
    const A: u32 = 0x76B4F312;
    const X: u32 = 0x9CED3C26;
    let shuffle = |v: u32| {
        let b = v.to_le_bytes();
        u32::from_le_bytes([b[3], b[2], b[0], b[1]])
    };
    let s = match 0x0792DF7DF4D5624C_u64
        .wrapping_add(peb_key())
        .wrapping_sub(image_key())
        % 4
    {
        0 => shuffle(x.wrapping_add(A) ^ X),
        1 => shuffle(x.wrapping_add(A)) ^ X,
        2 => shuffle(x ^ X).wrapping_add(A),
        _ => shuffle((x ^ X).wrapping_add(A)),
    };
    match s {
        s @ 6..=8 => s - 1,
        s => s,
    }
}

fn y9s4_dec_match_index(b: u8) -> u8 {
    if 0x8829E22D0FD6E77A_u64
        .wrapping_add(peb_key())
        .wrapping_sub(image_key())
        & 1
        == 0
    {
        b.rotate_left(3).wrapping_add(0x47)
    } else {
        b.wrapping_add(0x47).rotate_left(3)
    }
}

fn y9s4_dec_match_data(q: u64) -> u64 {
    const C: u64 = 0xDBDAEB88B190AA28;
    const E: u64 = 0x63;
    match image_key().wrapping_add(peb_key() ^ 0xDBDAEB88B190AAF9) % 5 {
        0 => q
            .wrapping_sub(E)
            .rotate_left(47)
            .wrapping_add(C)
            .wrapping_add(E),
        1 | 4 => q.wrapping_add(C).rotate_left(47),
        _ => q.rotate_left(47).wrapping_add(C),
    }
}

fn y9s4_dec_match_count(q: u64) -> u64 {
    const C: u64 = 0xA3919662A7597DAB;
    const E: u64 = 0xE64F730B2C7C7540;
    match image_key().wrapping_add(peb_key() ^ 0xDBDAEB88B190AAF9) % 5 {
        0 => q
            .wrapping_sub(E)
            .rotate_left(47)
            .wrapping_add(C)
            .wrapping_add(E),
        1 | 4 => q.wrapping_add(C).rotate_left(47),
        _ => q.rotate_left(47).wrapping_add(C),
    }
}

fn y9s4_enc_playlist(uid: u64) -> u64 {
    const X: u64 = 0x20EBB0E46F0801EB;
    const A: u64 = 0xE73F9F2F3578320F;
    match image_key().wrapping_add(peb_key() ^ 0xC7D42FCB5A7033DC) % 6 {
        0 => uid.wrapping_sub(A).rotate_right(8) ^ X,
        1 => uid.rotate_right(8).wrapping_sub(A) ^ X,
        2 => (uid ^ X).wrapping_sub(A).rotate_right(8),
        3 => (uid.wrapping_sub(A) ^ X).rotate_right(8),
        4 => (uid.rotate_right(8) ^ X).wrapping_sub(A),
        _ => (uid ^ X).rotate_right(8).wrapping_sub(A),
    }
}

fn y9s4_enc_mode(h: u64) -> u64 {
    match (image_key() ^ 0x558AFDE1_u64.wrapping_sub(peb_key())) % 3 {
        0 => shuffle_bytes(
            shuffle_bytes(h, [3, 2, 0, 4, 1, 6, 7, 5]).rotate_right(46),
            [0, 4, 7, 2, 6, 3, 5, 1],
        ),
        1 => shuffle_bytes(h.rotate_right(46), [3, 1, 5, 0, 7, 4, 6, 2]),
        _ => shuffle_bytes(h, [2, 7, 0, 6, 4, 5, 1, 3]).rotate_right(46),
    }
}

fn y9s4_enc_map(h: u64) -> u64 {
    h.wrapping_add(0x2C)
}

fn y9s4_dec_match_tod(q: u64) -> u64 {
    const S: [usize; 8] = [3, 0, 1, 4, 6, 2, 5, 7];
    const K: u64 = 0x09FCBBCDC6C3DD1E;
    match 0x09FCBBCDB390EC42_u64
        .wrapping_add(peb_key())
        .wrapping_sub(image_key())
        % 5
    {
        0 => (shuffle_bytes(q, S) ^ K).rotate_left(51),
        1 => shuffle_bytes(q, S).rotate_left(51) ^ K,
        2 => shuffle_bytes((q ^ K).rotate_left(51), S),
        3 => shuffle_bytes(q ^ K, S).rotate_left(51),
        _ => shuffle_bytes(q.rotate_left(51), S) ^ K,
    }
}

fn y9s4_dec_match_settings(q: u64) -> u64 {
    const S: [usize; 8] = [6, 2, 1, 0, 4, 7, 5, 3];
    const K: u64 = 0x8E29AFEEBA0D05BA;
    match (image_key() ^ 0x8E29AFEEA4AE4BC7_u64.wrapping_sub(peb_key())) % 5 {
        0 => shuffle_bytes(q.rotate_left(18) ^ K, S),
        1 => shuffle_bytes(q.rotate_left(18), S) ^ K,
        2 => shuffle_bytes(q ^ K, S).rotate_left(18),
        3 => shuffle_bytes((q ^ K).rotate_left(18), S),
        _ => shuffle_bytes(q, S).rotate_left(18) ^ K,
    }
}

fn y9s4_dec_match_settings_count(q: u64) -> u64 {
    const S: [usize; 8] = [6, 2, 1, 0, 4, 7, 5, 3];
    const K: u64 = 0x153B0507AD39709D;
    match (image_key() ^ 0x8E29AFEEA4AE4BC7_u64.wrapping_sub(peb_key())) % 5 {
        0 => shuffle_bytes(q.rotate_left(18) ^ K, S),
        1 => shuffle_bytes(q.rotate_left(18), S) ^ K,
        2 => shuffle_bytes(q ^ K, S).rotate_left(18),
        3 => shuffle_bytes((q ^ K).rotate_left(18), S),
        _ => shuffle_bytes(q, S).rotate_left(18) ^ K,
    }
}

fn y9s4_enc_match_settings_count(v: u64) -> u64 {
    const S: [usize; 8] = [3, 2, 1, 7, 4, 6, 0, 5];
    const K: u64 = 0x153B0507AD39709D;
    match (image_key() ^ 0x8E29AFEEA4AE4BC7_u64.wrapping_sub(peb_key())) % 5 {
        0 => (shuffle_bytes(v, S) ^ K).rotate_right(18),
        1 => shuffle_bytes(v ^ K, S).rotate_right(18),
        2 => shuffle_bytes(v.rotate_right(18), S) ^ K,
        3 => shuffle_bytes(v, S).rotate_right(18) ^ K,
        _ => shuffle_bytes((v ^ K).rotate_right(18), S),
    }
}

fn y9s4_enc_handle_uid(uid: u64) -> u64 {
    const KEY: u64 = 0xE7CDD05836841AEC;
    const PI: [usize; 8] = [4, 1, 3, 6, 0, 2, 5, 7];
    match (image_key() ^ 0xE7CDD0584E71162C_u64.wrapping_sub(peb_key())) % 6 {
        0 => shuffle_bytes(uid, PI).rotate_right(52).wrapping_sub(KEY),
        1 => shuffle_bytes(uid.rotate_right(52), PI).wrapping_sub(KEY),
        2 => shuffle_bytes(uid, PI).wrapping_sub(KEY).rotate_right(52),
        3 => shuffle_bytes(uid.wrapping_sub(KEY), PI).rotate_right(52),
        4 => shuffle_bytes(uid.rotate_right(52).wrapping_sub(KEY), PI),
        _ => shuffle_bytes(uid.wrapping_sub(KEY).rotate_right(52), PI),
    }
}

fn y9s4_enc_object_uid(uid: u64) -> u64 {
    const P: [usize; 8] = [2, 7, 5, 4, 6, 3, 0, 1];
    match (image_key() ^ 0xD821E972_u64.wrapping_sub(peb_key())) % 3 {
        0 => shuffle_bytes(uid, P).rotate_right(25).wrapping_add(0x2F),
        1 => shuffle_bytes(uid.rotate_right(25), P).wrapping_add(0x2F),
        _ => shuffle_bytes(uid.wrapping_add(0x2F), P).rotate_right(25),
    }
}

fn y9s4_dec_settings(q: u64) -> u64 {
    const K: u64 = 0x3788F649AB28025C;
    match (image_key() ^ 0x3788F649AB280229_u64.wrapping_sub(peb_key())) % 6 {
        0 => q.rotate_left(41).wrapping_sub(0x62) ^ K,
        1 => (q.rotate_left(41) ^ K).wrapping_sub(0x62),
        2 => (q.wrapping_sub(0x62) ^ K).rotate_left(41),
        3 => q.wrapping_sub(0x62).rotate_left(41) ^ K,
        4 => (q ^ K).rotate_left(41).wrapping_sub(0x62),
        _ => (q ^ K).wrapping_sub(0x62).rotate_left(41),
    }
}

fn y9s4_dec_settings_count(q: u64) -> u64 {
    const A: u64 = 0x161A122651AE07DC;
    const K: u64 = 0x1B446A2B4C2D1638;
    match (image_key() ^ 0x3788F649AB280229_u64.wrapping_sub(peb_key())) % 6 {
        0 => q.rotate_left(41).wrapping_add(A) ^ K,
        1 => (q.rotate_left(41) ^ K).wrapping_add(A),
        2 => (q.wrapping_add(A) ^ K).rotate_left(41),
        3 => q.wrapping_add(A).rotate_left(41) ^ K,
        4 => (q ^ K).rotate_left(41).wrapping_add(A),
        _ => (q ^ K).wrapping_add(A).rotate_left(41),
    }
}

fn y9s4_running(key: u64) -> u8 {
    match 0x2C59C2650C5B881B_u64
        .wrapping_add(peb_key())
        .wrapping_sub(key)
        % 6
    {
        0 | 1 | 3 => 0xCE,
        _ => 0x0E,
    }
}

fn y9s4_countdown(key: u64) -> u8 {
    match (key ^ 0x04B11FFE3E45E813_u64.wrapping_sub(peb_key())) % 5 {
        0 => 0x04,
        1 => 0x07,
        2 => 0x01,
        3 => 0xA7,
        _ => 0x61,
    }
}

fn y9s4_match_over(key: u64) -> u8 {
    match (key ^ 0x8BD6BC139882716D_u64.wrapping_sub(peb_key())) % 6 {
        0 => 0xDA,
        1 => 0x8F,
        2 => 0x04,
        3 => 0x22,
        4 => 0xAD,
        _ => 0xD1,
    }
}

fn y9s4_unfrozen(key: u64) -> u32 {
    match (key ^ 0x7AF9C2C5D41B24F9_u64.wrapping_sub(peb_key())) % 3 {
        0 | 1 => 0x0F50B41C,
        _ => 0xCF2C2BE2,
    }
}

fn y9s4_display(key: u64) -> [u8; 2] {
    if key.wrapping_add(peb_key() ^ 0x83F07C7DE7687C1F) & 1 == 0 {
        [0xEE, 0xE6]
    } else {
        [0xF4, 0xEC]
    }
}

fn y9s4_enc_end(v: u32) -> u32 {
    const X: u32 = 0x99B38504;
    let mix = |x: u32| {
        let b = x.to_le_bytes();
        u32::from_le_bytes([b[0], b[2], b[3], b[1]])
    };
    match (image_key() ^ 0xA37A2572E162F347_u64.wrapping_sub(peb_key())) % 5 {
        0 => mix((v ^ X).rotate_left(11)),
        1 => mix(v.rotate_left(11) ^ X),
        2 => (mix(v) ^ X).rotate_left(11),
        3 => mix(v ^ X).rotate_left(11),
        _ => mix(v.rotate_left(11)) ^ X,
    }
}

fn y9s4_dec_end(x: u32) -> u32 {
    const X: u32 = 0x99B38504;
    let unmix = |x: u32| {
        let b = x.to_le_bytes();
        u32::from_le_bytes([b[0], b[3], b[1], b[2]])
    };
    match (image_key() ^ 0xA37A2572E162F347_u64.wrapping_sub(peb_key())) % 5 {
        0 => unmix(x).rotate_right(11) ^ X,
        1 => (unmix(x) ^ X).rotate_right(11),
        2 => unmix(x.rotate_right(11) ^ X),
        3 => unmix(x.rotate_right(11)) ^ X,
        _ => unmix(x ^ X).rotate_right(11),
    }
}

fn y9s4_enc_duration(v: u32) -> u32 {
    const X: u32 = 0xFEA2FB5A;
    if image_key().wrapping_add(peb_key() ^ 0x733868D1B004D5E9) & 1 == 0 {
        v.rotate_left(16).wrapping_add(0x75) ^ X
    } else {
        v.wrapping_add(0x75).rotate_left(16) ^ X
    }
}

fn y9s4_dec_duration(x: u32) -> u32 {
    const X: u32 = 0xFEA2FB5A;
    if image_key().wrapping_add(peb_key() ^ 0x733868D1B004D5E9) & 1 == 0 {
        (x ^ X).wrapping_sub(0x75).rotate_left(16)
    } else {
        (x ^ X).rotate_left(16).wrapping_sub(0x75)
    }
}

fn y9s4_dec_list(q: u64) -> u64 {
    const P: [usize; 8] = [2, 0, 6, 7, 4, 3, 5, 1];
    if image_key().wrapping_add(peb_key() ^ 0x936B4A11) & 1 == 0 {
        shuffle_bytes(q, P).wrapping_sub(0x0B).rotate_left(29)
    } else {
        shuffle_bytes(q, P).rotate_left(29).wrapping_sub(0x0B)
    }
}

fn y9s4_dec_count(q: u64) -> u64 {
    const P: [usize; 8] = [2, 0, 6, 7, 4, 3, 5, 1];
    const C: u64 = 0xCFBF1CBD28ACEF85;
    if image_key().wrapping_add(peb_key() ^ 0x936B4A11) & 1 == 0 {
        shuffle_bytes(q, P).wrapping_add(C).rotate_left(29)
    } else {
        shuffle_bytes(q, P).rotate_left(29).wrapping_add(C)
    }
}

fn y9s4_dec_handle(q: u64) -> u64 {
    const C: u64 = 0xA5B14930BADAE9BA;
    const P: [usize; 8] = [2, 1, 6, 7, 0, 3, 5, 4];
    const S: [usize; 8] = [3, 5, 2, 7, 6, 0, 1, 4];
    match image_key().wrapping_add(peb_key() ^ 0xA5B14930A4DB25D7) % 5 {
        0 => shuffle_bytes(
            shuffle_bytes(q, [2, 4, 1, 6, 3, 0, 7, 5]).wrapping_add(C),
            [0, 2, 3, 6, 5, 4, 7, 1],
        ),
        1 => shuffle_bytes(q, P).wrapping_add(C),
        2 => shuffle_bytes(q.wrapping_add(C), S),
        3 => shuffle_bytes(q.wrapping_add(C), P),
        _ => shuffle_bytes(q, S).wrapping_add(C),
    }
}

fn y9s4_dec_loadout(q: u64) -> u64 {
    const X: u64 = 0x726558D6FC394054;
    const A: u64 = 0x3F;
    match 0x726558D6FC394071_u64
        .wrapping_add(peb_key())
        .wrapping_sub(image_key())
        % 5
    {
        0 => (q ^ X).rotate_left(38).wrapping_sub(A),
        1 => (q ^ X).wrapping_sub(A).rotate_left(38),
        2 => q.rotate_left(38).wrapping_sub(A) ^ X,
        3 => (q.rotate_left(38) ^ X).wrapping_sub(A),
        _ => (q.wrapping_sub(A) ^ X).rotate_left(38),
    }
}

fn y9s4_dec_slot(x: u64) -> u64 {
    const P: [usize; 8] = [0, 2, 3, 7, 5, 6, 4, 1];
    match (image_key() ^ 0xB476645A_u64.wrapping_sub(peb_key())) % 5 {
        0 => shuffle_bytes(x.rotate_left(17), P).wrapping_sub(0x0D),
        1 => shuffle_bytes(x.rotate_left(17).wrapping_sub(0x0D), P),
        2 => shuffle_bytes(x, P).wrapping_sub(0x0D).rotate_left(17),
        3 => shuffle_bytes(x, P).rotate_left(17).wrapping_sub(0x0D),
        _ => shuffle_bytes(x.wrapping_sub(0x0D).rotate_left(17), P),
    }
}

fn y9s4_enc_item(v: u64) -> u64 {
    const P: [usize; 8] = [4, 1, 6, 2, 7, 3, 5, 0];
    const C: u64 = 0xDBA59183391B9179;
    match (image_key() ^ 0xDBA59183A50E64FE_u64.wrapping_sub(peb_key())) % 5 {
        0 => shuffle_bytes(v.rotate_left(11), P).wrapping_sub(C),
        1 => shuffle_bytes(v, P).rotate_left(11).wrapping_sub(C),
        2 => shuffle_bytes(v.wrapping_sub(C).rotate_left(11), P),
        3 => shuffle_bytes(v.rotate_left(11).wrapping_sub(C), P),
        _ => shuffle_bytes(v, P).wrapping_sub(C).rotate_left(11),
    }
}

fn y10s1_dec_gm(x: u64) -> u64 {
    const E: u64 = 0xAECA1949691ED7BA;
    if x == 0 {
        0
    } else {
        (x ^ E).rotate_right(34) ^ image_key()
    }
}

fn y10s1_dec_state(x: u32) -> u32 {
    const A: u32 = 0x33640A2D;
    let s = (x ^ A).rotate_right(15) ^ image_key() as u32;
    match s {
        s @ 6..=8 => s - 1,
        s => s,
    }
}

fn y10s1_dec_match_index(b: u8) -> u8 {
    b ^ 0xF2
}

fn y10s1_dec_match_data(q: u64) -> u64 {
    const X: u64 = 0x1572AFE6EBE0ABC5;
    const A: u64 = 0xA80AEF80B690BD78;
    (q ^ X).wrapping_add(A) ^ image_key()
}

fn y10s1_dec_match_count(q: u64) -> u64 {
    const X: u64 = 0x7B25010C967BA8A2;
    const A: u64 = 0x4B0E646B7F9F21D4;
    (q ^ X).wrapping_add(A) ^ image_key()
}

fn y10s1_enc_playlist(uid: u64) -> u64 {
    uid.wrapping_sub(0x1ECD0A1EE8A45E2D)
}

fn y10s1_enc_mode(h: u64) -> u64 {
    const A: u64 = 0x3F136ED72192314F;
    const B: u64 = 0x3E1FE098FC6608E9;
    h.wrapping_add(image_key())
        .wrapping_sub(B)
        .rotate_right(51)
        .wrapping_sub(A)
}

fn y10s1_enc_map(h: u64) -> u64 {
    const A: u64 = 0x4045C32F05F4E395;
    const X: u64 = 0xE42A899F;
    (h.wrapping_sub(image_key()).rotate_right(17) ^ X).wrapping_sub(A)
}

fn y10s1_dec_match_tod(q: u64) -> u64 {
    const X1: u64 = 0x4E15A877;
    const A: u64 = 0x058635B11251B425;
    const X2: u64 = 0xB3507B0DC889BDB2;
    ((q ^ X1).wrapping_add(A) ^ X2).wrapping_add(image_key())
}

fn y10s1_dec_match_settings(q: u64) -> u64 {
    const A: u64 = 0xF879DC0D4F5DC4A9;
    const X: u64 = 0x275463E3;
    (q.wrapping_add(A) ^ X)
        .rotate_left(60)
        .wrapping_add(image_key())
}

fn y10s1_dec_match_settings_count(q: u64) -> u64 {
    const A: u64 = 0xD88EA5D9AAF4E863;
    const X: u64 = 0x275463E3;
    (q.wrapping_add(A) ^ X)
        .rotate_left(60)
        .wrapping_add(image_key())
}

fn y10s1_enc_match_settings_count(v: u64) -> u64 {
    const A: u64 = 0xD88EA5D9AAF4E863;
    const X: u64 = 0x275463E3;
    (v.wrapping_sub(image_key()).rotate_right(60) ^ X).wrapping_sub(A)
}

fn y10s1_enc_handle_uid(uid: u64) -> u64 {
    const X: u64 = 0x77B5DDB649D95C58;
    const C: u64 = 0x70C176E014ED2986;
    (uid.wrapping_add(image_key()) ^ X).wrapping_add(C)
}

fn y10s1_enc_object_uid(uid: u64) -> u64 {
    const Y: u64 = 0x2D2E2C2F;
    const Z: u64 = 0xD8A0CEF2;
    (uid.wrapping_sub(image_key()) ^ Y).rotate_left(28) ^ Z
}

fn y10s1_dec_settings(q: u64) -> u64 {
    const A: u64 = 0xCFDB4FBB8C24EA98;
    const B: u64 = 0x57F1362FDA847B0D;
    q.wrapping_add(A)
        .rotate_left(61)
        .wrapping_add(B)
        .wrapping_sub(image_key())
}

fn y10s1_dec_settings_count(q: u64) -> u64 {
    const A: u64 = 0xD30C8374D6D9EE0D;
    const B: u64 = 0xF005EE4C7AFF31A0;
    q.wrapping_add(A)
        .rotate_left(61)
        .wrapping_add(B)
        .wrapping_sub(image_key())
}

fn y10s1_live(key: u64) -> u32 {
    const A: u32 = 0xAAAD54F6;
    const X: u32 = 0x8DF8588B;
    (key as u32).wrapping_add(1).rotate_left(5).wrapping_add(A) ^ X
}

fn y10s1_running(key: u64) -> u8 {
    (key as u8).wrapping_add(0x2D) ^ 0x20
}

fn y10s1_match_over(key: u64) -> u8 {
    (0xA1_u8.wrapping_sub(key as u8) ^ 0x9D).wrapping_add(0x24)
}

fn y10s1_unfrozen(key: u64) -> u32 {
    const A: u32 = 0xDAB89D54;
    const B: u32 = 0x2B81B6B9;
    (key as u32).wrapping_add(A).rotate_left(1).wrapping_add(B)
}

fn y10s1_display(key: u64) -> [u8; 2] {
    let k = key as u8;
    [
        k.wrapping_add(0x79).rotate_left(6).wrapping_add(0xAC),
        k.wrapping_add(0x78).rotate_left(6).wrapping_add(0xAC),
    ]
}

fn y10s1_enc_end(v: u32) -> u32 {
    const X: u32 = 0xC9F023AB;
    const A: u32 = 0x570A4901;
    (v.wrapping_add(image_key() as u32) ^ X)
        .rotate_left(15)
        .wrapping_add(A)
}

fn y10s1_dec_end(x: u32) -> u32 {
    const X: u32 = 0xC9F023AB;
    const A: u32 = 0x570A4901;
    (x.wrapping_sub(A).rotate_right(15) ^ X).wrapping_sub(image_key() as u32)
}

fn y10s1_enc_duration(v: u32) -> u32 {
    const X: u32 = 0x1C6DFE32;
    (v.wrapping_add(image_key() as u32) ^ X).rotate_left(7)
}

fn y10s1_dec_duration(x: u32) -> u32 {
    const X: u32 = 0x1C6DFE32;
    (x.rotate_right(7) ^ X).wrapping_sub(image_key() as u32)
}

fn y10s1_dec_list(q: u64) -> u64 {
    const A: u64 = 0x246F232A8FB9BA7B;
    q.rotate_left(2).wrapping_sub(A).wrapping_sub(image_key())
}

fn y10s1_dec_count(q: u64) -> u64 {
    const A: u64 = 0x65863A663BAB1515;
    q.rotate_left(2).wrapping_sub(A).wrapping_sub(image_key())
}

fn y10s1_dec_handle(q: u64) -> u64 {
    const C: u64 = 0x5DC54BD36AF7670D;
    const X: u64 = 0x21EA0A139210B0B2;
    const A: u64 = 0x54310A636376094A;
    (q.wrapping_sub(C) ^ X)
        .wrapping_sub(A)
        .wrapping_sub(image_key())
}

fn y10s1_dec_loadout(q: u64) -> u64 {
    const X: u64 = 0xB48D0377;
    const A: u64 = 0x91666EAE7B3A9148;
    const Y: u64 = 0x1E8395AB;
    (q ^ X).wrapping_sub(A) ^ Y ^ image_key()
}

fn y10s1_dec_slot(x: u64) -> u64 {
    const C: u64 = 0xE07DD03FA648AEED;
    const A: u64 = 0x1600D2D268DD95AB;
    x.wrapping_sub(C).rotate_left(3).wrapping_sub(A) ^ image_key()
}

fn y10s1_enc_item(v: u64) -> u64 {
    const X: u64 = 0x0301D044EA875A0D;
    const C: u64 = 0x957F24CC854119FC;
    (v.wrapping_add(image_key()) ^ X).wrapping_add(C)
}

fn y10s1_watermark(key: u64) -> [u8; 2] {
    let off = (key as u8).wrapping_add(0x2A).rotate_left(1) ^ 0xEA;
    [off, off]
}

struct Phase {
    start: u32,
    duration: u32,
    end: u32,
    at: u32,
}

static MULTIPLAYER_MODES: &[(&str, u64)] = &[
    ("Bomb", 0x138BDF6E),
    ("Bomb - No Prep Phase", 0x25A11B0495),
    ("Hostage", 0xA934BDF6),
    ("Secure Area", 0x76337AA1),
    ("Warmup", WARMUP_MODE),
];
static TERRORIST_HUNT_MODES: &[(&str, u64)] = &[
    ("Disarm Bomb", 0x2036D0C6E),
    ("Elimination", 0x58A02A93),
    ("Extract Hostage", 0x67FDB2DB),
    ("Protect Hostage", 0x58A02A91),
];
static DIFFICULTIES: &[(&str, u64)] = &[
    ("Normal", 0xCDE271EC),
    ("Hard", 0xCDE271ED),
    ("Realistic", 0xCDE271EE),
];
const GYM_MODE: u64 = 0x72AFC423;
pub(crate) const WARMUP_MODE: u64 = 0x7A956D02F;
const WARMUP_MAPS: &[u64] = &[0x57AC2280B2];
const WARMUP_PLAYLIST: u64 = 0x574F867F9E;

static EVENT_FORGES: &[(u64, &str)] = &[
    (0x2D1D494EF2, "evn00_stadium_events"),
    (0x2BCB159A9C, "evn01_floyd_events"),
    (0x2F83769722, "evn02_cowboy_events"),
    (0xB3555E074, "evn03_halloween_events"),
    (0xB3555E074, "evn03_doktorscurse_events"),
    (0x3E005881AF, "evn04_gangdestruction_events"),
    (0x3EE107746E, "evn05_stadium2_events"),
    (0x40DC810C48, "evn06_morpheus_events"),
    (0x40DC810C48, "evn06_muteprotocol_events"),
    (0x431BA741C3, "evn07_telly_events"),
    (0x50183207D0, "evn08_beret_events"),
    (0x5464BBF809, "evn10_quarant_events"),
    (0x5617C235CE, "evn11_ballpark_events"),
    (0x585E25818B, "evn12_toky_events"),
    (0x5C4B99AE00, "evn14_snow3_events"),
    (0x5C4B99AE00, "evn14_freezeforall_events"),
];
static CLASSIC_EVENT_FORGES: &[(&str, &str)] = &[
    ("Outbreak", "fury_d01_resort"),
    ("Rainbow is Magic", "pvp_floyd_events"),
    ("Showdown", "evn02_cowboy_events"),
    ("Road To S.I. 2020", "evn00_stadium_events"),
];

type PlaylistEvent = (&'static str, u64, u64, &'static [usize]);
type HeadCodec = [fn(u64) -> u64; 2];

enum Roster {
    Plain {
        list: i32,
        handle: i32,
    },
    Encoded {
        lists: [u64; 2],
        dec_head: HeadCodec,
        handles: &'static [u64],
        dec_handle: fn(u64) -> u64,
        session_handles: Option<(u64, [u64; 2], &'static [u64])>,
    },
}

struct RoundProfile {
    live: (u64, Raw<u32>),
    running: (u64, Raw<u8>),
    countdown: (u64, Raw<u8>),
    match_over: (u64, Raw<u8>),
    now: u64,
    start: u64,
    end: u64,
    duration: u64,
    freeze: (u64, Raw<u32>),
    infinite_display: Option<(u64, Raw<[u8; 2]>)>,
    enc_end: fn(u32) -> u32,
    dec_end: fn(u32) -> u32,
    enc_duration: fn(u32) -> u32,
    dec_duration: fn(u32) -> u32,
    roster: Roster,
    loadout: i32,
    dec_loadout: Option<fn(u64) -> u64>,
    slots: [i32; 4],
    slot: Option<SlotChain>,
}

struct SlotChain {
    dec: fn(u64) -> u64,
    field: u64,
    item: u64,
    enc_item: fn(u64) -> u64,
}

struct MatchLayout {
    playlist: u64,
    mode: u64,
    map: u64,
    difficulty: u64,
    time_of_day: u64,
    settings: u64,
}

const Y5_MATCH_LAYOUT: MatchLayout = MatchLayout {
    playlist: 0x00,
    mode: 0x08,
    map: 0x10,
    difficulty: 0x30,
    time_of_day: 0x38,
    settings: 0x48,
};

const Y7_MATCH_LAYOUT: MatchLayout = MatchLayout {
    playlist: 0x00,
    mode: 0x08,
    map: 0x10,
    difficulty: 0x38,
    time_of_day: 0x40,
    settings: 0x50,
};

const Y8S3_MATCH_LAYOUT: MatchLayout = MatchLayout {
    playlist: 0x00,
    mode: 0x08,
    map: 0x10,
    difficulty: 0x38,
    time_of_day: 0x40,
    settings: 0x60,
};

const Y9S1_MATCH_LAYOUT: MatchLayout = MatchLayout {
    playlist: 0x68,
    mode: 0x10,
    map: 0x48,
    difficulty: 0x60,
    time_of_day: 0x28,
    settings: 0x50,
};

const Y9S2_MATCH_LAYOUT: MatchLayout = MatchLayout {
    playlist: 0x10,
    mode: 0x50,
    map: 0x38,
    difficulty: 0x28,
    time_of_day: 0x00,
    settings: 0x40,
};

const Y9S3_MATCH_LAYOUT: MatchLayout = MatchLayout {
    playlist: 0x10,
    mode: 0x68,
    map: 0x18,
    difficulty: 0x58,
    time_of_day: 0x20,
    settings: 0x48,
};

const Y9S4_MATCH_LAYOUT: MatchLayout = MatchLayout {
    playlist: 0x20,
    mode: 0x38,
    map: 0x68,
    difficulty: 0x18,
    time_of_day: 0x58,
    settings: 0x28,
};

const Y10S1_MATCH_LAYOUT: MatchLayout = MatchLayout {
    playlist: 0x18,
    mode: 0x38,
    map: 0x30,
    difficulty: 0x40,
    time_of_day: 0x20,
    settings: 0x60,
};

enum MatchState {
    Indexed {
        entries: u64,
        index: u64,
        stride: u64,
    },
    Encoded(&'static MatchCodec),
}

struct MatchCodec {
    index: (u64, fn(u8) -> u8),
    entries: (u64, HeadCodec),
    enc_playlist: fn(u64) -> u64,
    enc_mode: fn(u64) -> u64,
    enc_map: fn(u64) -> u64,
    dec_tod: fn(u64) -> u64,
    settings: HeadCodec,
    enc_settings_count: fn(u64) -> u64,
}

impl MatchState {
    fn codec(&self) -> Option<&'static MatchCodec> {
        match *self {
            MatchState::Indexed { .. } => None,
            MatchState::Encoded(codec) => Some(codec),
        }
    }
}

struct GameProfile {
    tag: &'static str,
    game_manager: u64,
    dec_gm: Option<fn(u64) -> u64>,
    state: u64,
    dec_state: Option<fn(u32) -> u32>,
    role: u64,
    match_state: MatchState,
    match_layout: MatchLayout,
    gamemode: u64,
    settings: u64,
    dec_settings: Option<HeadCodec>,
    time_of_day: u64,
    enc_handle_uid: Option<fn(u64) -> u64>,
    enc_object_uid: Option<fn(u64) -> u64>,
    round: RoundProfile,
    maps: &'static [(&'static str, u64)],
    standard_maps: usize,
    thunt_maps: &'static [usize],
    tdm_maps: &'static [usize],
    arcade: &'static [PlaylistEvent],
    events: &'static [PlaylistEvent],
}

static Y5S1_GAME: GameProfile = GameProfile {
    tag: "y5s1",
    game_manager: 0x53A6868,
    dec_gm: None,
    state: 0xF0,
    dec_state: None,
    role: 0x230,
    match_state: MatchState::Indexed {
        entries: 0x90,
        index: 0x8C,
        stride: 0x58,
    },
    match_layout: Y5_MATCH_LAYOUT,
    gamemode: 0x3A94168,
    settings: 0x70,
    dec_settings: None,
    time_of_day: 0x20,
    enc_handle_uid: None,
    enc_object_uid: None,
    round: RoundProfile {
        live: (0x1F4, Raw::Fixed(1)),
        running: (0x998, Raw::Fixed(1)),
        countdown: (0x999, Raw::Fixed(0)),
        match_over: (0x99F, Raw::Fixed(1)),
        now: 0x1678,
        start: 0x944,
        end: 0x948,
        duration: 0x950,
        freeze: (0x94C, Raw::Fixed(0)),
        infinite_display: None,
        enc_end: plain_u32,
        dec_end: plain_u32,
        enc_duration: plain_u32,
        dec_duration: plain_u32,
        roster: Roster::Encoded {
            lists: [0x300, 0x4D8],
            dec_head: [plain_u64; 2],
            handles: &[0x98],
            dec_handle: plain_u64,
            session_handles: None,
        },
        loadout: 0x40,
        dec_loadout: None,
        slots: Y5_SLOTS,
        slot: None,
    },
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
    tdm_maps: &[],
    arcade: &[
        ("Warmup", WARMUP_MODE, WARMUP_PLAYLIST, &[]),
        ("Golden Gun", 0x3EA695B452, 0x3F331EF4E2, &[1]),
    ],
    events: &[("Grand Larceny", 0x39E871FC8A, 0x4024D5CD3C, &[22])],
};

static Y5S2_GAME: GameProfile = GameProfile {
    tag: "y5s2",
    game_manager: 0x603DD10,
    dec_gm: None,
    state: 0x114,
    dec_state: None,
    role: 0x278,
    match_state: MatchState::Indexed {
        entries: 0xF8,
        index: 0xF4,
        stride: 0x80,
    },
    match_layout: Y5_MATCH_LAYOUT,
    gamemode: 0x4C8AD70,
    settings: 0x70,
    dec_settings: None,
    time_of_day: 0x20,
    enc_handle_uid: None,
    enc_object_uid: None,
    round: RoundProfile {
        live: (0x234, Raw::Fixed(1)),
        running: (0x5E8, Raw::Fixed(0)),
        countdown: (0x670, Raw::Fixed(0)),
        match_over: (0x97C, Raw::Fixed(1)),
        now: 0x16C8,
        start: 0xA60,
        end: 0x600,
        duration: 0x970,
        freeze: (0x96C, Raw::Fixed(0x4E4DD9F6)),
        infinite_display: None,
        enc_end: y5s2_end,
        dec_end: y5s2_end,
        enc_duration: y5s2_duration,
        dec_duration: y5s2_duration,
        roster: Roster::Plain {
            list: 0x3C0,
            handle: 0xA8,
        },
        loadout: 0x40,
        dec_loadout: None,
        slots: Y5_SLOTS,
        slot: None,
    },
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
    tdm_maps: &[],
    arcade: &[("Warmup", WARMUP_MODE, WARMUP_PLAYLIST, &[])],
    events: &[("M.U.T.E. Protocol", 0x41E157DDE5, 0x41E15761D1, &[22])],
};

static Y5S3_GAME: GameProfile = GameProfile {
    tag: "y5s3",
    game_manager: 0x6E3FF30,
    dec_gm: None,
    state: 0x50,
    dec_state: None,
    role: 0x308,
    match_state: MatchState::Indexed {
        entries: 0x68,
        index: 0x64,
        stride: 0x88,
    },
    match_layout: Y5_MATCH_LAYOUT,
    gamemode: 0x534D4B0,
    settings: 0x68,
    dec_settings: None,
    time_of_day: 0x40,
    enc_handle_uid: None,
    enc_object_uid: None,
    round: RoundProfile {
        live: (0x2CC, Raw::Fixed(1)),
        running: (0x418, Raw::Fixed(2)),
        countdown: (0x874, Raw::Fixed(0x5E)),
        match_over: (0xB68, Raw::Fixed(0x4C)),
        now: 0x5118,
        start: 0x6BC,
        end: 0x7A4,
        duration: 0x7F0,
        freeze: (0xBF0, Raw::Fixed(0x365D0F95)),
        infinite_display: None,
        enc_end: y5s3_enc_end,
        dec_end: y5s3_dec_end,
        enc_duration: y5s3_enc_duration,
        dec_duration: y5s3_dec_duration,
        roster: Roster::Plain {
            list: 0x8C0,
            handle: 0xA8,
        },
        loadout: 0x38,
        dec_loadout: None,
        slots: Y5_SLOTS,
        slot: None,
    },
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
    tdm_maps: &[],
    arcade: &[
        ("Warmup", WARMUP_MODE, WARMUP_PLAYLIST, &[]),
        ("Golden Gun", 0x3EA695B452, 0x3F331EF4E2, &[12, 21, 15]),
    ],
    events: &[("Sugar Fright", 0x45840C11F0, 0x45840EDB69, &[22])],
};

static Y5S4_GAME: GameProfile = GameProfile {
    tag: "y5s4",
    game_manager: 0x6E85930,
    dec_gm: Some(y5s4_dec_gm),
    state: 0xB8,
    dec_state: Some(y5s4_dec_state),
    role: 0x348,
    match_state: MatchState::Indexed {
        entries: 0xC8,
        index: 0xC4,
        stride: 0x88,
    },
    match_layout: Y5_MATCH_LAYOUT,
    gamemode: 0x53A7BA0,
    settings: 0x68,
    dec_settings: None,
    time_of_day: 0x40,
    enc_handle_uid: None,
    enc_object_uid: None,
    round: RoundProfile {
        live: (0x30C, Raw::Fixed(1)),
        running: (0x438, Raw::Fixed(0x12)),
        countdown: (0x9F0, Raw::Fixed(0x8A)),
        match_over: (0x478, Raw::Fixed(0x3F)),
        now: 0x51E8,
        start: 0xB88,
        end: 0xA08,
        duration: 0xAE4,
        freeze: (0xBD0, Raw::Fixed(0x2520D190)),
        infinite_display: None,
        enc_end: y5s4_enc_end,
        dec_end: y5s4_dec_end,
        enc_duration: y5s4_enc_duration,
        dec_duration: y5s4_dec_duration,
        roster: Roster::Encoded {
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
    },
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
    tdm_maps: &[],
    arcade: &[("Warmup", WARMUP_MODE, WARMUP_PLAYLIST, &[])],
    events: &[("Road To S.I. 2021", 0x483AB3E121, 0x4707B87CF1, &[22])],
};

static Y6S1_GAME: GameProfile = GameProfile {
    tag: "y6s1",
    game_manager: 0x70D88C8,
    dec_gm: Some(y6s1_dec_gm),
    state: 0x184,
    dec_state: Some(y6s1_dec_state),
    role: 0x340,
    match_state: MatchState::Indexed {
        entries: 0xA0,
        index: 0x9C,
        stride: 0x88,
    },
    match_layout: Y5_MATCH_LAYOUT,
    gamemode: 0x55444F0,
    settings: 0x68,
    dec_settings: None,
    time_of_day: 0x40,
    enc_handle_uid: None,
    enc_object_uid: None,
    round: RoundProfile {
        live: (0x304, Raw::Fixed(1)),
        running: (0x738, Raw::Fixed(0x96)),
        countdown: (0x9A0, Raw::Fixed(0x36)),
        match_over: (0xC18, Raw::Fixed(0xB7)),
        now: 0x51C8,
        start: 0xADC,
        end: 0xAC0,
        duration: 0xB70,
        freeze: (0x3BC, Raw::Fixed(0xA3B33AA2)),
        infinite_display: Some((0xA81, Raw::Fixed([0x36, 0x2E]))),
        enc_end: y6s1_enc_end,
        dec_end: y6s1_dec_end,
        enc_duration: y6s1_enc_duration,
        dec_duration: y6s1_dec_duration,
        roster: Roster::Encoded {
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
    },
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
    tdm_maps: &[],
    arcade: &[("Warmup", WARMUP_MODE, WARMUP_PLAYLIST, &[])],
    events: &[
        ("Rainbow is Magic", 0x4DB61E967D, 0x4DB61E9851, &[22]),
        ("Apocalypse", 0x530F7BDC93, 0x530F7BED20, &[23]),
    ],
};

static Y6S2_GAME: GameProfile = GameProfile {
    tag: "y6s2",
    game_manager: 0x729A598,
    dec_gm: Some(y6s2_dec_gm),
    state: 0x150,
    dec_state: Some(y6s2_dec_state),
    role: 0x358,
    match_state: MatchState::Indexed {
        entries: 0xD0,
        index: 0xCC,
        stride: 0x88,
    },
    match_layout: Y5_MATCH_LAYOUT,
    gamemode: 0x56FCF90,
    settings: 0x68,
    dec_settings: None,
    time_of_day: 0x40,
    enc_handle_uid: None,
    enc_object_uid: None,
    round: RoundProfile {
        live: (0x31C, Raw::Fixed(1)),
        running: (0x488, Raw::Fixed(0xEE)),
        countdown: (0xA61, Raw::Fixed(0x42)),
        match_over: (0x800, Raw::Fixed(0xFF)),
        now: 0x6050,
        start: 0x928,
        end: 0x490,
        duration: 0x7D8,
        freeze: (0x948, Raw::Fixed(0xDE263BB2)),
        infinite_display: Some((0x7F0, Raw::Fixed([0xD8, 0xD5]))),
        enc_end: y6s2_enc_end,
        dec_end: y6s2_dec_end,
        enc_duration: y6s2_enc_duration,
        dec_duration: y6s2_dec_duration,
        roster: Roster::Encoded {
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
    },
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
    tdm_maps: &[],
    arcade: &[("Warmup", WARMUP_MODE, WARMUP_PLAYLIST, &[])],
    events: &[("Containment", 0x547CD662FD, 0x54A95068E8, &[22])],
};

static Y6S3_GAME: GameProfile = GameProfile {
    tag: "y6s3",
    game_manager: 0x75AE938,
    dec_gm: Some(y6s3_dec_gm),
    state: 0x158,
    dec_state: Some(y6s3_dec_state),
    role: 0x338,
    match_state: MatchState::Indexed {
        entries: 0xD0,
        index: 0xCC,
        stride: 0x88,
    },
    match_layout: Y5_MATCH_LAYOUT,
    gamemode: 0x5B9F930,
    settings: 0x68,
    dec_settings: None,
    time_of_day: 0x40,
    enc_handle_uid: None,
    enc_object_uid: None,
    round: RoundProfile {
        live: (0x2FC, Raw::Fixed(1)),
        running: (0xC30, Raw::Fixed(0xF2)),
        countdown: (0xC33, Raw::Fixed(0x29)),
        match_over: (0xC2B, Raw::Fixed(0x8F)),
        now: 0x5B28,
        start: 0xBC4,
        end: 0xBFC,
        duration: 0xBDC,
        freeze: (0xBB4, Raw::Fixed(0xD24ED01E)),
        infinite_display: Some((0xC2C, Raw::Fixed([0x99, 0x19]))),
        enc_end: y6s3_enc_end,
        dec_end: y6s3_dec_end,
        enc_duration: y6s3_enc_duration,
        dec_duration: y6s3_dec_duration,
        roster: Roster::Encoded {
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
    },
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
    tdm_maps: &[],
    arcade: &[("Warmup", WARMUP_MODE, WARMUP_PLAYLIST, &[])],
    events: &[
        ("Showdown", 0x30B6346015, 0x4DB61E9852, &[22]),
        ("Doktor's Curse", 0x55CDE1F7AE, 0x55CDE1FAB7, &[23]),
    ],
};

static Y6S4_GAME: GameProfile = GameProfile {
    tag: "y6s4",
    game_manager: 0x790A308,
    dec_gm: Some(y6s4_dec_gm),
    state: 0x158,
    dec_state: Some(y6s4_dec_state),
    role: 0x380,
    match_state: MatchState::Indexed {
        entries: 0x60,
        index: 0x5C,
        stride: 0x88,
    },
    match_layout: Y5_MATCH_LAYOUT,
    gamemode: 0x56C33E0,
    settings: 0x68,
    dec_settings: None,
    time_of_day: 0x40,
    enc_handle_uid: None,
    enc_object_uid: None,
    round: RoundProfile {
        live: (0x344, Raw::Fixed(1)),
        running: (0xD5C, Raw::Fixed(0x54)),
        countdown: (0xD5D, Raw::Fixed(0x48)),
        match_over: (0xD54, Raw::Fixed(0x75)),
        now: 0x5D58,
        start: 0xCF0,
        end: 0xCF4,
        duration: 0xCFC,
        freeze: (0xD2C, Raw::Fixed(0x197E7E42)),
        infinite_display: Some((0xD52, Raw::Fixed([0x2C, 0x28]))),
        enc_end: y6s4_enc_end,
        dec_end: y6s4_dec_end,
        enc_duration: y6s4_enc_duration,
        dec_duration: y6s4_dec_duration,
        roster: Roster::Encoded {
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
    },
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
    tdm_maps: &[16, 13, 18],
    arcade: &[("Team Deathmatch", WARMUP_MODE, WARMUP_PLAYLIST, &[])],
    events: &[
        ("Snow Brawl", 0x5797ECFED9, 0x4A9198FBF7, &[22]),
        ("Road To S.I. 2022", 0x57490FFCEA, 0x4DA02B54A4, &[23]),
        ("Road To S.I. 2022", 0x57490FFCEA, 0x5763873C39, &[24]),
    ],
};

static Y7S1_GAME: GameProfile = GameProfile {
    tag: "y7s1",
    game_manager: 0x7C91C48,
    dec_gm: Some(y7s1_dec_gm),
    state: 0x15C,
    dec_state: Some(y7s1_dec_state),
    role: 0x380,
    match_state: MatchState::Indexed {
        entries: 0x70,
        index: 0x6C,
        stride: 0x90,
    },
    match_layout: Y7_MATCH_LAYOUT,
    gamemode: 0x64CB688,
    settings: 0x68,
    dec_settings: None,
    time_of_day: 0x40,
    enc_handle_uid: None,
    enc_object_uid: None,
    round: RoundProfile {
        live: (0x344, Raw::Fixed(1)),
        running: (0xD64, Raw::Fixed(0xF3)),
        countdown: (0xD67, Raw::Fixed(0x95)),
        match_over: (0xD5C, Raw::Fixed(0xE4)),
        now: 0x6040,
        start: 0xD14,
        end: 0xCD8,
        duration: 0xD0C,
        freeze: (0xD50, Raw::Fixed(0x570004F)),
        infinite_display: Some((0xD62, Raw::Fixed([0xA9, 0xA8]))),
        enc_end: y7s1_enc_end,
        dec_end: y7s1_dec_end,
        enc_duration: y7s1_enc_duration,
        dec_duration: y7s1_dec_duration,
        roster: Roster::Encoded {
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
    },
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
    tdm_maps: &[13, 15, 16, 18, 22],
    arcade: &[("Team Deathmatch", WARMUP_MODE, WARMUP_PLAYLIST, &[])],
    events: &[("Rengoku", 0x585B44DA5C, 0x585B4583CD, &[23])],
};

static Y7S2_GAME: GameProfile = GameProfile {
    tag: "y7s2",
    game_manager: 0x81F7A60,
    dec_gm: Some(y7s2_dec_gm),
    state: 0x15C,
    dec_state: Some(y7s2_dec_state),
    role: 0x380,
    match_state: MatchState::Indexed {
        entries: 0xE0,
        index: 0xDC,
        stride: 0x90,
    },
    match_layout: Y7_MATCH_LAYOUT,
    gamemode: 0x61C55C8,
    settings: 0x68,
    dec_settings: None,
    time_of_day: 0x40,
    enc_handle_uid: None,
    enc_object_uid: None,
    round: RoundProfile {
        live: (0x344, Raw::Fixed(1)),
        running: (0xD7D, Raw::Fixed(0x68)),
        countdown: (0xD83, Raw::Fixed(0x75)),
        match_over: (0xD76, Raw::Fixed(0x2A)),
        now: 0x64C8,
        start: 0xD14,
        end: 0xD00,
        duration: 0xD08,
        freeze: (0xD6C, Raw::Fixed(0xE)),
        infinite_display: Some((0xD7B, Raw::Fixed([0x0B, 0x6B]))),
        enc_end: y7s2_enc_end,
        dec_end: y7s2_dec_end,
        enc_duration: y7s2_enc_duration,
        dec_duration: y7s2_dec_duration,
        roster: Roster::Encoded {
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
    },
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
        ("Close Quarter", 0x57AC2280B2),
        ("M.U.T.E. Protocol", 0x40DC810C48),
    ],
    standard_maps: 24,
    thunt_maps: &[
        0, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17, 18, 19, 20, 21, 22,
    ],
    tdm_maps: &[13, 14, 15, 16, 18, 22, 23],
    arcade: &[("Team Deathmatch", WARMUP_MODE, WARMUP_PLAYLIST, &[])],
    events: &[("M.U.T.E. Protocol", 0x59E5A4411F, 0x58CE8D5DB2, &[24])],
};

static Y7S3_GAME: GameProfile = GameProfile {
    tag: "y7s3",
    game_manager: 0x7E00080,
    dec_gm: Some(y7s3_dec_gm),
    state: 0x158,
    dec_state: Some(y7s3_dec_state),
    role: 0x390,
    match_state: MatchState::Indexed {
        entries: 0xE8,
        index: 0xE4,
        stride: 0x90,
    },
    match_layout: Y7_MATCH_LAYOUT,
    gamemode: 0x655DAC8,
    settings: 0x68,
    dec_settings: None,
    time_of_day: 0x40,
    enc_handle_uid: Some(y7s3_enc_handle_uid),
    enc_object_uid: None,
    round: RoundProfile {
        live: (0x344, Raw::Fixed(1)),
        running: (0xED1, Raw::Fixed(0x36)),
        countdown: (0xEC4, Raw::Fixed(0xFB)),
        match_over: (0xEC9, Raw::Fixed(0xEA)),
        now: 0x66F8,
        start: 0xE48,
        end: 0xE84,
        duration: 0xE4C,
        freeze: (0xE90, Raw::Fixed(6)),
        infinite_display: Some((0xEC7, Raw::Fixed([0xFC, 0xF4]))),
        enc_end: y7s3_enc_end,
        dec_end: y7s3_dec_end,
        enc_duration: y7s3_enc_duration,
        dec_duration: y7s3_dec_duration,
        roster: Roster::Encoded {
            lists: [0x5F0, 0x958],
            dec_head: [y7s3_dec_list; 2],
            handles: &[0x370, 0x88],
            dec_handle: y7s3_dec_handle,
            session_handles: None,
        },
        loadout: 0x38,
        dec_loadout: None,
        slots: Y5_SLOTS,
        slot: Some(SlotChain {
            dec: y7s3_dec_slot,
            field: 0x20,
            item: 0x20,
            enc_item: y7s3_enc_item,
        }),
    },
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
        ("Close Quarter", 0x57AC2280B2),
        ("Fort Truth", 0x2F83769722),
        ("Doktor's Curse", 0xB3555E074),
    ],
    standard_maps: 25,
    thunt_maps: &[
        0, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17, 18, 19, 20, 21, 22,
    ],
    tdm_maps: &[13, 14, 15, 16, 18, 22, 23, 24],
    arcade: &[
        ("Team Deathmatch", WARMUP_MODE, WARMUP_PLAYLIST, &[]),
        ("Snipers Only", 0x5A4EE6FC73, 0x5A4EE6FC65, &[2, 25, 9]),
    ],
    events: &[("Doktor's Curse", 0x55CDE1F7AE, 0x55CDE1FAB7, &[26])],
};

static Y7S4_GAME: GameProfile = GameProfile {
    tag: "y7s4",
    game_manager: 0x7D00580,
    dec_gm: Some(y7s4_dec_gm),
    state: 0x15C,
    dec_state: Some(y7s4_dec_state),
    role: 0x390,
    match_state: MatchState::Indexed {
        entries: 0x100,
        index: 0xFC,
        stride: 0x90,
    },
    match_layout: Y7_MATCH_LAYOUT,
    gamemode: 0x68EB5E0,
    settings: 0x68,
    dec_settings: None,
    time_of_day: 0x40,
    enc_handle_uid: Some(y7s4_enc_handle_uid),
    enc_object_uid: None,
    round: RoundProfile {
        live: (0x344, Raw::Fixed(1)),
        running: (0xEC8, Raw::Fixed(0x2A)),
        countdown: (0xECF, Raw::Fixed(0xCF)),
        match_over: (0xEC9, Raw::Fixed(0x9D)),
        now: 0x66F8,
        start: 0xEAC,
        end: 0xEA4,
        duration: 0xEA8,
        freeze: (0xE9C, Raw::Fixed(0x2970F3CF)),
        infinite_display: Some((0xED2, Raw::Fixed([0xCE, 0xAD]))),
        enc_end: y7s4_enc_end,
        dec_end: y7s4_dec_end,
        enc_duration: y7s4_enc_duration,
        dec_duration: y7s4_dec_duration,
        roster: Roster::Encoded {
            lists: [0x678, 0xA40],
            dec_head: [y7s4_dec_list, y7s4_dec_count],
            handles: &[0x110, 0x450],
            dec_handle: y7s4_dec_handle,
            session_handles: None,
        },
        loadout: 0x38,
        dec_loadout: None,
        slots: [0x88, 0x70, 0x58, 0x40],
        slot: Some(SlotChain {
            dec: y7s4_dec_slot,
            field: 0x20,
            item: 0x18,
            enc_item: y7s4_enc_item,
        }),
    },
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
        ("Close Quarter", 0x57AC2280B2),
        ("Snow Brawl", 0x5617C235CE),
    ],
    standard_maps: 26,
    thunt_maps: &[
        0, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17, 18, 19, 20, 21, 22, 24,
    ],
    tdm_maps: &[13, 14, 15, 16, 18, 22, 23, 24, 25],
    arcade: &[("Team Deathmatch", WARMUP_MODE, WARMUP_PLAYLIST, &[])],
    events: &[("Snow Brawl", 0x5797ECFED9, 0x4A9198FBF7, &[26])],
};

static Y8S1_GAME: GameProfile = GameProfile {
    tag: "y8s1",
    game_manager: 0x7927E10,
    dec_gm: Some(y7s4_dec_gm),
    state: 0x1C4,
    dec_state: Some(y8s1_dec_state),
    role: 0x3B8,
    match_state: MatchState::Indexed {
        entries: 0x60,
        index: 0x5C,
        stride: 0x90,
    },
    match_layout: Y7_MATCH_LAYOUT,
    gamemode: 0x6585BB0,
    settings: 0x70,
    dec_settings: Some([y8s1_dec_settings, y8s1_dec_settings_count]),
    time_of_day: 0x40,
    enc_handle_uid: Some(y8s1_enc_handle_uid),
    enc_object_uid: None,
    round: RoundProfile {
        live: (0x1B8, Raw::Fixed(0xCFBCDB01)),
        running: (0xF57, Raw::Fixed(0x17)),
        countdown: (0xF4E, Raw::Fixed(0x86)),
        match_over: (0xF4F, Raw::Fixed(0x76)),
        now: 0x5F08,
        start: 0xEEC,
        end: 0xEF4,
        duration: 0xF48,
        freeze: (0xF28, Raw::Fixed(0x63)),
        infinite_display: Some((0xF4C, Raw::Fixed([0xA3, 0x93]))),
        enc_end: y8s1_enc_end,
        dec_end: y8s1_dec_end,
        enc_duration: y8s1_enc_duration,
        dec_duration: y8s1_dec_duration,
        roster: Roster::Encoded {
            lists: [0x440, 0x908],
            dec_head: [y8s1_dec_list, y8s1_dec_count],
            handles: &[0x4D0, 0x1A0],
            dec_handle: y8s1_dec_handle,
            session_handles: None,
        },
        loadout: 0x18,
        dec_loadout: Some(y8s1_dec_loadout),
        slots: [0x08, 0x48, 0x20, 0x80],
        slot: Some(SlotChain {
            dec: y8s1_dec_slot,
            field: 0x28,
            item: 0x30,
            enc_item: y8s1_enc_item,
        }),
    },
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
        ("Close Quarter", 0x57AC2280B2),
        ("Rainbow is Magic", 0x2BCB159A9C),
    ],
    standard_maps: 26,
    thunt_maps: &[
        0, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17, 18, 19, 20, 21, 22, 24,
    ],
    tdm_maps: &[13, 14, 15, 16, 18, 22, 23, 24, 25],
    arcade: &[
        ("Team Deathmatch", WARMUP_MODE, WARMUP_PLAYLIST, &[]),
        ("Snipers Only", 0x5A4EE6FC73, 0x5A4EE6FC65, &[2, 9]),
        (
            "Golden Gun",
            0x5949AB6753,
            0x3F331EF4E2,
            &[25, 23, 13, 22, 14, 15],
        ),
    ],
    events: &[("Rainbow is Magic", 0x5B55A9D54B, 0x4DB61E9851, &[26])],
};

static Y8S2_GAME: GameProfile = GameProfile {
    tag: "y8s2",
    game_manager: 0x70BD170,
    dec_gm: Some(y8s2_dec_gm),
    state: 0x1C0,
    dec_state: Some(y8s2_dec_state),
    role: 0x3B8,
    match_state: MatchState::Indexed {
        entries: 0x70,
        index: 0x6C,
        stride: 0x90,
    },
    match_layout: Y7_MATCH_LAYOUT,
    gamemode: 0x6745B80,
    settings: 0x60,
    dec_settings: Some([y8s2_dec_settings, y8s2_dec_settings_count]),
    time_of_day: 0x20,
    enc_handle_uid: Some(y8s2_enc_handle_uid),
    enc_object_uid: None,
    round: RoundProfile {
        live: (0x1C4, Raw::Fixed(0x7A43955D)),
        running: (0xF60, Raw::Fixed(0x21)),
        countdown: (0xF69, Raw::Fixed(0x59)),
        match_over: (0xF62, Raw::Fixed(0x9A)),
        now: 0x5F30,
        start: 0xEE0,
        end: 0xEE4,
        duration: 0xF14,
        freeze: (0xEE8, Raw::Fixed(0xC36095E0)),
        infinite_display: Some((0xF65, Raw::Fixed([0xEE, 0x0E]))),
        enc_end: y8s2_enc_end,
        dec_end: y8s2_dec_end,
        enc_duration: y8s2_enc_duration,
        dec_duration: y8s2_dec_duration,
        roster: Roster::Encoded {
            lists: [0x9F0, 0x428],
            dec_head: [y8s2_dec_list, y8s2_dec_count],
            handles: &[0xA0, 0x3D8],
            dec_handle: y8s2_dec_handle,
            session_handles: None,
        },
        loadout: 0x28,
        dec_loadout: Some(y8s2_dec_loadout),
        slots: [0x40, 0x80, 0x60, 0x70],
        slot: Some(SlotChain {
            dec: y8s2_dec_slot,
            field: 0x10,
            item: 0x18,
            enc_item: y8s2_enc_item,
        }),
    },
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
        ("Close Quarter", 0x57AC2280B2),
        ("Rainbow is Magic", 0x2BCB159A9C),
        ("Rengoku", 0x585E25818B),
    ],
    standard_maps: 26,
    thunt_maps: &[
        0, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17, 18, 19, 20, 21, 22, 24,
    ],
    tdm_maps: &[6, 13, 14, 15, 16, 18, 22, 23, 24, 25],
    arcade: &[
        (
            "Free For All",
            0x59D4686E9F,
            0x59D469F60F,
            &[25, 23, 22, 18, 16, 13, 6, 24, 3, 14, 15, 21],
        ),
        ("Team Deathmatch", WARMUP_MODE, WARMUP_PLAYLIST, &[]),
        ("Snipers Only", 0x5A4EE6FC73, 0x5A4EE6FC65, &[2, 9]),
        (
            "Headshots Only",
            0x5A237A780E,
            0x54BBE4E534,
            &[15, 18, 24, 25, 16],
        ),
        (
            "Golden Gun",
            0x5949AB6753,
            0x3F331EF4E2,
            &[25, 23, 13, 22, 14, 15],
        ),
    ],
    events: &[
        ("Rainbow is Magic", 0x5B55A9D54B, 0x4DB61E9851, &[26]),
        ("Rengoku", 0x585B44DA5C, 0x585B4583CD, &[27]),
    ],
};

static Y8S3_GAME: GameProfile = GameProfile {
    tag: "y8s3",
    game_manager: 0x711F310,
    dec_gm: Some(y8s2_dec_gm),
    state: 0x1B8,
    dec_state: Some(y8s3_dec_state),
    role: 0x3B8,
    match_state: MatchState::Indexed {
        entries: 0x88,
        index: 0x84,
        stride: 0xA0,
    },
    match_layout: Y8S3_MATCH_LAYOUT,
    gamemode: 0x65EDB30,
    settings: 0x68,
    dec_settings: Some([y8s3_dec_settings, y8s3_dec_settings_count]),
    time_of_day: 0x80,
    enc_handle_uid: Some(y8s3_enc_handle_uid),
    enc_object_uid: None,
    round: RoundProfile {
        live: (0x1C0, Raw::Fixed(0x157B2A4B)),
        running: (0xF6C, Raw::Fixed(0x92)),
        countdown: (0xF77, Raw::Fixed(0x42)),
        match_over: (0xF73, Raw::Fixed(0x7D)),
        now: 0x5F40,
        start: 0xF0C,
        end: 0xF54,
        duration: 0xF48,
        freeze: (0xF44, Raw::Fixed(0xABC32425)),
        infinite_display: Some((0xF76, Raw::Fixed([0xFF, 0xBF]))),
        enc_end: y8s3_enc_end,
        dec_end: y8s3_dec_end,
        enc_duration: y8s3_enc_duration,
        dec_duration: y8s3_dec_duration,
        roster: Roster::Encoded {
            lists: [0xB20, 0x510],
            dec_head: [y8s3_dec_list, y8s3_dec_count],
            handles: &[0xA8, 0x3E0],
            dec_handle: y8s3_dec_handle,
            session_handles: None,
        },
        loadout: 0x18,
        dec_loadout: Some(y8s3_dec_loadout),
        slots: [0x80, 0x78, 0x40, 0x50],
        slot: Some(SlotChain {
            dec: y8s3_dec_slot,
            field: 0x18,
            item: 0x38,
            enc_item: y8s3_enc_item,
        }),
    },
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
        ("Close Quarter", 0x57AC2280B2),
        ("Doktor's Curse", 0xB3555E074),
    ],
    standard_maps: 26,
    thunt_maps: &[
        0, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17, 18, 19, 20, 21, 22, 24,
    ],
    tdm_maps: &[6, 13, 14, 15, 16, 18, 22, 23, 24, 25],
    arcade: &[
        (
            "Free For All",
            0x59D4686E9F,
            0x59D469F60F,
            &[25, 23, 22, 18, 16, 13, 6, 24, 3, 14, 15, 21],
        ),
        ("Team Deathmatch", WARMUP_MODE, WARMUP_PLAYLIST, &[]),
        ("Snipers Only", 0x5A4EE6FC73, 0x5A4EE6FC65, &[2, 9]),
        (
            "Headshots Only",
            0x5A237A780E,
            0x54BBE4E534,
            &[15, 18, 24, 25, 16],
        ),
        (
            "Golden Gun",
            0x5949AB6753,
            0x3F331EF4E2,
            &[25, 23, 13, 22, 14, 15],
        ),
        (
            "Weapon Roulette",
            0x5B8EF085DD,
            0x5CDFA96065,
            &[25, 23, 13, 24, 16, 18, 15],
        ),
    ],
    events: &[("Doktor's Curse", 0x55CDE1F7AE, 0x55CDE1FAB7, &[26])],
};

static Y8S4_GAME: GameProfile = GameProfile {
    tag: "y8s4",
    game_manager: 0x7306460,
    dec_gm: Some(y8s2_dec_gm),
    state: 0x1B8,
    dec_state: Some(y8s4_dec_state),
    role: 0x3B8,
    match_state: MatchState::Indexed {
        entries: 0x68,
        index: 0x64,
        stride: 0xA0,
    },
    match_layout: Y8S3_MATCH_LAYOUT,
    gamemode: 0x68736C8,
    settings: 0xD0,
    dec_settings: Some([y8s4_dec_settings, y8s4_dec_settings_count]),
    time_of_day: 0x70,
    enc_handle_uid: Some(y8s4_enc_handle_uid),
    enc_object_uid: Some(y8s4_enc_object_uid),
    round: RoundProfile {
        live: (0x1C0, Raw::Fixed(0x59)),
        running: (0x10C4, Raw::Fixed(0x93)),
        countdown: (0x10C7, Raw::Keyed(y8s4_countdown)),
        match_over: (0x10C8, Raw::Fixed(0x04)),
        now: 0x60F0,
        start: 0x104C,
        end: 0x106C,
        duration: 0x10BC,
        freeze: (0x1068, Raw::Fixed(0x24)),
        infinite_display: Some((0x10C6, Raw::Fixed([0xDC, 0xDA]))),
        enc_end: y8s4_enc_end,
        dec_end: y8s4_dec_end,
        enc_duration: y8s4_enc_duration,
        dec_duration: y8s4_dec_duration,
        roster: Roster::Encoded {
            lists: [0xC10, 0x878],
            dec_head: [y8s4_dec_list, y8s4_dec_count],
            handles: &[0xB0, 0x3E0],
            dec_handle: y8s4_dec_handle,
            session_handles: None,
        },
        loadout: 0x18,
        dec_loadout: Some(y8s4_dec_loadout),
        slots: [0x10, 0x00, 0x18, 0x50],
        slot: Some(SlotChain {
            dec: y8s4_dec_slot,
            field: 0x28,
            item: 0x28,
            enc_item: y8s4_enc_item,
        }),
    },
    maps: &[
        ("Oregon", 0x35F2901CF4),
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
        ("Lair", 0x5A5AF8ECF7),
        ("Close Quarter", 0x57AC2280B2),
        ("Freeze For All", 0x5C4B99AE00),
    ],
    standard_maps: 26,
    thunt_maps: &[],
    tdm_maps: &[5, 12, 13, 14, 15, 17, 21, 25, 23, 24, 22],
    arcade: &[
        (
            "Free For All",
            0x59D4686E9F,
            0x59D469F60F,
            &[25, 23, 21, 17, 15, 12, 5, 24, 2, 13, 14, 20, 22],
        ),
        ("Team Deathmatch", WARMUP_MODE, WARMUP_PLAYLIST, &[]),
        ("Snipers Only", 0x5A4EE6FC73, 0x5A4EE6FC65, &[1, 8]),
        (
            "Headshots Only",
            0x5A237A780E,
            0x54BBE4E534,
            &[14, 17, 23, 25, 15],
        ),
        (
            "Golden Gun",
            0x5949AB6753,
            0x3F331EF4E2,
            &[25, 22, 12, 21, 13, 24, 14],
        ),
        (
            "Weapon Roulette",
            0x5B8EF085DD,
            0x5CDFA96065,
            &[25, 14, 12, 23, 15, 17, 24, 22],
        ),
    ],
    events: &[("Freeze For All", 0x5C895B9F5C, 0x5C895BA04D, &[26])],
};

static Y9S1_MATCH: MatchCodec = MatchCodec {
    index: (0xBC, y9s1_dec_match_index),
    entries: (0xC0, [y9s1_dec_match_data, y9s1_dec_match_count]),
    enc_playlist: y9s1_enc_playlist,
    enc_mode: y9s1_enc_mode,
    enc_map: y9s1_enc_map,
    dec_tod: y9s1_dec_match_tod,
    settings: [y9s1_dec_match_settings, y9s1_dec_match_settings_count],
    enc_settings_count: y9s1_enc_match_settings_count,
};

static Y9S1_GAME: GameProfile = GameProfile {
    tag: "y9s1",
    game_manager: 0x7F25C00,
    dec_gm: Some(y8s2_dec_gm),
    state: 0x1CC,
    dec_state: Some(y9s1_dec_state),
    role: 0x3C0,
    match_state: MatchState::Encoded(&Y9S1_MATCH),
    match_layout: Y9S1_MATCH_LAYOUT,
    gamemode: 0x741FE48,
    settings: 0x98,
    dec_settings: Some([y9s1_dec_settings, y9s1_dec_settings_count]),
    time_of_day: 0xB0,
    enc_handle_uid: Some(y9s1_enc_handle_uid),
    enc_object_uid: Some(y9s1_enc_object_uid),
    round: RoundProfile {
        live: (0x1C8, Raw::Fixed(0x215F8540)),
        running: (0x1142, Raw::Fixed(0x49)),
        countdown: (0x1139, Raw::Keyed(y9s1_countdown)),
        match_over: (0x1141, Raw::Fixed(0x61)),
        now: 0x5C10,
        start: 0x10B4,
        end: 0x10C8,
        duration: 0x1128,
        freeze: (0x10B0, Raw::Keyed(y9s1_unfrozen)),
        infinite_display: Some((0x113B, Raw::Fixed([0x6C, 0x7C]))),
        enc_end: y9s1_enc_end,
        dec_end: y9s1_dec_end,
        enc_duration: y9s1_enc_duration,
        dec_duration: y9s1_dec_duration,
        roster: Roster::Encoded {
            lists: [0xC08, 0x408],
            dec_head: [y9s1_dec_list, y9s1_dec_count],
            handles: &[0xC0, 0x3F8],
            dec_handle: y9s1_dec_handle,
            session_handles: None,
        },
        loadout: 0x30,
        dec_loadout: Some(y9s1_dec_loadout),
        slots: [0x98, 0x48, 0x90, 0x28],
        slot: Some(SlotChain {
            dec: y9s1_dec_slot,
            field: 0x28,
            item: 0x20,
            enc_item: y9s1_enc_item,
        }),
    },
    maps: &[
        ("Oregon", 0x35F2901CF4),
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
        ("Lair", 0x5A5AF8ECF7),
        ("Close Quarter", 0x57AC2280B2),
        ("Containment", 0x5464BBF809),
    ],
    standard_maps: 26,
    thunt_maps: &[],
    tdm_maps: &[5, 12, 13, 14, 15, 17, 21, 25, 23, 24, 22],
    arcade: &[
        (
            "Free For All",
            0x59D4686E9F,
            0x59D469F60F,
            &[25, 23, 21, 17, 15, 12, 5, 24, 2, 13, 14, 20, 22],
        ),
        ("Team Deathmatch", WARMUP_MODE, WARMUP_PLAYLIST, &[]),
        ("Snipers Only", 0x5A4EE6FC73, 0x5A4EE6FC65, &[1, 8]),
        (
            "Headshots Only",
            0x5A237A780E,
            0x54BBE4E534,
            &[14, 17, 23, 25, 15],
        ),
        (
            "Golden Gun",
            0x5949AB6753,
            0x3F331EF4E2,
            &[25, 22, 12, 21, 13, 24, 14],
        ),
        (
            "Weapon Roulette",
            0x5B8EF085DD,
            0x5CDFA96065,
            &[25, 14, 12, 23, 15, 17, 24, 22],
        ),
    ],
    events: &[("Containment", 0x547CD662FD, 0x54A95068E8, &[26])],
};

static Y9S2_MATCH: MatchCodec = MatchCodec {
    index: (0x15D, y9s2_dec_match_index),
    entries: (0x160, [y9s2_dec_match_data, y9s2_dec_match_count]),
    enc_playlist: y9s2_enc_playlist,
    enc_mode: y9s2_enc_mode,
    enc_map: y9s2_enc_map,
    dec_tod: y9s2_dec_match_tod,
    settings: [y9s2_dec_match_settings, y9s2_dec_match_settings_count],
    enc_settings_count: y9s2_enc_match_settings_count,
};

static Y9S2_GAME: GameProfile = GameProfile {
    tag: "y9s2",
    game_manager: 0x81F9118,
    dec_gm: Some(y7s4_dec_gm),
    state: 0x1CC,
    dec_state: Some(y9s2_dec_state),
    role: 0x3C0,
    match_state: MatchState::Encoded(&Y9S2_MATCH),
    match_layout: Y9S2_MATCH_LAYOUT,
    gamemode: 0x76BA838,
    settings: 0x48,
    dec_settings: Some([y9s2_dec_settings, y9s2_dec_settings_count]),
    time_of_day: 0xD0,
    enc_handle_uid: Some(y9s2_enc_handle_uid),
    enc_object_uid: Some(y9s2_enc_object_uid),
    round: RoundProfile {
        live: (0x1C8, Raw::Keyed(y9s2_live)),
        running: (0x1141, Raw::Fixed(0x80)),
        countdown: (0x114D, Raw::Keyed(y9s2_countdown)),
        match_over: (0x114B, Raw::Keyed(y9s2_match_over)),
        now: 0x5C30,
        start: 0x1118,
        end: 0x1120,
        duration: 0x10C4,
        freeze: (0x1104, Raw::Keyed(y9s2_unfrozen)),
        infinite_display: Some((0x1142, Raw::Keyed(y9s2_display))),
        enc_end: y9s2_enc_end,
        dec_end: y9s2_dec_end,
        enc_duration: y9s2_enc_duration,
        dec_duration: y9s2_dec_duration,
        roster: Roster::Encoded {
            lists: [0xAF0, 0x4A8],
            dec_head: [y9s2_dec_list, y9s2_dec_count],
            handles: &[0xB8, 0x3F0],
            dec_handle: y9s2_dec_handle,
            session_handles: None,
        },
        loadout: 0x20,
        dec_loadout: Some(y9s2_dec_loadout),
        slots: [0x38, 0x68, 0x08, 0xA0],
        slot: Some(SlotChain {
            dec: y9s2_dec_slot,
            field: 0x10,
            item: 0x30,
            enc_item: y9s2_enc_item,
        }),
    },
    maps: &[
        ("Oregon", 0x35F2901CF4),
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
        ("Stadium Bravo - Rework", 0x5D7B68A718),
        ("Nighthaven Labs", 0x58260EEFB3),
        ("Lair", 0x5A5AF8ECF7),
        ("Stadium Alpha", 0x5E5E235604),
        ("Close Quarter", 0x57AC2280B2),
        ("M.U.T.E. Protocol", 0x40DC810C48),
    ],
    standard_maps: 28,
    thunt_maps: &[],
    tdm_maps: &[5, 25, 13, 27, 15, 17, 24, 26, 11, 2, 12, 14, 21, 22],
    arcade: &[
        (
            "Free For All",
            0x59D4686E9F,
            0x59D469F60F,
            &[27, 24, 5, 17, 15, 25, 26, 11, 0, 14, 2, 12, 13, 20, 21, 22],
        ),
        ("Team Deathmatch", WARMUP_MODE, WARMUP_PLAYLIST, &[]),
        ("Snipers Only", 0x5A4EE6FC73, 0x5A4EE6FC65, &[1, 8]),
        (
            "Headshots Only",
            0x5A237A780E,
            0x54BBE4E534,
            &[14, 17, 24, 27, 15],
        ),
        (
            "Golden Gun",
            0x5949AB6753,
            0x3F331EF4E2,
            &[27, 22, 23, 12, 21, 13, 25, 26, 14, 19],
        ),
        (
            "Weapon Roulette",
            0x5B8EF085DD,
            0x5CDFA96065,
            &[27, 14, 12, 24, 15, 25, 26, 22, 23, 19, 17],
        ),
        ("Attrition", 0x5DD3A86E8C, 0x5DD3A86E4E, &[2, 5, 11, 0, 25]),
    ],
    events: &[("M.U.T.E. Protocol", 0x59E5A4411F, 0x58CE8D5DB2, &[28])],
};

static Y9S3_MATCH: MatchCodec = MatchCodec {
    index: (0x14C, y9s3_dec_match_index),
    entries: (0x150, [y9s3_dec_match_data, y9s3_dec_match_count]),
    enc_playlist: y9s3_enc_playlist,
    enc_mode: y9s3_enc_mode,
    enc_map: y9s3_enc_map,
    dec_tod: y9s3_dec_match_tod,
    settings: [y9s3_dec_match_settings, y9s3_dec_match_settings_count],
    enc_settings_count: y9s3_enc_match_settings_count,
};

static Y9S3_GAME: GameProfile = GameProfile {
    tag: "y9s3",
    game_manager: 0xBA92A88,
    dec_gm: Some(y7s4_dec_gm),
    state: 0x1CC,
    dec_state: Some(y9s3_dec_state),
    role: 0x3C8,
    match_state: MatchState::Encoded(&Y9S3_MATCH),
    match_layout: Y9S3_MATCH_LAYOUT,
    gamemode: 0xB178588,
    settings: 0xD0,
    dec_settings: Some([y9s3_dec_settings, y9s3_dec_settings_count]),
    time_of_day: 0x80,
    enc_handle_uid: Some(y9s3_enc_handle_uid),
    enc_object_uid: Some(y9s3_enc_object_uid),
    round: RoundProfile {
        live: (0x1C8, Raw::Keyed(y9s3_live)),
        running: (0x10A0, Raw::Fixed(0x5E)),
        countdown: (0x109D, Raw::Fixed(0xC9)),
        match_over: (0x109C, Raw::Keyed(y9s3_match_over)),
        now: 0x1DEC8,
        start: 0x106C,
        end: 0x103C,
        duration: 0x107C,
        freeze: (0x1064, Raw::Keyed(y9s3_unfrozen)),
        infinite_display: Some((0x109A, Raw::Keyed(y9s3_display))),
        enc_end: y9s3_enc_end,
        dec_end: y9s3_dec_end,
        enc_duration: y9s3_enc_duration,
        dec_duration: y9s3_dec_duration,
        roster: Roster::Encoded {
            lists: [0xCE8, 0x8F0],
            dec_head: [y9s3_dec_list, y9s3_dec_count],
            handles: &[0x88, 0x3C0],
            dec_handle: y9s3_dec_handle,
            session_handles: None,
        },
        loadout: 0x28,
        dec_loadout: Some(y9s3_dec_loadout),
        slots: [0x10, 0x80, 0x00, 0x88],
        slot: Some(SlotChain {
            dec: y9s3_dec_slot,
            field: 0x10,
            item: 0x20,
            enc_item: y9s3_enc_item,
        }),
    },
    maps: &[
        ("Oregon", 0x35F2901CF4),
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
        ("Stadium Bravo - Rework", 0x5D7B68A718),
        ("Nighthaven Labs", 0x58260EEFB3),
        ("Lair", 0x5A5AF8ECF7),
        ("Stadium Alpha", 0x5E5E235604),
        ("Close Quarter", 0x57AC2280B2),
        ("Doktor's Curse", 0xB3555E074),
    ],
    standard_maps: 28,
    thunt_maps: &[],
    tdm_maps: &[5, 25, 13, 27, 15, 17, 24, 26, 11, 2, 12, 14, 21, 22],
    arcade: &[
        (
            "Free For All",
            0x59D4686E9F,
            0x59D469F60F,
            &[27, 24, 5, 17, 15, 25, 26, 11, 0, 14, 2, 12, 13, 20, 21, 22],
        ),
        ("Team Deathmatch", WARMUP_MODE, WARMUP_PLAYLIST, &[]),
        ("Snipers Only", 0x5A4EE6FC73, 0x5A4EE6FC65, &[1, 8]),
        (
            "Headshots Only",
            0x5A237A780E,
            0x54BBE4E534,
            &[14, 17, 24, 27, 15],
        ),
        (
            "Golden Gun",
            0x5949AB6753,
            0x3F331EF4E2,
            &[27, 22, 23, 12, 21, 13, 25, 26, 14, 19],
        ),
        (
            "Weapon Roulette",
            0x5FCFDA4DBF,
            0x5FCFDA4CB5,
            &[27, 14, 12, 24, 15, 25, 26, 22, 23, 19, 17],
        ),
        ("Attrition", 0x5DD3A86E8C, 0x5DD3A86E4E, &[2, 5, 11, 0, 25]),
    ],
    events: &[("Doktor's Curse", 0x55CDE1F7AE, 0x55CDE1FAB7, &[28])],
};

static Y9S4_MATCH: MatchCodec = MatchCodec {
    index: (0x135, y9s4_dec_match_index),
    entries: (0x138, [y9s4_dec_match_data, y9s4_dec_match_count]),
    enc_playlist: y9s4_enc_playlist,
    enc_mode: y9s4_enc_mode,
    enc_map: y9s4_enc_map,
    dec_tod: y9s4_dec_match_tod,
    settings: [y9s4_dec_match_settings, y9s4_dec_match_settings_count],
    enc_settings_count: y9s4_enc_match_settings_count,
};

static Y9S4_GAME: GameProfile = GameProfile {
    tag: "y9s4",
    game_manager: 0xD4A6668,
    dec_gm: Some(y8s2_dec_gm),
    state: 0x1C4,
    dec_state: Some(y9s4_dec_state),
    role: 0x3C8,
    match_state: MatchState::Encoded(&Y9S4_MATCH),
    match_layout: Y9S4_MATCH_LAYOUT,
    gamemode: 0xC9B9F88,
    settings: 0xD0,
    dec_settings: Some([y9s4_dec_settings, y9s4_dec_settings_count]),
    time_of_day: 0x28,
    enc_handle_uid: Some(y9s4_enc_handle_uid),
    enc_object_uid: Some(y9s4_enc_object_uid),
    round: RoundProfile {
        live: (0x1C0, Raw::Fixed(0x57770188)),
        running: (0x1127, Raw::Keyed(y9s4_running)),
        countdown: (0x112C, Raw::Keyed(y9s4_countdown)),
        match_over: (0x1128, Raw::Keyed(y9s4_match_over)),
        now: 0x1DFA8,
        start: 0x1118,
        end: 0x10D8,
        duration: 0x10E0,
        freeze: (0x10AC, Raw::Keyed(y9s4_unfrozen)),
        infinite_display: Some((0x1133, Raw::Keyed(y9s4_display))),
        enc_end: y9s4_enc_end,
        dec_end: y9s4_dec_end,
        enc_duration: y9s4_enc_duration,
        dec_duration: y9s4_dec_duration,
        roster: Roster::Encoded {
            lists: [0xCD0, 0x8A0],
            dec_head: [y9s4_dec_list, y9s4_dec_count],
            handles: &[0xE8, 0x428],
            dec_handle: y9s4_dec_handle,
            session_handles: None,
        },
        loadout: 0x30,
        dec_loadout: Some(y9s4_dec_loadout),
        slots: [0x50, 0x90, 0x00, 0x78],
        slot: Some(SlotChain {
            dec: y9s4_dec_slot,
            field: 0x20,
            item: 0x28,
            enc_item: y9s4_enc_item,
        }),
    },
    maps: &[
        ("Oregon", 0x35F2901CF4),
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
        ("Stadium Bravo - Rework", 0x5D7B68A718),
        ("Nighthaven Labs", 0x58260EEFB3),
        ("Lair", 0x5A5AF8ECF7),
        ("Stadium Alpha", 0x5E5E235604),
        ("Close Quarter", 0x57AC2280B2),
        ("Freeze For All", 0x5C4B99AE00),
    ],
    standard_maps: 28,
    thunt_maps: &[],
    tdm_maps: &[5, 25, 13, 27, 15, 17, 24, 26, 11, 2, 12, 14, 21, 22],
    arcade: &[
        (
            "Free For All",
            0x59D4686E9F,
            0x59D469F60F,
            &[27, 24, 5, 17, 15, 25, 26, 11, 0, 14, 2, 12, 13, 20, 21, 22],
        ),
        ("Team Deathmatch", WARMUP_MODE, WARMUP_PLAYLIST, &[]),
        ("Snipers Only", 0x5A4EE6FC73, 0x5A4EE6FC65, &[1, 8]),
        (
            "Headshots Only",
            0x5A237A780E,
            0x54BBE4E534,
            &[14, 17, 24, 27, 15],
        ),
        (
            "Golden Gun",
            0x5949AB6753,
            0x3F331EF4E2,
            &[27, 22, 23, 12, 21, 13, 25, 26, 14, 19],
        ),
        (
            "Weapon Roulette",
            0x5FCFDA4DBF,
            0x5FCFDA4CB5,
            &[27, 14, 12, 24, 15, 25, 26, 22, 23, 19, 17],
        ),
        ("Attrition", 0x5DD3A86E8C, 0x5DD3A86E4E, &[2, 5, 11, 0, 25]),
    ],
    events: &[("Freeze For All", 0x5C895B9F5C, 0x5C895BA04D, &[28])],
};

static Y10S1_MATCH: MatchCodec = MatchCodec {
    index: (0xFE, y10s1_dec_match_index),
    entries: (0x100, [y10s1_dec_match_data, y10s1_dec_match_count]),
    enc_playlist: y10s1_enc_playlist,
    enc_mode: y10s1_enc_mode,
    enc_map: y10s1_enc_map,
    dec_tod: y10s1_dec_match_tod,
    settings: [y10s1_dec_match_settings, y10s1_dec_match_settings_count],
    enc_settings_count: y10s1_enc_match_settings_count,
};

static Y10S1_GAME: GameProfile = GameProfile {
    tag: "y10s1",
    game_manager: 0xE3AEEE0,
    dec_gm: Some(y10s1_dec_gm),
    state: 0x1C0,
    dec_state: Some(y10s1_dec_state),
    role: 0x3C8,
    match_state: MatchState::Encoded(&Y10S1_MATCH),
    match_layout: Y10S1_MATCH_LAYOUT,
    gamemode: 0xD7A9E10,
    settings: 0x20,
    dec_settings: Some([y10s1_dec_settings, y10s1_dec_settings_count]),
    time_of_day: 0xA8,
    enc_handle_uid: Some(y10s1_enc_handle_uid),
    enc_object_uid: Some(y10s1_enc_object_uid),
    round: RoundProfile {
        live: (0x1C4, Raw::Keyed(y10s1_live)),
        running: (0x1143, Raw::Keyed(y10s1_running)),
        countdown: (0x1142, Raw::Fixed(0xF3)),
        match_over: (0x113A, Raw::Keyed(y10s1_match_over)),
        now: 0x1E010,
        start: 0x1100,
        end: 0x10B8,
        duration: 0x1120,
        freeze: (0x1108, Raw::Keyed(y10s1_unfrozen)),
        infinite_display: Some((0x1134, Raw::Keyed(y10s1_display))),
        enc_end: y10s1_enc_end,
        dec_end: y10s1_dec_end,
        enc_duration: y10s1_enc_duration,
        dec_duration: y10s1_dec_duration,
        roster: Roster::Encoded {
            lists: [0xDD0, 0xAC8],
            dec_head: [y10s1_dec_list, y10s1_dec_count],
            handles: &[0xB0, 0x3E8],
            dec_handle: y10s1_dec_handle,
            session_handles: None,
        },
        loadout: 0x18,
        dec_loadout: Some(y10s1_dec_loadout),
        slots: [0xA0, 0x90, 0x00, 0x40],
        slot: Some(SlotChain {
            dec: y10s1_dec_slot,
            field: 0x18,
            item: 0x28,
            enc_item: y10s1_enc_item,
        }),
    },
    maps: &[
        ("Oregon", 0x35F2901CF4),
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
        ("Stadium Bravo - Rework", 0x5D7B68A718),
        ("Nighthaven Labs", 0x58260EEFB3),
        ("Lair", 0x5A5AF8ECF7),
        ("Stadium Alpha", 0x5E5E235604),
        ("Close Quarter", 0x57AC2280B2),
    ],
    standard_maps: 27,
    thunt_maps: &[],
    tdm_maps: &[4, 24, 12, 26, 14, 16, 23, 25, 10, 1, 11, 13, 20, 21],
    arcade: &[
        (
            "Free For All",
            0x59D4686E9F,
            0x59D469F60F,
            &[26, 23, 4, 16, 14, 24, 25, 10, 0, 13, 1, 11, 12, 19, 20, 21],
        ),
        ("Team Deathmatch", WARMUP_MODE, WARMUP_PLAYLIST, &[]),
        ("Snipers Only", 0x5A4EE6FC73, 0x5A4EE6FC65, &[7]),
        (
            "Headshots Only",
            0x5A237A780E,
            0x54BBE4E534,
            &[13, 16, 23, 26, 14],
        ),
        (
            "Golden Gun",
            0x5949AB6753,
            0x3F331EF4E2,
            &[26, 21, 22, 11, 20, 12, 24, 25, 13, 18],
        ),
        (
            "Weapon Roulette",
            0x5FCFDA4DBF,
            0x5FCFDA4CB5,
            &[26, 13, 11, 23, 14, 24, 25, 21, 22, 18, 16],
        ),
        ("Attrition", 0x5DD3A86E8C, 0x5DD3A86E4E, &[1, 4, 10, 0, 24]),
    ],
    events: &[],
};

fn game_profile(build: &str) -> Option<&'static GameProfile> {
    match build {
        "Y5S1_14303219" => Some(&Y5S1_GAME),
        "Y5S2_14664131" => Some(&Y5S2_GAME),
        "Y5S3_15018155" => Some(&Y5S3_GAME),
        "Y5S4_15241382" => Some(&Y5S4_GAME),
        "Y6S1_15680437" => Some(&Y6S1_GAME),
        "Y6S2_15772114" => Some(&Y6S2_GAME),
        "Y6S3_35130040" => Some(&Y6S3_GAME),
        "Y6S4_37046065" => Some(&Y6S4_GAME),
        "Y7S1_39785137" => Some(&Y7S1_GAME),
        "Y7S2_43489433" => Some(&Y7S2_GAME),
        "Y7S3_47165842" => Some(&Y7S3_GAME),
        "Y7S4_50497889" => Some(&Y7S4_GAME),
        "Y8S1_55217154" => Some(&Y8S1_GAME),
        "Y8S2_58222837" => Some(&Y8S2_GAME),
        "Y8S3_62486471" => Some(&Y8S3_GAME),
        "Y8S4_67466841" => Some(&Y8S4_GAME),
        "Y9S1_69866602" => Some(&Y9S1_GAME),
        "Y9S2_72730050" => Some(&Y9S2_GAME),
        "Y9S3_79429095" => Some(&Y9S3_GAME),
        "Y9S4_82873685" => Some(&Y9S4_GAME),
        "Y10S1_87835624" => Some(&Y10S1_GAME),
        _ => None,
    }
}

#[derive(Clone, Copy)]
struct Pick {
    mode: u64,
    map: u64,
    difficulty: u64,
    playlist: u64,
}

struct Lobby {
    m: u64,
    base: u64,
    playlist: u64,
    tod_at: u64,
    tod: u64,
    entries: Vec<u64>,
}

fn parse_pick(game: &GameProfile, id: &str) -> Option<Pick> {
    let parts: Vec<&str> = id.split(':').collect();
    if parts.first() != Some(&game.tag) {
        return None;
    }
    let n = |i: usize| parts.get(i)?.parse::<usize>().ok();
    let map = |i: usize| Some(game.maps.get(n(i)?)?.1);
    let plain = |modes: &[(&str, u64)], difficulty: u64| {
        Some(Pick {
            mode: modes.get(n(2)?)?.1,
            map: map(3)?,
            difficulty,
            playlist: 0,
        })
    };
    match *parts.get(1)? {
        "mp" => plain(MULTIPLAYER_MODES, 0).map(|pick| Pick {
            playlist: if pick.mode == WARMUP_MODE && !game.tdm_maps.is_empty() {
                WARMUP_PLAYLIST
            } else {
                0
            },
            ..pick
        }),
        "dev" => Some(Pick {
            mode: GYM_MODE,
            map: map(2)?,
            difficulty: 0,
            playlist: 0,
        }),
        "th" if game.thunt_maps.contains(&n(3)?) => {
            plain(TERRORIST_HUNT_MODES, DIFFICULTIES.get(n(4)?)?.1)
        }
        kind @ ("ev" | "arc") => {
            let table = if kind == "ev" {
                game.events
            } else {
                game.arcade
            };
            let (_, mode, playlist, _) = table.get(n(2)?)?;
            Some(Pick {
                mode: *mode,
                map: map(3)?,
                difficulty: 0,
                playlist: *playlist,
            })
        }
        _ => None,
    }
}

fn build_profile_tree_json(game: &GameProfile, installed: impl Fn(u64) -> bool) -> String {
    let tag = game.tag;
    let maps = |prefix: String, warmup: bool| {
        (0..game.standard_maps)
            .filter(|&map| warmup || !WARMUP_MAPS.contains(&game.maps[map].1))
            .map(|map| leaf(game.maps[map].0, format!("{prefix}:{map}")))
            .collect::<Vec<_>>()
    };
    let mut multiplayer: Vec<TNode> = MULTIPLAYER_MODES
        .iter()
        .enumerate()
        .map(|(m, &(name, mode))| match game.tdm_maps {
            tdm if mode == WARMUP_MODE && !tdm.is_empty() => branch(
                "Team Deathmatch",
                tdm.iter()
                    .map(|&map| leaf(game.maps[map].0, format!("{tag}:mp:{m}:{map}")))
                    .collect(),
            ),
            _ => branch(name, maps(format!("{tag}:mp:{m}"), mode == WARMUP_MODE)),
        })
        .collect();
    let tdm = multiplayer.pop().unwrap_or_default();
    let modes = game
        .arcade
        .iter()
        .enumerate()
        .map(|(a, (name, mode, _, maps))| {
            if *mode == WARMUP_MODE {
                return tdm.clone();
            }
            let leaves = maps
                .iter()
                .filter(|&&map| installed(game.maps[map].1))
                .map(|&map| leaf(game.maps[map].0, format!("{tag}:arc:{a}:{map}")))
                .collect();
            branch(name, leaves)
        })
        .collect();
    multiplayer.push(branch("Arcade", modes));
    let mut gym = maps(format!("{tag}:dev"), false);
    gym.retain(|map| map.text != "Lair");
    let hunt = TERRORIST_HUNT_MODES
        .iter()
        .enumerate()
        .map(|(m, (name, _))| {
            let maps = game
                .thunt_maps
                .iter()
                .map(|&map| {
                    let levels = DIFFICULTIES
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
        let leaves: Vec<TNode> = maps
            .iter()
            .filter(|&&map| installed(game.maps[map].1))
            .map(|&map| leaf(game.maps[map].0, format!("{tag}:ev:{e}:{map}")))
            .collect();
        if leaves.is_empty() {
            continue;
        }
        match events.iter_mut().find(|node| node.text == *name) {
            Some(group) => group.children.extend(leaves),
            None => events.push(branch(name, leaves)),
        }
    }
    for node in &mut events {
        if node.children.len() == 1 {
            node.id = node.children.remove(0).id;
        }
    }
    let mut roots = vec![branch("Multiplayer", multiplayer)];
    if !game.thunt_maps.is_empty() {
        roots.push(branch("Terrorist Hunt", hunt));
    }
    roots.push(branch("Development", gym));
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
    gametype: u64,
    disabled: [bool; DISABLE_MODS.len()],
    infinite_time: bool,
    phase: Option<Phase>,
    match_end: bool,
    pick: Option<Pick>,
    handles: HashMap<u64, u64>,
    playlists: HashMap<u64, u64>,
    scanned_match: u64,
    lobbies: Vec<Lobby>,
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
            gametype: 0,
            disabled: [false; DISABLE_MODS.len()],
            infinite_time: false,
            phase: None,
            match_end: false,
            pick: None,
            handles: HashMap::new(),
            playlists: HashMap::new(),
            scanned_match: 0,
            lobbies: Vec::new(),
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
                .write_mem(self.eng.at(p.addr), self.eng.bytes(p.addr, p.bytes));
        }
    }

    fn is_full_feature(&self) -> bool {
        (0..SEASON_Y5S1).contains(&self.season) || game_profile(self.build).is_some()
    }

    fn match_entry(&self, game: &GameProfile, gm: u64) -> u64 {
        match game.match_state {
            MatchState::Indexed {
                entries,
                index,
                stride,
            } => {
                let data = self.eng.read_u64(gm + entries);
                let count = self.eng.read_u64(gm + entries + 8) & 0x3FFF_FFFF;
                let index = u64::from(self.eng.read_u8(gm + index));
                if data == 0 || index >= count {
                    0
                } else {
                    data + index * stride
                }
            }
            MatchState::Encoded(codec) => {
                let (at, [dec_data, dec_count]) = codec.entries;
                let data = dec_data(self.eng.read_u64(gm + at));
                let count = dec_count(self.eng.read_u64(gm + at + 8)) & 0x3FFF_FFFF;
                let index = u64::from((codec.index.1)(self.eng.read_u8(gm + codec.index.0)));
                if data == 0 || index >= count {
                    0
                } else {
                    data + index * 0xA0
                }
            }
        }
    }

    fn match_base(&self, game: &GameProfile, gm: u64) -> u64 {
        match game.match_state {
            MatchState::Indexed { entries, .. } => self.eng.read_u64(gm + entries),
            MatchState::Encoded(codec) => {
                (codec.entries.1[0])(self.eng.read_u64(gm + codec.entries.0))
            }
        }
    }

    fn deref_chain(&self, start: u64, offsets: &[i32]) -> u64 {
        let mut a = start;
        for off in offsets {
            if a == 0 {
                return 0;
            }
            a = self.eng.read_ptr(a.wrapping_add(*off as i64 as u64));
        }
        a
    }

    fn loadout_container(&self, round: &RoundProfile, loadout: u64) -> u64 {
        match round.dec_loadout {
            Some(dec) if loadout != 0 => dec(self
                .eng
                .read_u64(loadout.wrapping_add(round.loadout as u64))),
            Some(_) => 0,
            None => self.deref_chain(loadout, &[round.loadout]),
        }
    }

    fn apply_disable(&self) {
        if !self.disabled.iter().any(|on| *on) {
            return;
        }
        let Some((game, round)) = self.round_profile() else {
            return;
        };
        let gm = self.gm(game);
        if !(5..=7).contains(&self.game_state(game, gm)) {
            return;
        }
        let containers: Vec<u64> = match round.roster {
            Roster::Plain { list, handle } => (-5..5)
                .map(|player| {
                    self.loadout_container(
                        round,
                        self.deref_chain(gm, &[list, player * 8, 0x00, handle, 0x00]),
                    )
                })
                .collect(),
            Roster::Encoded {
                lists,
                dec_head,
                handles,
                dec_handle,
                session_handles,
            } => {
                let handles = session_handles
                    .filter(|(config, flags, _)| {
                        let host = self.eng.read_ptr(self.eng.at(*config));
                        host != 0 && flags.iter().all(|f| self.eng.read_u8(host + f) != 0)
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
                    .map(|entry| self.deref_chain(entry, &[0x00, 0x00]))
                    .filter(|&pawn| pawn != 0)
                    .flat_map(|pawn| {
                        handles.iter().map(move |at| {
                            self.loadout_container(
                                round,
                                self.deref_chain(dec_handle(self.eng.read_u64(pawn + at)), &[0x00]),
                            )
                        })
                    })
                    .collect()
            }
        };
        let slot = round.slot.as_ref();
        let (item, empty) = slot.map_or((0x18, 0), |slot| (slot.item, (slot.enc_item)(0)));
        for container in containers {
            if container == 0 {
                continue;
            }
            for (i, at) in round.slots.iter().enumerate() {
                if !self.disabled[i] {
                    continue;
                }
                let child = self.deref_chain(container, &[*at]);
                let entry = match slot {
                    Some(slot) if child != 0 => {
                        self.deref_chain((slot.dec)(self.eng.read_u64(child + slot.field)), &[0x00])
                    }
                    Some(_) => 0,
                    None => self.deref_chain(child, &[0x20, 0x00]),
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
        match game_profile(self.build) {
            Some(game) => build_profile_tree_json(game, |map| self.event_installed(map)),
            None => self.build_tree_json(),
        }
    }

    fn event_installed(&self, map: u64) -> bool {
        let mut forges = EVENT_FORGES
            .iter()
            .filter(|&&(uid, _)| uid == map)
            .peekable();
        forges.peek().is_none() || forges.any(|(_, forge)| self.forge_installed(forge))
    }

    fn classic_event_installed(&self, event: &str) -> bool {
        CLASSIC_EVENT_FORGES
            .iter()
            .filter(|(name, _)| *name == event)
            .all(|(_, forge)| self.forge_installed(forge))
    }

    fn forge_installed(&self, forge: &str) -> bool {
        self.eng.dir.as_os_str().is_empty()
            || self
                .eng
                .dir
                .join(format!("datapc64_{forge}.forge"))
                .is_file()
    }

    fn briefing_installed(&self, situation: &str) -> bool {
        let Some(number) = situation.get(..2).and_then(|n| n.parse::<u32>().ok()) else {
            return true;
        };
        let video = format!("training_m{number:02}");
        self.eng.dir.as_os_str().is_empty()
            || std::fs::read_dir(self.eng.dir.join("videos")).is_ok_and(|files| {
                files.flatten().any(|file| {
                    file.file_name()
                        .to_string_lossy()
                        .to_ascii_lowercase()
                        .starts_with(&video)
                })
            })
    }

    fn write_chain(&self, feature: &str, bytes: &[u8]) {
        if let Some(fc) = find_chain(feature, self.build) {
            let a = self.eng.resolve_ptr_chain(fc.base_offset, fc.offsets);
            self.eng.write_mem(a, bytes);
        }
    }

    fn skip(&mut self) {
        if !self.patched {
            return;
        }
        if let Some((game, round)) = self.round_profile() {
            let gm = self.host_gm(game);
            if gm != 0
                && (4..=7).contains(&self.game_state(game, gm))
                && self.eng.read_u32(gm + round.now) != 0
            {
                let (offset, unfrozen) = round.freeze;
                self.eng
                    .write_mem(gm + offset, &unfrozen.get().to_le_bytes());
                self.time_out(round, gm);
            }
        } else if let Some(timer) = self.classic_timer() {
            let gm = self.classic_gm(timer);
            if gm != 0
                && (4..=7).contains(&self.eng.read_u32(gm + timer.state))
                && self.eng.read_u8(gm + timer.armed) != 0
            {
                if timer.freeze != 0 {
                    self.eng.write_mem(gm + timer.freeze, &[0; 4]);
                }
                if self.eng.read_u32(gm + timer.length) == 0 {
                    self.eng
                        .write_mem(gm + timer.length, &timer.second.to_le_bytes());
                }
                let now = self.eng.read_u32(gm + timer.now);
                self.eng.write_mem(gm + timer.end, &now.to_le_bytes());
                self.phase = None;
            }
        }
    }

    fn end_match(&mut self) {
        if !self.patched {
            return;
        }
        if let Some((game, round)) = self.round_profile() {
            if self.round_live(game, round, self.host_gm(game)) {
                self.end_match_patch(true);
                self.match_end = true;
                self.apply_match_end();
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

    fn round_profile(&self) -> Option<(&'static GameProfile, &'static RoundProfile)> {
        game_profile(self.build).map(|game| (game, &game.round))
    }

    fn classic_timer(&self) -> Option<&'static ClassicTimer> {
        CLASSIC_TIMERS
            .iter()
            .find(|timer| timer.build == self.build)
    }

    fn classic_gm(&self, timer: &ClassicTimer) -> u64 {
        let gm = self.eng.read_ptr(self.eng.at(timer.global));
        if gm != 0 && self.eng.read_u32(gm + timer.role) <= 1 {
            gm
        } else {
            0
        }
    }

    fn gm(&self, game: &GameProfile) -> u64 {
        let at = self.eng.at(game.game_manager);
        match game.dec_gm {
            Some(decode) => decode(self.eng.read_u64(at)),
            None => self.eng.read_ptr(at),
        }
    }

    fn host_gm(&self, game: &GameProfile) -> u64 {
        let gm = self.gm(game);
        if gm != 0 && self.eng.read_u32(gm + game.role) <= 1 {
            gm
        } else {
            0
        }
    }

    fn game_state(&self, game: &GameProfile, gm: u64) -> u32 {
        let raw = self.eng.read_u32(gm + game.state);
        game.dec_state.map_or(raw, |decode| decode(raw))
    }

    fn has_flag(&self, gm: u64, (offset, value): (u64, Raw<u8>)) -> bool {
        self.eng.read_u8(gm + offset) == value.get()
    }

    fn round_live(&self, game: &GameProfile, round: &RoundProfile, gm: u64) -> bool {
        gm != 0
            && self.game_state(game, gm) == 7
            && self.eng.read_u32(gm + round.live.0) == round.live.1.get()
    }

    fn apply_match_end(&mut self) {
        let Some((game, round)) = self.round_profile() else {
            return;
        };
        let gm = self.host_gm(game);
        if !self.round_live(game, round, gm) || self.has_flag(gm, round.match_over) {
            self.end_match_patch(false);
            self.match_end = false;
        } else if self.has_flag(gm, round.running) {
            self.time_out(round, gm);
        }
    }

    fn time_out(&self, round: &RoundProfile, gm: u64) {
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

    fn apply_infinite(&mut self) {
        if let Some(timer) = self.classic_timer() {
            self.apply_classic_infinite(timer);
            return;
        }
        let Some((game, round)) = self.round_profile() else {
            return;
        };
        let gm = self.host_gm(game);
        if gm == 0 {
            return;
        }
        let infinite = (round.enc_duration)(0);
        let start = self.eng.read_u32(gm + round.start);
        let duration = self.eng.read_u32(gm + round.duration);
        let now = self.eng.read_u32(gm + round.now);
        if !self.infinite_time {
            if let Some(phase) = self.phase.take() {
                if phase.start == start && duration == infinite {
                    let end = (round.dec_end)(phase.end).wrapping_add(now.wrapping_sub(phase.at));
                    self.eng
                        .write_mem(gm + round.end, &(round.enc_end)(end).to_le_bytes());
                    self.eng
                        .write_mem(gm + round.duration, &phase.duration.to_le_bytes());
                    self.set_infinite_display(round, gm, false);
                }
            }
            return;
        }
        let state = self.game_state(game, gm);
        if (state == 5 || state == 7)
            && self.has_flag(gm, round.running)
            && self.has_flag(gm, round.countdown)
            && duration != infinite
            && (round.dec_duration)(duration) >= MIN_PHASE_MS
        {
            self.phase = Some(Phase {
                start,
                duration,
                end: self.eng.read_u32(gm + round.end),
                at: now,
            });
            self.eng
                .write_mem(gm + round.duration, &infinite.to_le_bytes());
            self.eng
                .write_mem(gm + round.end, &(round.enc_end)(now).to_le_bytes());
            self.set_infinite_display(round, gm, true);
        }
    }

    fn apply_classic_infinite(&mut self, timer: &ClassicTimer) {
        let gm = self.classic_gm(timer);
        if gm == 0 {
            return;
        }
        let start = self.eng.read_u32(gm + timer.start);
        let length = self.eng.read_u32(gm + timer.length);
        let now = self.eng.read_u32(gm + timer.now);
        if let Some(phase) = self.phase.take() {
            if phase.start == start && length == 0 {
                let end = now.wrapping_add(phase.end.wrapping_sub(phase.at));
                self.eng.write_mem(gm + timer.end, &end.to_le_bytes());
                if self.infinite_time {
                    self.phase = Some(phase);
                } else {
                    self.eng
                        .write_mem(gm + timer.length, &phase.duration.to_le_bytes());
                }
                return;
            }
        }
        if self.infinite_time
            && (4..=7).contains(&self.eng.read_u32(gm + timer.state))
            && self.eng.read_u8(gm + timer.armed) != 0
            && self.eng.read_u8(gm + timer.countup) == 0
            && (timer.freeze == 0 || self.eng.read_u32(gm + timer.freeze) == 0)
            && length >= MIN_PHASE_MS / 1000 * timer.second
        {
            self.phase = Some(Phase {
                start,
                duration: length,
                end: self.eng.read_u32(gm + timer.end),
                at: now,
            });
            self.eng.write_mem(gm + timer.length, &[0; 4]);
        }
    }

    fn set_infinite_display(&self, round: &RoundProfile, gm: u64, on: bool) {
        if let Some((offset, flags)) = round.infinite_display {
            let [enabled, disabled] = flags.get();
            self.eng
                .write_mem(gm + offset, &[if on { enabled } else { disabled }]);
        }
    }

    fn scan_handles(&mut self, game: &GameProfile) {
        let modes: Vec<u64> = MULTIPLAYER_MODES
            .iter()
            .chain(TERRORIST_HUNT_MODES)
            .map(|m| m.1)
            .chain([GYM_MODE])
            .chain(game.events.iter().chain(game.arcade).map(|e| e.1))
            .collect();
        let playlists: Vec<u64> = game.events.iter().chain(game.arcade).map(|e| e.2).collect();
        let objects: HashMap<u64, u64> = playlists
            .iter()
            .map(|&uid| (game.enc_object_uid.map_or(uid, |enc| enc(uid)), uid))
            .collect();
        let stored: HashMap<u64, u64> = game
            .maps
            .iter()
            .chain(DIFFICULTIES)
            .map(|t| t.1)
            .chain(modes.iter().chain(&playlists).copied())
            .map(|uid| (game.enc_handle_uid.map_or(uid, |enc| enc(uid)), uid))
            .collect();
        let mut targets: Vec<u64> = stored.keys().chain(objects.keys()).copied().collect();
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
        let gamemode = self.eng.at(game.gamemode);
        self.handles.clear();
        self.playlists.clear();
        for at in hits {
            let raw = self.eng.read_u64(at);
            let obj = at - 0x10;
            let head = self.eng.read_u64(obj);
            if image.contains(&head) {
                if let Some(&uid) = objects.get(&raw) {
                    self.playlists.entry(uid).or_insert(obj);
                }
            } else if let Some(&uid) = stored.get(&raw) {
                if obj % 0x20 == 0
                    && self.eng.read_u64(obj + 0x18) == 0
                    && self.eng.read_u64(obj + 8) >> 32 != 0
                    && (head == 0 || (0x10000..0x7FF0_0000_0000).contains(&head))
                    && (!modes.contains(&uid) || self.eng.read_u64(head) == gamemode)
                {
                    self.handles.entry(uid).or_insert(obj);
                }
            }
        }
    }

    fn handle(&self, game: &GameProfile, uid: u64) -> u64 {
        match self.handles.get(&uid) {
            Some(&handle)
                if self.eng.read_u64(handle + 0x10)
                    == game.enc_handle_uid.map_or(uid, |enc| enc(uid)) =>
            {
                handle
            }
            _ => 0,
        }
    }

    fn read_array(&self, at: u64) -> Vec<u64> {
        self.array_entries(self.eng.read_u64(at), self.eng.read_u64(at + 8))
    }

    fn array_entries(&self, ptr: u64, packed: u64) -> Vec<u64> {
        if ptr == 0 || packed as u32 & 0xC000_0000 != 0xC000_0000 {
            return Vec::new();
        }
        (0..(packed & 0x3FFF_FFFF).min(4096))
            .map(|i| self.eng.read_u64(ptr + i * 8))
            .collect()
    }

    fn settings_head(&self, at: u64, codec: Option<&MatchCodec>) -> (u64, u64) {
        let ptr = self.eng.read_u64(at);
        let packed = self.eng.read_u64(at + 8);
        match codec {
            Some(codec) => ((codec.settings[0])(ptr), (codec.settings[1])(packed)),
            None => (ptr, packed),
        }
    }

    fn match_tod(&self, game: &GameProfile, m: u64) -> u64 {
        let at = m + game.match_layout.time_of_day;
        match game.match_state.codec() {
            Some(codec) => (codec.dec_tod)(self.eng.read_u64(at)),
            None => self.eng.read_ptr(at),
        }
    }

    fn write_settings(&self, at: u64, entries: &[u64], codec: Option<&MatchCodec>) {
        let (ptr, packed) = self.settings_head(at, codec);
        let cap = packed >> 32;
        let count = entries.len() as u64;
        if ptr == 0 || packed as u32 & 0xC000_0000 != 0xC000_0000 || count > cap {
            return;
        }
        for (i, entry) in entries.iter().enumerate() {
            self.eng.write_mem(ptr + i as u64 * 8, &entry.to_le_bytes());
        }
        let head = cap << 32 | count | 0xC000_0000;
        let head = codec.map_or(head, |codec| (codec.enc_settings_count)(head));
        self.eng.write_mem(at + 8, &head.to_le_bytes());
        for i in count..(packed & 0x3FFF_FFFF).min(cap) {
            self.eng.write_mem(ptr + i * 8, &0u64.to_le_bytes());
        }
    }

    fn apply_playlist(&mut self) {
        let Some(pick) = self.pick else {
            return;
        };
        let Some(game) = game_profile(self.build) else {
            return;
        };
        let gm = self.host_gm(game);
        if gm == 0 || self.game_state(game, gm) != 3 {
            return;
        }
        let m = self.match_entry(game, gm);
        if m == 0 {
            return;
        }
        let ready = |runner: &Self| {
            [pick.mode, pick.map, pick.difficulty]
                .iter()
                .all(|&uid| uid == 0 || runner.handle(game, uid) != 0)
                && (pick.playlist == 0 || runner.playlists.contains_key(&pick.playlist))
        };
        if !ready(self) {
            if self.scanned_match == m {
                return;
            }
            self.scanned_match = m;
            self.scan_handles(game);
            if !ready(self) {
                return;
            }
        }
        let codec = game.match_state.codec();
        let encode = |handle: u64, field: fn(&MatchCodec) -> fn(u64) -> u64| {
            codec.map_or(handle, |codec| field(codec)(handle))
        };
        let layout = &game.match_layout;
        let mode = encode(self.handle(game, pick.mode), |codec| codec.enc_mode);
        let map = encode(self.handle(game, pick.map), |codec| codec.enc_map);
        self.eng.write_mem(m + layout.mode, &mode.to_le_bytes());
        self.eng.write_mem(m + layout.map, &map.to_le_bytes());
        if pick.difficulty != 0 {
            self.eng.write_mem(
                m + layout.difficulty,
                &self.handle(game, pick.difficulty).to_le_bytes(),
            );
        }
        let base = self.match_base(game, gm);
        if pick.playlist == 0 {
            self.restore_lobby(game, base);
        } else {
            self.apply_event(game, m, base, pick.playlist);
        }
    }

    fn apply_event(&mut self, game: &GameProfile, m: u64, base: u64, playlist: u64) {
        let codec = game.match_state.codec();
        let layout = &game.match_layout;
        let tod_at = self.match_tod(game, m);
        self.lobbies.retain(|lobby| lobby.base == base);
        let lobby = match self.lobbies.iter().position(|lobby| lobby.m == m) {
            Some(i) => self.lobbies.swap_remove(i),
            None => {
                let (ptr, packed) = self.settings_head(m + layout.settings, codec);
                Lobby {
                    m,
                    base,
                    playlist: self.eng.read_u64(m + layout.playlist),
                    tod_at,
                    tod: self.eng.read_u64(tod_at),
                    entries: self.array_entries(ptr, packed),
                }
            }
        };
        let obj = self.playlists[&playlist];
        let at = obj + game.settings;
        let mut settings = match game.dec_settings {
            Some([ptr, count]) => {
                self.array_entries(ptr(self.eng.read_u64(at)), count(self.eng.read_u64(at + 8)))
            }
            None => self.read_array(at),
        };
        if let Some(&last) = settings.last() {
            settings.resize(settings.len().max(lobby.entries.len()), last);
        }
        self.write_settings(
            m + layout.settings,
            if settings.is_empty() {
                &lobby.entries
            } else {
                &settings
            },
            codec,
        );
        if tod_at != 0 {
            let tod = self
                .read_array(obj + game.time_of_day)
                .first()
                .copied()
                .unwrap_or(lobby.tod);
            self.eng.write_mem(tod_at, &tod.to_le_bytes());
        }
        let playlist = codec.map_or(playlist, |codec| (codec.enc_playlist)(playlist));
        self.eng
            .write_mem(m + layout.playlist, &playlist.to_le_bytes());
        self.lobbies.push(lobby);
    }

    fn restore_lobby(&mut self, game: &GameProfile, base: u64) {
        let layout = &game.match_layout;
        for lobby in std::mem::take(&mut self.lobbies) {
            if lobby.base != base {
                continue;
            }
            self.eng
                .write_mem(lobby.m + layout.playlist, &lobby.playlist.to_le_bytes());
            if lobby.tod_at != 0 && self.match_tod(game, lobby.m) == lobby.tod_at {
                self.eng.write_mem(lobby.tod_at, &lobby.tod.to_le_bytes());
            }
            self.write_settings(
                lobby.m + layout.settings,
                &lobby.entries,
                game.match_state.codec(),
            );
        }
    }

    fn set_watermark(&self, enabled: bool) -> bool {
        let Some(&(_, flag, manager, layer, dirty)) =
            WATERMARKS.iter().find(|(build, ..)| *build == self.build)
        else {
            return self.set_watermark_overlay(enabled);
        };
        self.eng.write_mem(self.eng.at(flag), &[u8::from(enabled)]);
        let ui = self.eng.read_ptr(self.eng.at(manager));
        if ui != 0 {
            self.eng.write_mem(ui + layer, &[u8::from(enabled)]);
            self.eng.write_mem(ui + dirty, &[1]);
        }
        true
    }

    fn set_watermark_overlay(&self, enabled: bool) -> bool {
        let Some(&WatermarkOverlay {
            flag,
            flag_bytes,
            manager,
            counter,
            entries,
            slot,
            persistent,
            dirty,
            ..
        }) = WATERMARK_OVERLAYS
            .iter()
            .find(|overlay| overlay.build == self.build)
        else {
            return false;
        };
        let [on, off] = flag_bytes.get();
        self.eng
            .write_mem(self.eng.at(flag), &[if enabled { on } else { off }]);
        let ui = self.eng.read_ptr(self.eng.at(manager));
        let list = self.deref_chain(ui, &[entries, 0]);
        let data = self.deref_chain(list, &[0x28]);
        if data == 0 {
            return true;
        }
        let head = self.eng.read_u32(list + 0x30);
        let size = head & 0x3FFF_FFFF;
        let id = self.eng.read_u32(ui + slot);
        let at = |k: u32| data + u64::from(k) * 0x18;
        if enabled {
            let object = self.eng.read_ptr(ui + persistent);
            if id != 0 || object == 0 || size >= self.eng.read_u32(list + 0x34) & 0x3FFF_FFFF {
                return true;
            }
            let id = self.eng.read_u32(self.eng.at(counter)).wrapping_add(1);
            self.eng.write_mem(self.eng.at(counter), &id.to_le_bytes());
            self.eng.write_mem(at(size), &id.to_le_bytes());
            self.eng.write_mem(at(size) + 8, &object.to_le_bytes());
            self.eng.write_mem(at(size) + 0x10, &[0; 4]);
            let refs = self.eng.read_u32(object + 0xC).wrapping_add(1);
            self.eng.write_mem(object + 0xC, &refs.to_le_bytes());
            self.eng.write_mem(ui + slot, &id.to_le_bytes());
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
            self.eng.write_mem(ui + slot, &[0; 4]);
        }
        self.eng.write_mem(list + dirty, &[1]);
        true
    }

    fn write_state(&self, feature: &str) -> bool {
        let Some(&(_, _, global, off, value)) = STATE_WRITES
            .iter()
            .find(|(build, name, ..)| *build == self.build && *name == feature)
        else {
            return false;
        };
        let obj = self.eng.read_ptr(self.eng.at(global));
        if obj != 0 {
            self.eng
                .write_mem(obj.wrapping_add(off as i64 as u64), &[value]);
        }
        true
    }

    fn apply_weapons(&self) {
        let [primary, secondary, ..] = self.disabled;
        let empty = primary && secondary && self.season < SEASON_Y2S3;
        for (feature, on) in [
            ("SetEmptySecondary", empty),
            ("SetDisablePrimaryWeapon", primary),
            ("SetDisableSecondaryWeapon", secondary && !empty),
        ] {
            self.apply_static(feature, if on { "enable" } else { "disable" });
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
        if let Some(i) = DISABLE_MODS.iter().position(|n| *n == mod_) {
            self.disabled[i] = enabled;
            if self.round_profile().is_some() {
                self.apply_disable();
                return;
            }
        }
        match mod_ {
            "infiniteTime" => {
                self.infinite_time = enabled;
                self.apply_infinite();
            }
            "disablePrimary" | "disableSecondary" => self.apply_weapons(),
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
        if let Some(game) = game_profile(self.build) {
            if let Some(pick) = parse_pick(game, id_str) {
                self.pick = Some(pick);
                self.scanned_match = 0;
                self.apply_playlist();
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
            .is_none_or(|(_, addr)| self.eng.read_u8(self.eng.at(*addr)) != 0)
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
        if let Some(t) = rh.children.get_mut(2) {
            label_situation(t, tp.situ_adv);
            if self.season < SEASON_Y3S4 {
                t.children
                    .retain(|situation| self.briefing_installed(&situation.text));
            }
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
        group_event_nodes(&mut rh, tp.event_mode, |event| {
            self.classic_event_installed(event)
        });
        for (map, from) in [("Yacht", SEASON_Y1S1), ("Border", SEASON_Y1S2)] {
            if self.season < from {
                drop_map(&mut rh, map);
            }
        }
        tree_event(&mut rh.children)
    }

    fn set_old_hereford(&self, on: bool) {
        let value: i64 = if on { 708483531 } else { 127951053400 };
        self.write_chain("SetOldHereford", &value.to_le_bytes());
    }

    fn scan_features(&mut self) {
        let round = self.round_profile().is_some();
        let classic = self.classic_timer().is_some();
        for (on, (key, feature)) in self.available.iter_mut().zip(CAPS) {
            *on = find_chain(feature, self.build).is_some()
                || patches_have(feature, self.build)
                || shadow_toggles(self.build)
                    .iter()
                    .any(|(name, ..)| *name == key)
                || ((round || classic) && matches!(key, "infiniteTime" | "skip"))
                || (round && DISABLE_MODS.contains(&key))
                || (key == "displayBuild"
                    && (WATERMARKS.iter().any(|(build, ..)| *build == self.build)
                        || WATERMARK_OVERLAYS
                            .iter()
                            .any(|overlay| overlay.build == self.build)))
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
            .filter(|((key, _), _)| match *key {
                "disableAI" => self.season < SEASON_Y8S4,
                "displayBuild" => self.season >= SEASON_Y2S4,
                _ => true,
            })
            .map(|((key, _), on)| format!("\"{key}\":{},", full && on))
            .collect();
        let attached = !self.eng.proc.is_null();
        format!(
            "{{\"event\":\"state\",\"attached\":{},\"unlocked\":{},\"unsupported\":{},\"capabilities\":{{{caps}\"fullFeature\":{full}}}}}",
            attached,
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
            let Some(&(build, season)) = BUILDS
                .iter()
                .find(|(b, _)| b.strip_suffix(num).is_some_and(|p| p.ends_with('_')))
            else {
                self.unsupported = true;
                return;
            };
            let renderers = match self.eng.exe {
                "RainbowSix_Vulkan.exe" => Some(VULKAN_BUILDS),
                "RainbowSix_DX11.exe" => Some(DX11_BUILDS),
                _ => None,
            };
            if let Some(renderers) = renderers {
                let Some(renderer) = renderers.iter().find(|r| r.build == build) else {
                    self.unsupported = true;
                    return;
                };
                self.eng.renderer = Some(renderer);
            }
            self.build = build;
            self.season = season;
            self.scan_features();
        }
        if self.pending {
            let idle = match shadow_regions_for_build(self.build) {
                Some(regions) => {
                    let sync = playlist_sync(self.build).map(|site| ShadowRegion {
                        offset: site.hook,
                        patch: site.moved,
                    });
                    let gate = warmup_gate(self.build).map(|site| ShadowRegion {
                        offset: site.site,
                        patch: site.moved,
                    });
                    let regions: Vec<ShadowRegion> =
                        regions.iter().copied().chain(sync).chain(gate).collect();
                    self.eng.shadow_run(&regions)
                }
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
            if let Some(site) = playlist_sync(self.build).filter(|site| !site.late) {
                self.eng.arm_playlist_sync(site);
            }
            if let Some(site) = warmup_gate(self.build) {
                self.eng.arm_warmup_gate(site);
            }
            self.patched = true;
        }
        for (build, offset, value) in INIT_BYTES {
            if *build == self.build && self.eng.read_u8(self.eng.at(*offset)) != *value {
                self.eng.write_mem(self.eng.at(*offset), &[*value]);
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
            if let Some(site) = playlist_sync(self.build).filter(|site| site.late) {
                self.eng.arm_playlist_sync(site);
            }
        }
        self.apply_disable();
        self.apply_playlist();
        if self.infinite_time {
            self.apply_infinite();
        }
        if self.match_end {
            self.apply_match_end();
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
            Some("skip") => self.skip(),
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
        let mut poll = Instant::now();
        loop {
            match rx.recv_timeout(next.min(poll).saturating_duration_since(Instant::now())) {
                Ok(line) if !line.is_empty() => self.handle_command(&line),
                Ok(_) | Err(RecvTimeoutError::Timeout) => {}
                Err(RecvTimeoutError::Disconnected) => return 0,
            }
            let now = Instant::now();
            if now >= poll {
                poll = now + PLAYLIST_POLL;
                self.apply_gametype();
            }
            if now >= next {
                next = now + Duration::from_secs(1);
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

#[cfg(test)]
mod table_tests {
    use super::*;

    fn key_index(key: &str) -> i32 {
        let s = key.find('S').unwrap();
        let end = key.find('_').unwrap();
        let year: i32 = key[1..s].parse().unwrap();
        let season: i32 = key[s + 1..end].parse().unwrap();
        (year - 1) * 4 + season
    }

    #[test]
    fn builds_index_matches_key() {
        for &(key, index) in BUILDS {
            assert_eq!(key_index(key), index, "{key}");
        }
    }

    #[test]
    fn shadow_toggle_builds_are_shadow_seasons() {
        for &(build, _) in BUILDS {
            if !shadow_toggles(build).is_empty() {
                assert!(
                    SHADOW_SEASONS.iter().any(|s| s.build == build),
                    "{build} has toggles but no shadow season"
                );
            }
        }
    }

    #[test]
    fn shadow_toggles_restore_from_a_region() {
        for season in SHADOW_SEASONS {
            for &(name, offset, on) in shadow_toggles(season.build) {
                let region = season
                    .regions
                    .iter()
                    .find(|r| r.offset == offset)
                    .unwrap_or_else(|| {
                        panic!(
                            "{} {name} at {offset:#x} has no shadow region",
                            season.build
                        )
                    });
                assert_eq!(
                    region.patch.len(),
                    on.len(),
                    "{} {name} at {offset:#x}",
                    season.build
                );
            }
        }
    }

    #[test]
    fn renderer_patches_map_an_address() {
        for build in VULKAN_BUILDS.iter().chain(DX11_BUILDS) {
            for patch in build.patches {
                assert!(
                    build.rvas.iter().any(|&(dx, _)| dx == patch.rva),
                    "{} renderer patch {:#x} has no address pair",
                    build.build,
                    patch.rva
                );
            }
        }
    }

    #[test]
    fn warmup_gates_map_every_address() {
        for gate in WARMUP_GATES {
            let Some(build) = VULKAN_BUILDS.iter().find(|b| b.build == gate.build) else {
                continue;
            };
            for rva in [gate.site, gate.no, gate.back] {
                assert!(
                    build.rvas.iter().any(|&(dx, _)| dx == rva),
                    "{} warmup gate {rva:#x} has no address pair",
                    gate.build
                );
            }
        }
    }

    #[test]
    fn patch_branches_share_length() {
        for patch in PATCHES {
            if patch.branch != "enable" {
                continue;
            }
            if let Some(disable) = PATCHES.iter().find(|q| {
                q.feature == patch.feature
                    && q.build == patch.build
                    && q.addr == patch.addr
                    && q.branch == "disable"
            }) {
                assert_eq!(
                    patch.bytes.len(),
                    disable.bytes.len(),
                    "{} {} at {:#x}",
                    patch.build,
                    patch.feature,
                    patch.addr
                );
            }
        }
    }
}
