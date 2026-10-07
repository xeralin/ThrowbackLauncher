use core::ffi::c_void;
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
        let mut buf = vec![0u8; (CHUNK + OVERLAP) as usize];
        let mut off: u32 = 0;
        while off < self.modsize {
            let remain = self.modsize - off;
            let toread = remain.min(CHUNK + OVERLAP);
            let got = self.read_mem(self.base + off as u64, &mut buf[..toread as usize]);
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

static Y5S1_SUPPORTED_FEATURES: &[&str] = &[
    "SetDisplayBuild",
    "SetDeathless",
    "SetDisableAI",
    "SetUnlimitedAmmo",
    "SetUnlimitedEquipment",
    "SetDisablePrimaryWeapon",
    "SetDisableSecondaryWeapon",
    "SetDisablePrimaryGadget",
    "SetDisableSecondaryGadget",
];

fn patches_have(feature: &str, build: &str) -> bool {
    PATCHES
        .iter()
        .any(|p| p.feature == feature && p.build == build)
}

const TREE_NULL: &str = "{\"event\":\"tree\",\"tree\":null}";
const Y5_MATCH_ROOT: u64 = 0x53A6868;
const Y5_TEMPLATE_ROOT: u64 = 0x60894D8;
static Y5_DISABLE_SLOTS: [(&str, i32); 4] = [
    ("disablePrimary", 0x10),
    ("disableSecondary", 0x18),
    ("disablePrimaryGadget", 0x28),
    ("disableSecondaryGadget", 0x38),
];

static Y5_CHAIN_MAP: &[i32] = &[0xC18, 0x210, 0x10, 0x3C0, 0xA48, 0x0, 0xC18, 0xD78, 0xF30];
static Y5_CHAIN_GAMETYPE: &[i32] = &[0x20, 0x228, 0x28, 0x40, 0x50, 0x0, 0x160, 0x40, 0x28];
static Y5_CHAIN_DIFF: &[i32] = &[
    0x28, 0x0, 0x58, 0x90, 0x40, 0x10, 0xA0, 0x60, 0x40, 0x170, 0x290,
];
static Y5_CHAIN_HQ: &[i32] = &[
    0x4D8, 0x48, 0xD20, 0xD00, 0xFD8, 0x0, 0x1C0, 0x0, 0x20, 0xC8,
];
static Y5_CHAIN_HEREFORD: &[i32] = &[0x158, 0x80, 0x148, 0x18, 0x20, 0x130, 0x0, 0x18, 0xA18];
static Y5_CHAIN_GRAND_LARCENY: &[i32] = &[
    0x88, 0x110, 0x1F0, 0x560, 0x50, 0x50, 0x0, 0x30, 0x18, 0x20, 0x10, 0x1F0,
];
static Y5_CHAIN_GOLDEN_GUN: &[i32] = &[
    0x88, 0x110, 0x1F0, 0x560, 0x50, 0x50, 0x0, 0x30, 0x18, 0x20, 0x10, 0x1E0,
];

static Y5_MAP_NAMES: &[&str] = &[
    "House",
    "Oregon",
    "Hereford Base",
    "Hereford Base - Rework",
    "Club House",
    "Presidential Plane",
    "Yacht",
    "Consulate",
    "Bank",
    "Kanal",
    "Chalet",
    "Bartlett University",
    "Kafe Dostoyevsky",
    "Border",
    "Favela",
    "Skyscraper",
    "Coastline",
    "Theme Park",
    "Tower",
    "Villa",
    "Fortress",
    "Outback",
    "Headquarters",
];
static Y5_MAP_OFF: &[i32] = &[
    0x530, 0x538, -1, 0x540, 0x548, 0x550, 0x558, 0x560, 0x568, 0x570, 0x578, 0x580, 0x588, 0x590,
    0x598, 0x5A0, 0x5A8, 0x5B0, 0x5B8, 0x5C0, 0x5C8, 0x5D0, -1,
];
static Y5_GAMETYPE_NAMES: &[&str] = &[
    "Hostage",
    "Secure Area",
    "Bomb",
    "Protect Hostage",
    "Elimination",
    "Extract Hostage",
    "Disarm Bomb",
    "Warmup",
    "Gym Game",
    "Bomb - No Prep Phase",
];
static Y5_GAMETYPE_OFF: &[i32] = &[
    0x590, 0x5D0, 0x5E0, 0x5A8, 0x5B0, 0x5B8, 0x5F0, 0x618, 0x5C8, 0x640,
];
static Y5_GAMETYPE_STRING_ID: &[u32] = &[
    0xF2ED, 0xF2EC, 0xF2EF, 0x21B88, 0xF2EB, 0x21B87, 0x21B89, 0, 0, 0xF2EF,
];
static Y5_DIFF_NAMES: &[&str] = &["Normal", "Hard", "Realistic"];
static Y5_DIFF_OFF: &[i32] = &[0x160, 0x168, 0x170];

fn y5_append(chain: &[i32], tail: i32) -> Vec<i32> {
    let mut out = chain.to_vec();
    out.push(tail);
    out
}

fn y5_map_nodes(gametype: usize) -> Vec<TNode> {
    Y5_MAP_NAMES
        .iter()
        .enumerate()
        .map(|(m, name)| TNode {
            text: name.to_string(),
            id: format!("y5:sel:{gametype}:{m}"),
            children: Vec::new(),
        })
        .collect()
}

pub struct Runner {
    eng: Engine,
    client: TcpStream,
    build: &'static str,
    patched: bool,
    pending: bool,
    countdown: i32,
    applied: bool,
    unsupported: bool,
    season: i32,
    disable_primary: bool,
    disable_secondary: bool,
    y5_disable: [bool; 4],
    lastsig: String,
    y5_map: i32,
    y5_gametype: i32,
    y5_difficulty: i32,
    y5_event: i32,
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
            applied: false,
            unsupported: false,
            season: -1,
            disable_primary: false,
            disable_secondary: false,
            y5_disable: [false; 4],
            lastsig: String::new(),
            y5_map: -1,
            y5_gametype: -1,
            y5_difficulty: -1,
            y5_event: 0,
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
        (0..=SEASON_Y5S1).contains(&self.season)
    }

    fn y5_shadow_toggle(&self, mod_: &str, enabled: bool) -> bool {
        let table: &[(&str, u64, &[u8])] = &[
            (
                "deathless",
                0x1842673,
                &[
                    0xC7, 0x87, 0x68, 0x01, 0x00, 0x00, 0x78, 0x00, 0x00, 0x00, 0x90, 0x90, 0x90,
                    0x90, 0x90,
                ],
            ),
            (
                "unlimitedEquip",
                0x12984AB,
                &[
                    0xC7, 0x41, 0x54, 0x06, 0x00, 0x00, 0x00, 0x48, 0x8B, 0x49, 0x18, 0x90, 0x90,
                    0x90, 0x90, 0xEB,
                ],
            ),
            (
                "unlimitedAmmo",
                0x1AB36DB,
                &[
                    0x44, 0x3B, 0x40, 0x7C, 0x90, 0x48, 0x89, 0x7C, 0x24, 0x50, 0x7E, 0x05, 0x44,
                    0x89, 0x40, 0x7C, 0x90,
                ],
            ),
            ("disableAI", 0x1081B80, &[0xC3]),
            ("displayBuild", 0x7386F8, &[0x90, 0x90]),
            ("displayBuild", 0x104E6C, &[0x00]),
        ];
        let mut matched = false;
        for (name, off, on) in table {
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

    fn y5_resolve(&self, static_offset: u64, chain: &[i32]) -> u64 {
        let Some((last, path)) = chain.split_last() else {
            return 0;
        };
        match self.y5_deref(self.eng.read_ptr(self.eng.base + static_offset), path) {
            0 => 0,
            a => a.wrapping_add(*last as i64 as u64),
        }
    }

    fn y5_gametype_entry(&self, gametype: usize) -> u64 {
        let template = self.y5_resolve(
            Y5_TEMPLATE_ROOT,
            &y5_append(Y5_CHAIN_GAMETYPE, Y5_GAMETYPE_OFF[gametype]),
        );
        self.y5_deref(template, &[0, 0])
    }

    fn y5_gametype_group(&self, gametype: usize) -> String {
        match self.y5_deref(self.y5_gametype_entry(gametype), &[0x58, 0, 0x20]) {
            0 => String::new(),
            name => self.eng.read_ascii(name, 64),
        }
    }

    fn y5_gametype_matches(&self, gametype: usize) -> bool {
        let entry = self.y5_gametype_entry(gametype);
        if entry == 0 {
            return false;
        }
        let mut raw = [0u8; 4];
        self.eng.read_mem(entry + 0x28, &mut raw);
        u32::from_le_bytes(raw) == Y5_GAMETYPE_STRING_ID[gametype]
    }

    fn y5_copy(&self, field: u64, template: u64) {
        if field == 0 || template == 0 {
            return;
        }
        let value = self.eng.read_ptr(template);
        if value != 0 {
            self.eng.write_mem(field, &value.to_le_bytes());
        }
    }

    fn build_y5s1_tree_json(&self) -> String {
        let mut pvp: Vec<usize> = Vec::new();
        let mut pve: Vec<usize> = Vec::new();
        let mut gym: Vec<usize> = Vec::new();
        for i in 0..Y5_GAMETYPE_NAMES.len() {
            match self.y5_gametype_group(i).as_str() {
                "MatchFlowPVP" => pvp.push(i),
                "MatchFlowPVEAttack" | "MatchFlowPVEDefend" => pve.push(i),
                "GymGameplay" => gym.push(i),
                _ => {}
            }
        }
        if pvp.is_empty() && pve.is_empty() && gym.is_empty() {
            return TREE_NULL.to_string();
        }
        pvp.sort_by_key(|i| Y5_GAMETYPE_NAMES[*i]);
        pve.sort_by_key(|i| Y5_GAMETYPE_NAMES[*i]);
        let mut roots: Vec<TNode> = Vec::new();
        if !pvp.is_empty() {
            let mut multiplayer = TNode {
                text: "Multiplayer".to_string(),
                ..Default::default()
            };
            for gt in &pvp {
                let mut gametype = TNode {
                    text: Y5_GAMETYPE_NAMES[*gt].to_string(),
                    ..Default::default()
                };
                gametype.children = y5_map_nodes(*gt);
                multiplayer.children.push(gametype);
            }
            roots.push(multiplayer);
        }
        if !pve.is_empty() {
            let mut hunt = TNode {
                text: "Terrorist Hunt".to_string(),
                ..Default::default()
            };
            for gt in &pve {
                let mut gametype = TNode {
                    text: Y5_GAMETYPE_NAMES[*gt].to_string(),
                    ..Default::default()
                };
                for (m, map_name) in Y5_MAP_NAMES.iter().enumerate() {
                    let mut map = TNode {
                        text: map_name.to_string(),
                        ..Default::default()
                    };
                    for (d, diff_name) in Y5_DIFF_NAMES.iter().enumerate() {
                        map.children.push(TNode {
                            text: diff_name.to_string(),
                            id: format!("y5:sel:{gt}:{m}:{d}"),
                            children: Vec::new(),
                        });
                    }
                    gametype.children.push(map);
                }
                hunt.children.push(gametype);
            }
            roots.push(hunt);
        }
        if !gym.is_empty() {
            let mut development = TNode {
                text: "Development".to_string(),
                ..Default::default()
            };
            development.children = y5_map_nodes(gym[0]);
            roots.push(development);
        }
        let mut events = TNode {
            text: "Events".to_string(),
            ..Default::default()
        };
        events.children.push(TNode {
            text: "Grand Larceny".to_string(),
            id: "y5:evt:1".to_string(),
            children: Vec::new(),
        });
        events.children.push(TNode {
            text: "Golden Gun".to_string(),
            id: "y5:evt:2".to_string(),
            children: Vec::new(),
        });
        roots.push(events);
        format!("{{\"event\":\"tree\",\"tree\":[{}]}}", tn_list(&roots))
    }

    fn tree_json(&self) -> String {
        if self.season == SEASON_Y5S1 {
            self.build_y5s1_tree_json()
        } else {
            self.build_tree_json()
        }
    }

    fn apply_y5_playlist(&self) {
        if self.season != SEASON_Y5S1 {
            return;
        }
        if self.y5_event == 0 && self.y5_map < 0 && self.y5_gametype < 0 {
            return;
        }
        let map_field = self.y5_resolve(Y5_MATCH_ROOT, &[0x90, 0x10]);
        if map_field == 0 {
            return;
        }
        let keepalive = self.y5_resolve(Y5_MATCH_ROOT, &[0x8B8]);
        if keepalive != 0 {
            self.eng.write_mem(keepalive, &[0u8]);
        }
        let gametype_field = self.y5_resolve(Y5_MATCH_ROOT, &[0x90, 0x8]);
        if self.y5_event == 1 {
            self.y5_copy(map_field, self.y5_resolve(Y5_MATCH_ROOT, Y5_CHAIN_HQ));
            self.y5_copy(
                gametype_field,
                self.y5_resolve(Y5_TEMPLATE_ROOT, Y5_CHAIN_GRAND_LARCENY),
            );
            return;
        }
        if self.y5_event == 2 {
            self.y5_copy(
                map_field,
                self.y5_resolve(Y5_MATCH_ROOT, &y5_append(Y5_CHAIN_MAP, 0x538)),
            );
            self.y5_copy(
                gametype_field,
                self.y5_resolve(Y5_TEMPLATE_ROOT, Y5_CHAIN_GOLDEN_GUN),
            );
            return;
        }
        if self.y5_map >= 0 && (self.y5_map as usize) < Y5_MAP_NAMES.len() {
            let map = self.y5_map as usize;
            let chain = if map == 2 {
                Y5_CHAIN_HEREFORD.to_vec()
            } else if map == 22 {
                Y5_CHAIN_HQ.to_vec()
            } else {
                y5_append(Y5_CHAIN_MAP, Y5_MAP_OFF[map])
            };
            self.y5_copy(map_field, self.y5_resolve(Y5_MATCH_ROOT, &chain));
        }
        if self.y5_gametype >= 0 && (self.y5_gametype as usize) < Y5_GAMETYPE_NAMES.len() {
            let gametype = self.y5_gametype as usize;
            if self.y5_gametype_matches(gametype) {
                self.y5_copy(
                    gametype_field,
                    self.y5_resolve(
                        Y5_TEMPLATE_ROOT,
                        &y5_append(Y5_CHAIN_GAMETYPE, Y5_GAMETYPE_OFF[gametype]),
                    ),
                );
            }
        }
        if self.y5_difficulty >= 0
            && (self.y5_difficulty as usize) < Y5_DIFF_NAMES.len()
            && (3..=6).contains(&self.y5_gametype)
        {
            let difficulty = self.y5_difficulty as usize;
            self.y5_copy(
                self.y5_resolve(Y5_MATCH_ROOT, &[0x90, 0x30]),
                self.y5_resolve(
                    Y5_TEMPLATE_ROOT,
                    &y5_append(Y5_CHAIN_DIFF, Y5_DIFF_OFF[difficulty]),
                ),
            );
        }
    }

    fn apply_y5_selection(&mut self, name: &str) {
        let parts: Vec<&str> = name.split(':').collect();
        if parts.len() < 3 {
            return;
        }
        if parts[1] == "evt" {
            self.y5_event = parts[2].parse().unwrap_or(0);
        } else if parts[1] == "sel" && parts.len() >= 4 {
            self.y5_gametype = parts[2].parse().unwrap_or(-1);
            self.y5_map = parts[3].parse().unwrap_or(-1);
            self.y5_difficulty = parts.get(4).map_or(-1, |d| d.parse().unwrap_or(-1));
            self.y5_event = 0;
        }
        self.apply_y5_playlist();
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

    fn write_chain(&self, feature: &str, bytes: &[u8]) {
        if let Some(fc) = find_chain(feature, self.build) {
            let a = self.eng.resolve_ptr_chain(fc.base_offset, fc.offsets);
            self.eng.write_mem(a, bytes);
        }
    }

    fn end_round(&self) {
        if self.patched && !self.write_state("EndRound") {
            self.write_chain("EndRound", &[1]);
        }
    }

    fn end_match(&self) {
        if self.patched && !self.write_state("EndMatch") {
            self.write_chain("EndMatch", &[1]);
            self.write_chain("EndMatchTrigger", &[1]);
        }
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

    fn apply_y5_disable(&self) {
        if self.season != SEASON_Y5S1 || !self.y5_disable.iter().any(|on| *on) {
            return;
        }
        let roster = self.y5_deref(self.eng.base + Y5_MATCH_ROOT, &[0x00, 0xC8]);
        if roster == 0 {
            return;
        }
        for player in 0..10 {
            let container = self.y5_deref(roster, &[player * 8, 0xC8, 0x98, 0x00, 0x40]);
            if container == 0 {
                continue;
            }
            for (i, (_, slot)) in Y5_DISABLE_SLOTS.iter().enumerate() {
                if !self.y5_disable[i] {
                    continue;
                }
                let entry = self.y5_deref(container, &[*slot, 0x20, 0x00]);
                if entry != 0 {
                    self.eng.write_mem(entry + 0x18, &0u64.to_le_bytes());
                }
            }
        }
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
        if self.season == SEASON_Y5S1 {
            if self.y5_shadow_toggle(mod_, enabled) {
                return;
            }
            if let Some(i) = Y5_DISABLE_SLOTS.iter().position(|(n, _)| *n == mod_) {
                self.y5_disable[i] = enabled;
                self.apply_y5_disable();
                return;
            }
        }
        match mod_ {
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
        if id_str.starts_with("y5:") {
            self.apply_y5_selection(id_str);
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
        self.write_chain(
            "SetGametype",
            &raw.parse::<i64>().unwrap_or(0).to_le_bytes(),
        );
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
        format!(
            "{{\"event\":\"tree\",\"tree\":[{}]}}",
            tn_list(&rh.children)
        )
    }

    fn set_old_hereford(&self, on: bool) {
        let value: i64 = if on { 708483531 } else { 127951053400 };
        self.write_chain("SetOldHereford", &value.to_le_bytes());
    }

    fn scan_features(&mut self) {
        for (on, (_, feature)) in self.available.iter_mut().zip(CAPS) {
            *on = find_chain(feature, self.build).is_some()
                || patches_have(feature, self.build)
                || (self.season == SEASON_Y5S1 && Y5S1_SUPPORTED_FEATURES.contains(&feature))
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
            "{{\"event\":\"state\",\"attached\":{},\"applied\":{},\"unsupported\":{},\"capabilities\":{{{caps}\"fullFeature\":{full}}}}}",
            !self.eng.proc.is_null(),
            self.applied,
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
            self.applied = false;
            return;
        }
        self.apply_y5_playlist();
        self.apply_y5_disable();
        if !self.tree_sent && self.is_full_feature() {
            let tj = self.tree_json();
            if tj != TREE_NULL {
                self.tree_sent = true;
                self.send_line(&tj);
            }
        }
        self.applied = true;
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
