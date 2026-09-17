use core::ffi::c_void;
use std::mem::{size_of, zeroed};

use windows_sys::Win32::Foundation::{CloseHandle, HANDLE, INVALID_HANDLE_VALUE};
use windows_sys::Win32::Networking::WinSock::{
    closesocket, connect, htonl, htons, recv, select, send, socket, WSAStartup, FD_SET,
    INVALID_SOCKET, IN_ADDR, IN_ADDR_0, SOCKADDR, SOCKADDR_IN, SOCKET, TIMEVAL, WSADATA,
};
use windows_sys::Win32::System::Diagnostics::Debug::{ReadProcessMemory, WriteProcessMemory};
use windows_sys::Win32::System::Diagnostics::ToolHelp::{
    CreateToolhelp32Snapshot, Module32First, Process32First, Process32Next, MODULEENTRY32,
    PROCESSENTRY32, TH32CS_SNAPMODULE, TH32CS_SNAPMODULE32, TH32CS_SNAPPROCESS,
};
use windows_sys::Win32::System::LibraryLoader::{GetModuleHandleA, GetProcAddress};
use windows_sys::Win32::System::Threading::{
    GetExitCodeProcess, OpenProcess, PROCESS_CREATE_THREAD, PROCESS_QUERY_INFORMATION,
    PROCESS_VM_OPERATION, PROCESS_VM_READ, PROCESS_VM_WRITE,
};

use crate::mapbuild::canonical_build_name;
use crate::ownership::{ownership_recipe, Ownership};
use crate::shadow::shadow_regions_for_build;
use crate::tables::*;
use crate::tree::*;

fn cstr_eq_ignore_case(raw: &[i8], name: &str) -> bool {
    let nb = name.as_bytes();
    let mut i = 0;
    while i < raw.len() && raw[i] != 0 {
        if i >= nb.len() {
            return false;
        }
        if !(raw[i] as u8).eq_ignore_ascii_case(&nb[i]) {
            return false;
        }
        i += 1;
    }
    i == nb.len()
}

fn find_pid_by_name(name: &str) -> u32 {
    unsafe {
        let snap = CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0);
        if snap == INVALID_HANDLE_VALUE {
            return 0;
        }
        let mut pe: PROCESSENTRY32 = zeroed();
        pe.dwSize = size_of::<PROCESSENTRY32>() as u32;
        let mut pid = 0u32;
        if Process32First(snap, &mut pe) != 0 {
            loop {
                if cstr_eq_ignore_case(&pe.szExeFile, name) {
                    pid = pe.th32ProcessID;
                    break;
                }
                if Process32Next(snap, &mut pe) == 0 {
                    break;
                }
            }
        }
        CloseHandle(snap);
        pid
    }
}

fn module_info(pid: u32) -> Option<(u64, u32)> {
    unsafe {
        let snap = CreateToolhelp32Snapshot(TH32CS_SNAPMODULE | TH32CS_SNAPMODULE32, pid);
        if snap == INVALID_HANDLE_VALUE {
            return None;
        }
        let mut me: MODULEENTRY32 = zeroed();
        me.dwSize = size_of::<MODULEENTRY32>() as u32;
        let mut out = None;
        if Module32First(snap, &mut me) != 0 {
            out = Some((me.modBaseAddr as u64, me.modBaseSize));
        }
        CloseHandle(snap);
        out
    }
}

pub struct Engine {
    pub proc: HANDLE,
    pub base: u64,
    pub modsize: u32,
    pub exe: &'static str,
    pub(crate) shadow_pages: Vec<u64>,
    pub(crate) shadow_delta: i64,
}

impl Engine {
    pub fn new() -> Self {
        Engine {
            proc: core::ptr::null_mut(),
            base: 0,
            modsize: 0,
            exe: "",
            shadow_pages: Vec::new(),
            shadow_delta: 0,
        }
    }

    pub fn attach(&mut self) -> bool {
        for name in [
            "RainbowSix.exe",
            "RainbowSix_DX11.exe",
            "RainbowSixGame.exe",
        ] {
            let pid = find_pid_by_name(name);
            if pid == 0 {
                continue;
            }
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
                continue;
            }
            match module_info(pid) {
                Some((base, size)) => {
                    self.proc = h;
                    self.base = base;
                    self.modsize = size;
                    self.exe = name;
                    return true;
                }
                None => {
                    unsafe { CloseHandle(h) };
                }
            }
        }
        false
    }

    pub fn reader(&self) -> impl FnMut(u64, &mut [u8]) + '_ {
        let h = self.proc;
        move |a: u64, b: &mut [u8]| read_zero(h, a, b)
    }

    pub fn process_alive(&self) -> bool {
        let mut code: u32 = 0;
        let ok = unsafe { GetExitCodeProcess(self.proc, &mut code) };
        ok != 0 && code == STILL_ACTIVE
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
            if got > 0 {
                if let Some(m) = crate::buildscan::scan_build(&buf[..got]) {
                    return Some(m);
                }
            }
            off += CHUNK;
        }
        None
    }

    pub fn find_chain(&self, feature: &str, build: &str) -> Option<&'static FeatureChain> {
        FEATURE_CHAINS
            .iter()
            .find(|fc| fc.feature == feature && fc.build == build)
    }
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
        let mut rd = self.reader();
        let mut node = TNode::new();
        let num = if direct {
            address
        } else {
            let first = crate::memread::mr_u64(&mut rd, address);
            crate::memread::mr_u64(&mut rd, first)
        };
        let name_addr = crate::memread::mr_u64(&mut rd, num + name_off as u64);
        let name_bytes = crate::memread::mr_read_ascii(&mut rd, name_addr, 100);
        let name = String::from_utf8_lossy(&name_bytes);
        node.text = if name.is_empty() {
            "<No Name>".to_string()
        } else {
            name.into_owned()
        };
        node.id = format!("{}", crate::memread::mr_read_pointer(&mut rd, address));
        if depth >= 8 {
            return node;
        }
        let cnt = crate::memread::mr_read_u16(&mut rd, num + children_off as u64 + 8);
        let cnt = if cnt > 64 { 0 } else { cnt };
        let kids_base = crate::memread::mr_u64(&mut rd, num + children_off as u64);
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
    if let Some(c) = get_path(mp, extract).cloned() {
        mp.children.push(c);
    }
    rm(mp, rm_path, rm_idx);
    set_text(mp, &[5], label);
}

pub(crate) fn read_zero(h: HANDLE, addr: u64, buf: &mut [u8]) {
    buf.fill(0);
    let mut got: usize = 0;
    unsafe {
        ReadProcessMemory(
            h,
            addr as usize as *const c_void,
            buf.as_mut_ptr() as *mut c_void,
            buf.len(),
            &mut got,
        );
    }
}

impl Engine {
    fn read1(&self, addr: u64) -> i32 {
        let mut b = [0u8; 1];
        self.read_mem(addr, &mut b);
        b[0] as i32
    }

    fn write_int64(&self, addr: u64, val: i64) {
        self.write_mem(addr, &val.to_le_bytes());
    }

    fn resolve_ptr_chain(&self, base_offset: u64, offsets: &[u64]) -> u64 {
        if offsets.is_empty() {
            return self.base + base_offset;
        }
        let mut rd = self.reader();
        let mut num = self.base + base_offset;
        let mut ptr = crate::memread::mr_u64(&mut rd, num);
        let n = offsets.len();
        for off in &offsets[..n - 1] {
            num = ptr.wrapping_add(*off);
            ptr = crate::memread::mr_u64(&mut rd, num);
        }
        ptr.wrapping_add(offsets[n - 1])
    }
}

fn json_find(line: &str, key: &str) -> Option<usize> {
    let tok = format!("\"{}\"", key);
    let p = line.find(&tok)?;
    let b = line.as_bytes();
    let mut i = p + tok.len();
    while i < b.len() && (b[i] == b' ' || b[i] == b'\t') {
        i += 1;
    }
    if i >= b.len() || b[i] != b':' {
        return None;
    }
    i += 1;
    while i < b.len() && (b[i] == b' ' || b[i] == b'\t') {
        i += 1;
    }
    Some(i)
}

fn json_str(line: &str, key: &str) -> Option<String> {
    let i = json_find(line, key)?;
    let b = line.as_bytes();
    if i >= b.len() || b[i] != b'"' {
        return None;
    }
    let mut j = i + 1;
    let mut out = String::new();
    while j < b.len() && b[j] != b'"' {
        out.push(b[j] as char);
        j += 1;
    }
    Some(out)
}

fn json_bool(line: &str, key: &str) -> bool {
    json_find(line, key).is_some_and(|i| line[i..].starts_with("true"))
}

const STILL_ACTIVE: u32 = 259;

const CAP_COUNT: usize = 12;

static CAP_FEATURES: [&str; CAP_COUNT] = [
    "SetDeathless",
    "SetDisableAI",
    "SetUnlimitedAmmo",
    "SetUnlimitedEquipment",
    "SetInfiniteTime",
    "SetDisablePrimaryWeapon",
    "SetDisableSecondaryWeapon",
    "SetDisablePrimaryGadget",
    "SetDisableSecondaryGadget",
    "SetDisplayBuild",
    "EndRound",
    "EndMatch",
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
const Y5_STRING_ID_OFFSET: u64 = 0x28;
const Y5_PLAYER_SLOTS: i32 = 10;
static Y5_DISABLE_SLOTS: [(&str, i32); 4] = [
    ("disablePrimary", 0x10),
    ("disableSecondary", 0x18),
    ("disablePrimaryGadget", 0x28),
    ("disableSecondaryGadget", 0x38),
];
const Y5_GROUP_SLOT: u64 = 0x58;
const Y5_NAME_SLOT: u64 = 0x20;
const Y5_GROUP_PVP: &str = "MatchFlowPVP";
const Y5_GROUP_PVE_ATTACK: &str = "MatchFlowPVEAttack";
const Y5_GROUP_PVE_DEFEND: &str = "MatchFlowPVEDefend";
const Y5_GROUP_GYM: &str = "GymGameplay";

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

fn y5_has_difficulty(gametype: usize) -> bool {
    (3..=6).contains(&gametype)
}

fn y5_append(chain: &[i32], tail: i32) -> Vec<i32> {
    let mut out = chain.to_vec();
    out.push(tail);
    out
}

pub struct Runner {
    eng: Engine,
    client: SOCKET,
    build: String,
    attached: bool,
    applied: bool,
    pending: bool,
    countdown: i32,
    shadow_ready: bool,
    status: String,
    season: i32,
    disable_primary: bool,
    disable_secondary: bool,
    y5_disable: [bool; 4],
    force: bool,
    lastsig: String,
    y5_map: i32,
    y5_gametype: i32,
    y5_difficulty: i32,
    y5_event: i32,
    tree_sent: bool,
    scanned: String,
    available: [bool; CAP_COUNT],
    ownership: Ownership,
}

fn bs_season_of(build: &str) -> i32 {
    for (b, s) in BUILD_SEASONS {
        if *b == build {
            return *s;
        }
    }
    -1
}

impl Runner {
    fn new() -> Runner {
        Runner {
            eng: Engine::new(),
            client: INVALID_SOCKET,
            build: String::new(),
            attached: false,
            applied: false,
            pending: true,
            countdown: 0,
            shadow_ready: false,
            status: "Waiting for R6S to launch".to_string(),
            season: -1,
            disable_primary: false,
            disable_secondary: false,
            y5_disable: [false; 4],
            force: false,
            lastsig: String::new(),
            y5_map: -1,
            y5_gametype: -1,
            y5_difficulty: -1,
            y5_event: 0,
            tree_sent: false,
            scanned: String::new(),
            available: [false; CAP_COUNT],
            ownership: Ownership::default(),
        }
    }

    fn send_line(&self, s: &str) {
        if self.client == INVALID_SOCKET {
            return;
        }
        unsafe {
            send(self.client, s.as_ptr(), s.len() as i32, 0);
            send(self.client, b"\n".as_ptr(), 1, 0);
        }
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
        self.season >= 0 && (self.season <= SEASON_Y4S4 || self.season == SEASON_Y5S1)
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
            } else if let Some(regions) = shadow_regions_for_build(&self.build) {
                if let Some(r) = regions.iter().find(|r| r.offset == *off) {
                    self.eng.shadow_write(*off, r.patch);
                }
            }
        }
        matched
    }

    fn y5_resolve(&self, static_offset: u64, chain: &[i32]) -> u64 {
        if chain.is_empty() {
            return 0;
        }
        let mut rd = self.eng.reader();
        let mut a = crate::memread::mr_read_pointer(&mut rd, self.eng.base + static_offset);
        if a == 0 {
            return 0;
        }
        for off in &chain[..chain.len() - 1] {
            a = crate::memread::mr_read_pointer(&mut rd, a.wrapping_add(*off as i64 as u64));
            if a == 0 {
                return 0;
            }
        }
        a.wrapping_add(chain[chain.len() - 1] as i64 as u64)
    }

    fn y5_gametype_entry(&self, gametype: usize) -> u64 {
        let template = self.y5_resolve(
            Y5_TEMPLATE_ROOT,
            &y5_append(Y5_CHAIN_GAMETYPE, Y5_GAMETYPE_OFF[gametype]),
        );
        if template == 0 {
            return 0;
        }
        let mut rd = self.eng.reader();
        let descriptor = crate::memread::mr_read_pointer(&mut rd, template);
        if descriptor == 0 {
            return 0;
        }
        crate::memread::mr_read_pointer(&mut rd, descriptor)
    }

    fn y5_gametype_group(&self, gametype: usize) -> String {
        let entry = self.y5_gametype_entry(gametype);
        if entry == 0 {
            return String::new();
        }
        let mut rd = self.eng.reader();
        let handle = crate::memread::mr_read_pointer(&mut rd, entry + Y5_GROUP_SLOT);
        if handle == 0 {
            return String::new();
        }
        let group = crate::memread::mr_read_pointer(&mut rd, handle);
        if group == 0 {
            return String::new();
        }
        let name = crate::memread::mr_read_pointer(&mut rd, group + Y5_NAME_SLOT);
        if name == 0 {
            return String::new();
        }
        let bytes = crate::memread::mr_read_ascii(&mut rd, name, 64);
        String::from_utf8_lossy(&bytes).into_owned()
    }

    fn y5_gametype_matches(&self, gametype: usize) -> bool {
        let entry = self.y5_gametype_entry(gametype);
        if entry == 0 {
            return false;
        }
        let mut raw = [0u8; 4];
        self.eng.read_mem(entry + Y5_STRING_ID_OFFSET, &mut raw);
        u32::from_le_bytes(raw) == Y5_GAMETYPE_STRING_ID[gametype]
    }

    fn y5_copy(&self, field: u64, template: u64) {
        if field == 0 || template == 0 {
            return;
        }
        let mut rd = self.eng.reader();
        let value = crate::memread::mr_read_pointer(&mut rd, template);
        if value != 0 {
            self.eng.write_int64(field, value as i64);
        }
    }

    fn y5_map_nodes(&self, gametype: usize) -> Vec<TNode> {
        let mut out = Vec::new();
        for (m, name) in Y5_MAP_NAMES.iter().enumerate() {
            out.push(TNode {
                text: name.to_string(),
                id: format!("y5:sel:{}:{}", gametype, m),
                children: Vec::new(),
            });
        }
        out
    }

    fn build_y5s1_tree_json(&self) -> String {
        let mut pvp: Vec<usize> = Vec::new();
        let mut pve: Vec<usize> = Vec::new();
        let mut gym: Vec<usize> = Vec::new();
        for i in 0..Y5_GAMETYPE_NAMES.len() {
            let group = self.y5_gametype_group(i);
            if group == Y5_GROUP_PVP {
                pvp.push(i);
            } else if group == Y5_GROUP_PVE_ATTACK || group == Y5_GROUP_PVE_DEFEND {
                pve.push(i);
            } else if group == Y5_GROUP_GYM {
                gym.push(i);
            }
        }
        if pvp.is_empty() && pve.is_empty() && gym.is_empty() {
            return TREE_NULL.to_string();
        }
        pvp.sort_by_key(|i| Y5_GAMETYPE_NAMES[*i]);
        pve.sort_by_key(|i| Y5_GAMETYPE_NAMES[*i]);
        let mut roots: Vec<TNode> = Vec::new();
        if !pvp.is_empty() {
            let mut multiplayer = TNode::new();
            multiplayer.text = "Multiplayer".to_string();
            for gt in &pvp {
                let mut gametype = TNode::new();
                gametype.text = Y5_GAMETYPE_NAMES[*gt].to_string();
                gametype.children = self.y5_map_nodes(*gt);
                multiplayer.children.push(gametype);
            }
            roots.push(multiplayer);
        }
        if !pve.is_empty() {
            let mut hunt = TNode::new();
            hunt.text = "Terrorist Hunt".to_string();
            for gt in &pve {
                let mut gametype = TNode::new();
                gametype.text = Y5_GAMETYPE_NAMES[*gt].to_string();
                for (m, map_name) in Y5_MAP_NAMES.iter().enumerate() {
                    let mut map = TNode::new();
                    map.text = map_name.to_string();
                    for (d, diff_name) in Y5_DIFF_NAMES.iter().enumerate() {
                        map.children.push(TNode {
                            text: diff_name.to_string(),
                            id: format!("y5:sel:{}:{}:{}", gt, m, d),
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
            let mut development = TNode::new();
            development.text = "Development".to_string();
            development.children = self.y5_map_nodes(gym[0]);
            roots.push(development);
        }
        let mut events = TNode::new();
        events.text = "Events".to_string();
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
            self.build_tree_json(&self.build)
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
            && self.y5_gametype >= 0
            && y5_has_difficulty(self.y5_gametype as usize)
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
            self.y5_difficulty = if parts.len() >= 5 {
                parts[4].parse().unwrap_or(-1)
            } else {
                -1
            };
            self.y5_event = 0;
        }
        self.apply_y5_playlist();
    }

    fn y5_deref(&self, start: u64, offsets: &[i32]) -> u64 {
        let mut rd = self.eng.reader();
        let mut a = start;
        for off in offsets {
            if a == 0 {
                return 0;
            }
            a = crate::memread::mr_read_pointer(&mut rd, a.wrapping_add(*off as i64 as u64));
        }
        a
    }

    fn end_ready(&self) -> bool {
        !self.eng.proc.is_null() && !self.build.is_empty() && self.applied
    }

    fn end_round(&mut self) {
        if !self.end_ready() {
            return;
        }
        if self.write_state("EndRound") {
            return;
        }
        if let Some(fc) = self.eng.find_chain("EndRound", &self.build) {
            let a = self.eng.resolve_ptr_chain(fc.base_offset, fc.offsets);
            self.eng.write_mem(a, &[1u8]);
        }
    }

    fn end_match(&mut self) {
        if !self.end_ready() {
            return;
        }
        if self.write_state("EndMatch") {
            return;
        }
        if let Some(fc) = self.eng.find_chain("EndMatch", &self.build) {
            let a = self.eng.resolve_ptr_chain(fc.base_offset, fc.offsets);
            self.eng.write_mem(a, &[1u8]);
        }
        if let Some(fc) = self.eng.find_chain("EndMatchTrigger", &self.build) {
            let a = self.eng.resolve_ptr_chain(fc.base_offset, fc.offsets);
            self.eng.write_mem(a, &[1u8]);
        }
    }

    fn write_state(&mut self, feature: &str) -> bool {
        for (build, name, global, off, value) in STATE_WRITES {
            if *build != self.build || *name != feature {
                continue;
            }
            let mut rd = self.eng.reader();
            let obj = crate::memread::mr_read_pointer(&mut rd, self.eng.base + global);
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
        for player in 0..Y5_PLAYER_SLOTS {
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
                    self.eng.write_int64(entry + 0x18, 0);
                }
            }
        }
    }

    fn apply_weapons(&self) {
        if self.disable_primary
            && self.disable_secondary
            && self.season >= 0
            && self.season < SEASON_Y2S3
        {
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
        let c3 = [0xC3u8];
        let tp = unsafe {
            GetProcAddress(
                GetModuleHandleA(b"kernel32.dll\0".as_ptr()),
                b"TerminateProcess\0".as_ptr(),
            )
        };
        let ntp = unsafe {
            GetProcAddress(
                GetModuleHandleA(b"ntdll.dll\0".as_ptr()),
                b"NtTerminateProcess\0".as_ptr(),
            )
        };
        if let Some(f) = tp {
            self.eng.write_mem(f as usize as u64, &c3);
        }
        if let Some(f) = ntp {
            self.eng.write_mem(f as usize as u64, &c3);
        }
    }

    fn set_mod(&mut self, mod_: &str, enabled: bool) {
        if !self.end_ready() {
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
        let table = [
            ("deathless", "SetDeathless"),
            ("disableAI", "SetDisableAI"),
            ("unlimitedAmmo", "SetUnlimitedAmmo"),
            ("unlimitedEquip", "SetUnlimitedEquipment"),
            ("disablePrimaryGadget", "SetDisablePrimaryGadget"),
            ("disableSecondaryGadget", "SetDisableSecondaryGadget"),
            ("displayBuild", "SetDisplayBuild"),
        ];
        for (name, feature) in table {
            if mod_ == name {
                self.apply_static(feature, if enabled { "enable" } else { "disable" });
                return;
            }
        }
        if mod_ == "infiniteTime" {
            if let Some(fc) = self.eng.find_chain("SetInfiniteTime", &self.build) {
                let num = self.eng.resolve_ptr_chain(fc.base_offset, fc.offsets);
                let v: u8 = if enabled { 0 } else { 1 };
                self.eng.write_mem(num, &[v]);
            }
            return;
        }

        if mod_ == "disablePrimary" {
            self.disable_primary = enabled;
            self.apply_weapons();
        } else if mod_ == "disableSecondary" {
            self.disable_secondary = enabled;
            self.apply_weapons();
        }
    }

    fn set_playlist(&mut self, id_str: &str) {
        if !self.end_ready() {
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
        if let Some(fc) = self.eng.find_chain("SetGametype", &self.build) {
            let id: i64 = raw.parse().unwrap_or(0);
            let num = self.eng.resolve_ptr_chain(fc.base_offset, fc.offsets);
            self.eng.write_mem(num, &id.to_le_bytes());
        }
    }

    fn is_idle(&self) -> bool {
        for (b, addr) in IDLE {
            if *b == self.build {
                let a = self.eng.base.wrapping_add(*addr);
                return self.eng.read1(a) != 0;
            }
        }
        self.season > SEASON_Y4S4
    }

    fn set_loading_status(&mut self) {
        if self.season >= 0 && (self.season as usize) < SEASON_NAMES.len() {
            self.status = format!("Loading {}", SEASON_NAMES[self.season as usize]);
        } else {
            self.status = "Loading".to_string();
        }
    }

    fn reset_attach_state(&mut self) {
        self.eng.shadow_pages.clear();
        self.eng.shadow_delta = 0;
        self.build.clear();
        self.scanned.clear();
        self.season = -1;
        self.applied = false;
        self.pending = true;
        self.countdown = 0;
        self.shadow_ready = false;
        self.y5_map = -1;
        self.y5_gametype = -1;
        self.y5_difficulty = -1;
        self.y5_event = 0;
        self.tree_sent = false;
        self.available = [false; CAP_COUNT];
        self.ownership = Ownership::default();
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
                let offsets = [0x20u64, 0x40, 0x298, 0x60];
                let chain = self.eng.resolve_ptr_chain(0x5E99BE0, &offsets);
                let idv = {
                    let mut rd = self.eng.reader();
                    crate::memread::mr_read_pointer(&mut rd, chain)
                };
                root.children[0].children.push(TNode {
                    text: "Doktor's Curse".to_string(),
                    id: format!("{}", idv),
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

    fn build_tree_json(&self, build: &str) -> String {
        let tp = match TREE_PARAMS.iter().find(|t| t.build == build) {
            Some(t) => t,
            None => return TREE_NULL.to_string(),
        };
        let root_addr = self.eng.resolve_ptr_chain(tp.root_base, tp.root_offs);
        if tp.root_count <= 0 || root_addr == 0 {
            return TREE_NULL.to_string();
        }
        let nroot = tp.root_count.min(64);
        let mut rh = TNode::new();
        {
            for i in 0..nroot {
                let mut rd = self.eng.reader();
                let a = crate::memread::mr_read_pointer(
                    &mut rd,
                    root_addr + (i as u64) * (tp.stride as u64),
                );
                let child = self
                    .eng
                    .build_nodes(a, true, tp.name_off, tp.children_off, 0);
                rh.children.push(child);
            }
        }
        self.apply_event_mode(&mut rh, tp.event_mode);
        let mut rem: Vec<i32> = tp.remove.iter().copied().take(8).collect();
        rem.sort_unstable_by(|a, b| b.cmp(a));
        for r in rem {
            if r >= 0 && (r as usize) < rh.children.len() {
                rh.children.remove(r as usize);
            }
        }
        if tp.i_mp >= 0 {
            if let Some(t) = rh.children.get_mut(tp.i_mp as usize) {
                label_multiplayer(t, self.season);
            }
        }
        if tp.i_th >= 0 {
            if let Some(t) = rh.children.get_mut(tp.i_th as usize) {
                label_terrorist_hunt(t, self.season);
            }
        }
        if tp.i_mm >= 0 {
            if let Some(t) = rh.children.get_mut(tp.i_mm as usize) {
                label_matchmaking(t);
            }
        }
        if tp.i_situ >= 0 {
            if let Some(t) = rh.children.get_mut(tp.i_situ as usize) {
                label_situation(t, tp.situ_adv);
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
        if self.build == "Y4S2_13147883" {
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
        if let Some(fc) = self.eng.find_chain("SetOldHereford", &self.build) {
            let a = self.eng.resolve_ptr_chain(fc.base_offset, fc.offsets);
            self.eng.write_int64(
                a,
                if on {
                    OLD_HEREFORD_VALUE_A
                } else {
                    OLD_HEREFORD_VALUE_B
                },
            );
        }
    }

    fn scan_features(&mut self) {
        for (i, feature) in CAP_FEATURES.iter().enumerate() {
            self.available[i] = self.eng.find_chain(feature, &self.build).is_some()
                || patches_have(feature, &self.build)
                || (self.season == SEASON_Y5S1 && Y5S1_SUPPORTED_FEATURES.contains(feature))
                || STATE_WRITES
                    .iter()
                    .any(|(b, f, ..)| *b == self.build && f == feature);
        }
    }

    fn build_state(&self) -> String {
        let detected = self.attached && !self.build.is_empty() && self.season >= 0;
        let full = detected && self.is_full_feature();
        let have = |feature: &str| {
            CAP_FEATURES
                .iter()
                .position(|f| *f == feature)
                .is_some_and(|i| self.available[i])
        };
        let deathless = full && have("SetDeathless");
        let disable_ai = full && have("SetDisableAI");
        let unlimited_ammo = full && have("SetUnlimitedAmmo");
        let unlimited_equip = full && have("SetUnlimitedEquipment");
        let infinite_time = full && have("SetInfiniteTime");
        let disable_primary = full && have("SetDisablePrimaryWeapon");
        let disable_secondary = full && have("SetDisableSecondaryWeapon");
        let disable_primary_gadget = full && have("SetDisablePrimaryGadget");
        let disable_secondary_gadget = full && have("SetDisableSecondaryGadget");
        let display_build = full && have("SetDisplayBuild");
        let end_round = full && have("EndRound");
        let end_match = full && have("EndMatch");
        let jb = |b: bool| if b { "true" } else { "false" };
        format!(
            "{{\"event\":\"state\",\"attached\":{},\"applied\":{},\"status\":\"{}\",\"capabilities\":{{\"deathless\":{},\"disableAI\":{},\"unlimitedAmmo\":{},\"unlimitedEquip\":{},\"infiniteTime\":{},\"disablePrimary\":{},\"disableSecondary\":{},\"disablePrimaryGadget\":{},\"disableSecondaryGadget\":{},\"displayBuild\":{},\"endRound\":{},\"endMatch\":{},\"fullFeature\":{}}}}}",
            jb(self.attached), jb(self.applied), self.status,
            jb(deathless), jb(disable_ai), jb(unlimited_ammo), jb(unlimited_equip),
            jb(infinite_time), jb(disable_primary), jb(disable_secondary),
            jb(disable_primary_gadget), jb(disable_secondary_gadget),
            jb(display_build),
            jb(end_round), jb(end_match), jb(full)
        )
    }

    fn push_state(&mut self) {
        let s = self.build_state();
        if !self.force && s == self.lastsig {
            return;
        }
        self.force = false;
        self.send_line(&s);
        self.lastsig = s;
    }

    fn tick(&mut self) {
        let was_attached = self.attached;
        if !self.eng.proc.is_null() && !self.eng.process_alive() {
            unsafe { CloseHandle(self.eng.proc) };
            self.eng.proc = core::ptr::null_mut();
            if was_attached {
                std::process::exit(0);
            }
        }
        if self.eng.proc.is_null() && !self.eng.attach() {
            self.attached = false;
            self.reset_attach_state();
            self.status = "Waiting for R6S to launch".to_string();
            return;
        }
        self.attached = true;
        if self.scanned.is_empty() {
            match self.eng.detect_build() {
                Some(s) => self.scanned = s,
                None => {
                    self.reset_attach_state();
                    self.status = "Loading".to_string();
                    return;
                }
            }
        }
        let b = canonical_build_name(&self.scanned);
        if b == "None" {
            self.status = "This game build is not supported".to_string();
            return;
        }
        if self.build != b {
            self.build = b.to_string();
            self.season = bs_season_of(&self.build);
            self.scan_features();
        }
        let shadow_enabled = shadow_regions_for_build(&self.build).is_some_and(|r| !r.is_empty());
        if shadow_enabled && !self.shadow_ready {
            self.shadow_ready = self.eng.shadow_run(&self.build) == 0;
        }
        if self.pending {
            self.set_loading_status();
            let idle = if shadow_enabled {
                self.shadow_ready
            } else {
                self.is_idle()
            };
            if !idle {
                return;
            }
            self.pending = false;
            self.countdown = 4;
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
        if !self.applied {
            self.disable_self_terminate();
            if shadow_enabled {
                self.eng.shadow_arm_pages();
            }
            self.apply_static("ApplyCorePatch", "always");
            for (build, offset, value) in INIT_BYTES {
                if *build == self.build {
                    self.eng.write_mem(self.eng.base + offset, &[*value]);
                }
            }
            self.applied = true;
        }
        if let Some(r) = ownership_recipe(&self.build, self.eng.exe) {
            self.ownership.step(&self.eng, r);
            if !self.ownership.done {
                self.set_loading_status();
                return;
            }
        }
        if self.season == SEASON_Y5S1 {
            self.apply_y5_playlist();
            self.apply_y5_disable();
        }
        if !self.tree_sent && self.is_full_feature() {
            let tj = self.tree_json();
            if tj != TREE_NULL {
                self.tree_sent = true;
                self.send_line(&tj);
            }
        }
        self.status = if self.is_full_feature() {
            "Idle".to_string()
        } else {
            "Unlock All has been applied".to_string()
        };
    }

    fn handle_command(&mut self, line: &str) {
        let cmd = match json_str(line, "cmd") {
            Some(c) => c,
            None => return,
        };
        match cmd.as_str() {
            "setMod" => {
                if let Some(m) = json_str(line, "mod") {
                    self.set_mod(&m, json_bool(line, "enabled"));
                }
            }
            "setPlaylist" => {
                if let Some(id) = json_str(line, "playlistId") {
                    self.set_playlist(&id);
                }
            }
            "endRound" => self.end_round(),
            "endMatch" => self.end_match(),
            _ => {}
        }
        self.force = true;
    }

    fn run(&mut self, port: u16) -> i32 {
        let mut wsadata: WSADATA = unsafe { zeroed() };
        if unsafe { WSAStartup(0x0202, &mut wsadata) } != 0 {
            return 1;
        }
        self.client = unsafe { socket(2, 1, 0) };
        if self.client == INVALID_SOCKET {
            return 1;
        }
        let mut addr: SOCKADDR_IN = unsafe { zeroed() };
        addr.sin_family = 2;
        addr.sin_addr = IN_ADDR {
            S_un: IN_ADDR_0 {
                S_addr: unsafe { htonl(0x7f000001) },
            },
        };
        addr.sin_port = unsafe { htons(port) };
        let joined = unsafe {
            connect(
                self.client,
                &addr as *const SOCKADDR_IN as *const SOCKADDR,
                size_of::<SOCKADDR_IN>() as i32,
            )
        };
        if joined != 0 {
            return 1;
        }
        let mut rbuf = vec![0u8; 8192];
        let mut rlen = 0usize;
        let mut last_tick = std::time::Instant::now() - std::time::Duration::from_secs(1);
        loop {
            let mut rf: FD_SET = unsafe { zeroed() };
            rf.fd_count = 1;
            rf.fd_array[0] = self.client;
            let tv = TIMEVAL {
                tv_sec: 0,
                tv_usec: 100000,
            };
            let sel = unsafe {
                select(
                    0,
                    &mut rf,
                    core::ptr::null_mut(),
                    core::ptr::null_mut(),
                    &tv,
                )
            };
            if sel > 0 {
                let cap = rbuf.len() - rlen;
                let n = unsafe { recv(self.client, rbuf[rlen..].as_mut_ptr(), cap as i32, 0) };
                if n <= 0 {
                    break;
                }
                rlen += n as usize;
                let mut start = 0;
                while let Some(rel) = rbuf[start..rlen].iter().position(|&c| c == b'\n') {
                    let nl = start + rel;
                    let line = String::from_utf8_lossy(&rbuf[start..nl]).into_owned();
                    if !line.is_empty() {
                        self.handle_command(&line);
                    }
                    start = nl + 1;
                }
                let leftover = rlen - start;
                rbuf.copy_within(start..rlen, 0);
                rlen = leftover;
            }
            if last_tick.elapsed() >= std::time::Duration::from_secs(1) {
                last_tick = std::time::Instant::now();
                self.tick();
            }
            self.push_state();
        }
        unsafe {
            closesocket(self.client);
        }
        0
    }
}

pub fn main_entry() -> i32 {
    let args: Vec<String> = std::env::args().collect();
    if args.len() > 2 && args[1] == "--runtime" {
        if let Ok(port) = args[2].parse::<u16>() {
            let mut r = Runner::new();
            return r.run(port);
        }
    }
    2
}
