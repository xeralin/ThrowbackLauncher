use core::iter::from_fn;

pub const RX_FRAME_MAX: usize = 1600;
const ETH_FRAME_MIN: usize = 60;

pub fn classify_high_priority(frame: &[u8]) -> bool {
    if frame.len() < 14 {
        return false;
    }
    let unicast = frame[0] & 1 == 0;
    match u16::from_be_bytes([frame[12], frame[13]]) {
        0x0806 => true,
        0x0800 if frame.len() >= 34 => match frame[23] {
            1 | 6 => true,
            17 => unicast,
            _ => false,
        },
        0x86DD if frame.len() >= 54 => match frame[20] {
            6 | 58 => true,
            17 => unicast,
            _ => false,
        },
        _ => false,
    }
}

pub fn tlv_encode(out: &mut [u8], frame: &[u8], mode: u32) -> usize {
    let padded = frame.len().max(ETH_FRAME_MIN);
    let header = if mode >= 2 { 8 } else { 4 };
    let needed = header + padded;
    if needed > out.len() {
        return 0;
    }
    if mode >= 2 {
        let prefix = match frame.first_chunk::<4>() {
            Some(&[a, b, c, d]) => [d, c, b, a],
            None => [0; 4],
        };
        out[..4].copy_from_slice(&prefix);
    }
    out[header - 4..header].copy_from_slice(&(padded as u32).to_le_bytes());
    let body = &mut out[header..needed];
    body[..frame.len()].copy_from_slice(frame);
    body[frame.len()..].fill(0);
    needed
}

pub fn tlv_frames(mut input: &[u8], mode: u32) -> impl Iterator<Item = &[u8]> {
    let skip = if mode >= 2 { 4 } else { 0 };
    from_fn(move || {
        let (len, rest) = input.get(skip..)?.split_first_chunk::<4>()?;
        let len = u32::from_le_bytes(*len) as usize;
        if len == 0 || len > RX_FRAME_MAX || len > rest.len() {
            return None;
        }
        let (frame, rest) = rest.split_at(len);
        input = rest;
        Some(frame)
    })
}
