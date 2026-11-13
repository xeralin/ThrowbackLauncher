use core::ffi::c_void;

use crate::ffi::Irp;
use crate::frame::{RX_FRAME_MAX, tlv_encode};

const IRP_QUEUE_SIZE: usize = 1024;
const RX_RING_SIZE: usize = 16;

#[derive(Clone, Copy)]
pub struct Queued {
    pub irp: *mut Irp,
    pub file_object: *mut c_void,
}

pub struct IrpQueue {
    pub entries: [Queued; IRP_QUEUE_SIZE],
    pub len: usize,
}

impl IrpQueue {
    pub fn is_full(&self) -> bool {
        self.len == IRP_QUEUE_SIZE
    }

    pub fn push(&mut self, entry: Queued) {
        self.entries[self.len] = entry;
        self.len += 1;
    }

    pub fn take(&mut self, pick: impl FnMut(&Queued) -> bool) -> Option<*mut Irp> {
        let index = self.entries[..self.len].iter().position(pick)?;
        let irp = self.entries[index].irp;
        self.entries.copy_within(index + 1..self.len, index);
        self.len -= 1;
        Some(irp)
    }

    pub fn take_first_per_file(
        &mut self,
        picked: &mut Self,
        mut pick: impl FnMut(&Queued) -> bool,
    ) {
        picked.len = 0;
        let mut kept = 0;
        for index in 0..self.len {
            let entry = self.entries[index];
            let seen = picked.entries[..picked.len]
                .iter()
                .any(|first| first.file_object == entry.file_object);
            if !seen && pick(&entry) {
                picked.push(entry);
            } else {
                self.entries[kept] = entry;
                kept += 1;
            }
        }
        self.len = kept;
    }
}

struct RxFrame {
    len: u16,
    data: [u8; RX_FRAME_MAX],
}

pub struct RxRing {
    write_idx: usize,
    read_idx: usize,
    frames: [RxFrame; RX_RING_SIZE],
}

impl RxRing {
    pub fn has_frames(&self) -> bool {
        self.write_idx != self.read_idx
    }

    pub fn push(&mut self, frame: &[u8]) {
        if self.write_idx.wrapping_sub(self.read_idx) < RX_RING_SIZE {
            let slot = &mut self.frames[self.write_idx % RX_RING_SIZE];
            slot.data[..frame.len()].copy_from_slice(frame);
            slot.len = frame.len() as u16;
            self.write_idx = self.write_idx.wrapping_add(1);
        }
    }

    pub fn drain(&mut self, out: &mut [u8], mode: u32) -> usize {
        let mut written = 0;
        while self.has_frames() {
            let slot = &self.frames[self.read_idx % RX_RING_SIZE];
            let frame = &slot.data[..usize::from(slot.len)];
            let count = tlv_encode(&mut out[written..], frame, mode);
            if count == 0 {
                break;
            }
            written += count;
            self.read_idx = self.read_idx.wrapping_add(1);
        }
        written
    }
}
