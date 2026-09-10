use core::ffi::c_void;
use std::collections::HashSet;

use windows_sys::Win32::System::Memory::{
    VirtualAllocEx, VirtualQueryEx, MEMORY_BASIC_INFORMATION, MEM_COMMIT, MEM_RESERVE, PAGE_GUARD,
    PAGE_NOACCESS, PAGE_READWRITE,
};

use crate::tables::*;
use crate::win::Engine;

pub(crate) fn ownership_recipe(build: &str) -> Option<&'static OwnershipRecipe> {
    OWNERSHIP.iter().find(|r| r.build == build)
}

#[derive(Default)]
pub(crate) struct Ownership {
    obj: u64,
    count: u32,
    ticks: u32,
    pub(crate) done: bool,
}

const FLAG_MASK: u32 = 0xC000_0000;
const SLACK: usize = 1024;
const SEARCH_EVERY: u32 = 5;

impl Ownership {
    pub(crate) fn step(&mut self, eng: &Engine, r: &OwnershipRecipe) {
        if self.done {
            return;
        }
        if self.obj == 0 {
            if self.ticks % SEARCH_EVERY == 0 {
                self.obj = eng.find_list(r.list_id).unwrap_or(0);
            }
            self.ticks += 1;
            if self.obj == 0 {
                return;
            }
        }
        match eng.list_count(self.obj) {
            Some(c) if c == self.count => self.done = eng.fill_list(self.obj, r.kinds),
            Some(c) => self.count = c,
            None => self.obj = 0,
        }
    }
}

impl Engine {
    fn read_u64(&self, addr: u64) -> u64 {
        let mut b = [0u8; 8];
        if self.read_mem(addr, &mut b) == 8 {
            u64::from_le_bytes(b)
        } else {
            0
        }
    }

    fn query(&self, addr: u64) -> Option<MEMORY_BASIC_INFORMATION> {
        let mut mbi: MEMORY_BASIC_INFORMATION = unsafe { core::mem::zeroed() };
        let n = unsafe {
            VirtualQueryEx(
                self.proc,
                addr as usize as *const c_void,
                &mut mbi,
                core::mem::size_of::<MEMORY_BASIC_INFORMATION>(),
            )
        };
        if n == 0 {
            None
        } else {
            Some(mbi)
        }
    }

    fn for_each_region(&self, mut f: impl FnMut(u64, &[u8])) {
        let mut addr: u64 = 0;
        let mut buf = vec![0u8; 1 << 22];
        while let Some(m) = self.query(addr) {
            let base = m.BaseAddress as u64;
            let size = m.RegionSize as u64;
            let inside_module = base >= self.base && base < self.base + self.modsize as u64;
            let readable = m.State == MEM_COMMIT && m.Protect & (PAGE_NOACCESS | PAGE_GUARD) == 0;
            if readable && !inside_module {
                let mut off = 0u64;
                while off < size {
                    let want = ((size - off).min(buf.len() as u64)) as usize;
                    let got = self.read_mem(base + off, &mut buf[..want]);
                    if got < 32 {
                        break;
                    }
                    let usable = got - got % 8;
                    f(base + off, &buf[..usable]);
                    off += usable as u64;
                }
            }
            addr = base.wrapping_add(size);
            if addr <= base || addr >= 0x7FFF_FFFF_0000 {
                break;
            }
        }
    }

    fn find_list(&self, list_id: u64) -> Option<u64> {
        let mod_lo = self.base;
        let mod_hi = self.base + self.modsize as u64;
        let mut lists = Vec::new();
        self.for_each_region(|base, b| {
            for (i, c) in b.chunks_exact(8).enumerate().skip(2) {
                if u64::from_le_bytes(c.try_into().unwrap()) == list_id {
                    let vt = u64::from_le_bytes(b[i * 8 - 16..i * 8 - 8].try_into().unwrap());
                    if vt >= mod_lo && vt < mod_hi {
                        lists.push(base + (i * 8) as u64 - 16);
                    }
                }
            }
        });
        match lists.as_slice() {
            [o] => Some(*o),
            _ => None,
        }
    }

    fn list_count(&self, obj: u64) -> Option<u32> {
        let data = self.read_u64(obj + 0x18);
        let cf = self.read_u64(obj + 0x20);
        let count = (cf as u32) & !FLAG_MASK;
        let cap = (cf >> 32) as u32;
        if data == 0 || count == 0 || count > 1 << 20 || cap < count {
            return None;
        }
        Some(count)
    }

    fn kind_objects(&self, kinds: &[u64]) -> HashSet<u64> {
        let mod_lo = self.base;
        let mod_hi = self.base + self.modsize as u64;
        let vts: Vec<u64> = kinds.iter().map(|k| mod_lo + k).collect();
        let mut objects = HashSet::new();
        self.for_each_region(|base, b| {
            for (i, c) in b.chunks_exact(8).enumerate() {
                let v = u64::from_le_bytes(c.try_into().unwrap());
                if v >= mod_lo && v < mod_hi && vts.contains(&v) {
                    objects.insert(base + (i * 8) as u64);
                }
            }
        });
        objects
    }

    fn handles_of(&self, objects: &HashSet<u64>) -> Vec<u64> {
        let mut handles = Vec::new();
        self.for_each_region(|base, b| {
            let mut i = 0usize;
            while i + 32 <= b.len() {
                let p = u64::from_le_bytes(b[i..i + 8].try_into().unwrap());
                if p >= 0x10000
                    && p <= 0x7FFF_FFFF_FFFF
                    && b[i + 11] & 0x80 != 0
                    && b[i + 24..i + 32] == [0u8; 8]
                    && objects.contains(&p)
                {
                    handles.push(base + i as u64);
                }
                i += 8;
            }
        });
        handles
    }

    fn fill_list(&self, obj: u64, kinds: &[u64]) -> bool {
        let data = self.read_u64(obj + 0x18);
        let cf = self.read_u64(obj + 0x20);
        let count = (cf as u32) & !FLAG_MASK;
        let mut old = vec![0u8; count as usize * 8];
        if self.read_mem(data, &mut old) < old.len() {
            return false;
        }
        let mut have: Vec<u64> = old
            .chunks_exact(8)
            .map(|c| u64::from_le_bytes(c.try_into().unwrap()))
            .collect();
        have.sort_unstable();
        let objects = self.kind_objects(kinds);
        if objects.is_empty() {
            return false;
        }
        let add: Vec<u64> = self
            .handles_of(&objects)
            .into_iter()
            .filter(|h| have.binary_search(h).is_err())
            .collect();
        if add.is_empty() {
            return true;
        }
        let newcap = count as usize + add.len() + SLACK;
        let newbuf = unsafe {
            VirtualAllocEx(
                self.proc,
                core::ptr::null(),
                newcap * 8,
                MEM_COMMIT | MEM_RESERVE,
                PAGE_READWRITE,
            )
        } as u64;
        if newbuf == 0 {
            return false;
        }
        for h in &add {
            old.extend_from_slice(&h.to_le_bytes());
        }
        if !self.write_mem(newbuf, &old) || !self.write_mem(obj + 0x18, &newbuf.to_le_bytes()) {
            return false;
        }
        let newcf = ((count as usize + add.len()) as u32 | (cf as u32 & FLAG_MASK)) as u64
            | ((newcap as u64) << 32);
        self.write_mem(obj + 0x20, &newcf.to_le_bytes())
    }
}
