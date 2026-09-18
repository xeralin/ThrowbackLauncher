const F_MODRM: u8 = 0x01;
const F_IMM8: u8 = 0x02;
const F_IMM16: u8 = 0x04;
const F_IMM32: u8 = 0x08;
const F_IMMZ: u8 = 0x10;
const F_IMM64: u8 = 0x20;
const F_REL8: u8 = 0x40;
const F_REL32: u8 = 0x80;

#[rustfmt::skip]
static T1: [u8; 256] = [
    1,1,1,1,2,16,0,0,1,1,1,1,2,16,0,0,
    1,1,1,1,2,16,0,0,1,1,1,1,2,16,0,0,
    1,1,1,1,2,16,0,0,1,1,1,1,2,16,0,0,
    1,1,1,1,2,16,0,0,1,1,1,1,2,16,0,0,
    0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,
    0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,
    0,0,0,1,0,0,0,0,16,17,2,3,0,0,0,0,
    64,64,64,64,64,64,64,64,64,64,64,64,64,64,64,64,
    3,17,3,3,1,1,1,1,1,1,1,1,1,1,1,1,
    0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,
    32,32,32,32,0,0,0,0,2,16,0,0,0,0,0,0,
    2,2,2,2,2,2,2,2,16,16,16,16,16,16,16,16,
    3,3,4,0,0,0,3,17,6,0,4,0,0,2,0,0,
    1,1,1,1,2,2,0,0,1,1,1,1,1,1,1,1,
    64,64,64,64,2,2,2,2,128,128,0,64,0,0,0,0,
    0,0,0,0,0,0,1,1,0,0,0,0,0,0,1,1,
];

#[rustfmt::skip]
static T2: [u8; 256] = [
    1,1,1,1,0,0,0,0,0,0,0,0,0,1,0,0,
    1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,
    1,1,1,1,0,0,0,0,1,1,1,1,1,1,1,1,
    0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,
    1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,
    1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,
    1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,
    3,3,3,3,1,1,1,0,1,1,0,0,1,1,1,1,
    128,128,128,128,128,128,128,128,128,128,128,128,128,128,128,128,
    1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,
    0,0,0,1,3,1,0,0,0,0,0,1,3,1,1,1,
    1,1,1,1,1,1,1,1,1,1,3,1,1,1,1,1,
    1,1,3,1,3,3,3,1,0,0,0,0,0,0,0,0,
    1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,
    1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,
    1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,0,
];

static X87M3: [u64; 8] = [
    u64::MAX,
    0xFFFF7F330001FFFF,
    0x00000200FFFFFFFF,
    0x00FFFF1FFFFFFFFF,
    0xFFFFFFFF0000FFFF,
    0x0000FFFFFFFF00FF,
    0xFFFFFFFF0200FFFF,
    0x00FFFF0100000000,
];

fn read_i32(b: &[u8], i: usize) -> i32 {
    i32::from_le_bytes([b[i], b[i + 1], b[i + 2], b[i + 3]])
}

fn decode_modrm(buf: &[u8], pos: &mut usize, len: usize, rip_disp: &mut Option<usize>) -> bool {
    if *pos >= len {
        return false;
    }
    let modrm = buf[*pos];
    *pos += 1;
    let mod_ = (modrm >> 6) & 3;
    let rm = modrm & 7;
    if mod_ == 3 {
        return true;
    }
    let disp_size: usize;
    let mut rip_disp_pos: usize = 0;
    let mut rip_rel = false;
    if mod_ == 0 && rm == 5 {
        rip_rel = true;
        disp_size = 4;
        rip_disp_pos = *pos;
    } else {
        let has_sib = rm == 4;
        let mut sib_base: i32 = -1;
        if has_sib {
            if *pos >= len {
                return false;
            }
            sib_base = (buf[*pos] & 7) as i32;
            *pos += 1;
        }
        if mod_ == 0 {
            disp_size = if has_sib && sib_base == 5 { 4 } else { 0 };
        } else if mod_ == 1 {
            disp_size = 1;
        } else {
            disp_size = 4;
        }
    }
    if rip_rel && rip_disp_pos + 4 <= len {
        *rip_disp = Some(rip_disp_pos);
    }
    *pos += disp_size;
    if *pos > len {
        return false;
    }
    true
}

fn push_rip_fix(
    buf: &[u8],
    buf_base: u64,
    mod_lo: u64,
    mod_hi: u64,
    instr_end: usize,
    rip_disp: Option<usize>,
    fix: &mut Vec<usize>,
) {
    let Some(rip_disp_pos) = rip_disp else {
        return;
    };
    let disp32 = read_i32(buf, rip_disp_pos);
    let tgt = buf_base
        .wrapping_add(instr_end as u64)
        .wrapping_add(disp32 as i64 as u64);
    if tgt >= mod_lo && tgt < mod_hi {
        fix.push(rip_disp_pos);
    }
}

pub fn apply_rip_delta(buf: &mut [u8], fix: &[usize], delta: i64) {
    for &fi in fix {
        let v = read_i32(buf, fi);
        let nv = ((v as i64) - delta) as i32;
        buf[fi..fi + 4].copy_from_slice(&nv.to_le_bytes());
    }
}

pub fn scan_rip_rel(buf: &[u8], buf_base: u64, mod_lo: u64, mod_hi: u64) -> Vec<usize> {
    let len = buf.len();
    let mut fix: Vec<usize> = Vec::new();
    let mut pos = 0usize;
    while pos < len {
        let start = pos;
        let mut rip_disp: Option<usize> = None;
        let mut pfx66 = false;
        let mut pfx67 = false;
        let mut rex: u8 = 0;
        let mut more = true;
        while more && pos < len {
            let b = buf[pos];
            match b {
                0x66 => {
                    pfx66 = true;
                    pos += 1;
                }
                0x67 => {
                    pfx67 = true;
                    pos += 1;
                }
                0xF0 | 0xF2 | 0xF3 | 0x2E | 0x36 | 0x3E | 0x26 | 0x64 | 0x65 => {
                    pos += 1;
                }
                _ => {
                    more = false;
                }
            }
        }
        if pos >= len {
            break;
        }
        let lead = buf[pos];
        if lead == 0xC5 || lead == 0xC4 || lead == 0x62 {
            let map: i32;
            if lead == 0xC5 {
                if pos + 1 >= len {
                    break;
                }
                map = 1;
                pos += 2;
            } else if lead == 0xC4 {
                if pos + 2 >= len {
                    break;
                }
                map = (buf[pos + 1] & 0x1F) as i32;
                if !(1..=3).contains(&map) {
                    pos = start + 1;
                    continue;
                }
                pos += 3;
            } else {
                if pos + 3 >= len {
                    break;
                }
                map = (buf[pos + 1] & 0x03) as i32;
                if !(1..=3).contains(&map)
                    || (buf[pos + 1] & 0x0C) != 0
                    || (buf[pos + 2] & 0x04) == 0
                {
                    pos = start + 1;
                    continue;
                }
                pos += 4;
            }
            if pos >= len {
                break;
            }
            let vop = buf[pos];
            pos += 1;
            let no_modrm = map == 1 && vop == 0x77;
            let has_imm8 = (map == 3)
                || (map == 1
                    && (vop == 0x70
                        || vop == 0x71
                        || vop == 0x72
                        || vop == 0x73
                        || vop == 0xC2
                        || vop == 0xC4
                        || vop == 0xC5
                        || vop == 0xC6));
            if !no_modrm {
                if !decode_modrm(buf, &mut pos, len, &mut rip_disp) {
                    pos = start + 1;
                    continue;
                }
                if has_imm8 {
                    pos += 1;
                }
            }
            push_rip_fix(buf, buf_base, mod_lo, mod_hi, pos, rip_disp, &mut fix);
            if pos > len {
                break;
            }
            continue;
        }
        if (buf[pos] & 0xF0) == 0x40 {
            rex = buf[pos];
            pos += 1;
        }
        if pos >= len {
            break;
        }
        let rexw = (rex & 0x08) != 0;
        let op_len: usize;
        let op0 = buf[pos];
        let flags: u8;
        if op0 == 0x0F {
            if pos + 1 >= len {
                break;
            }
            let op1 = buf[pos + 1];
            if op1 == 0x38 || op1 == 0x3A {
                if pos + 2 >= len {
                    break;
                }
                op_len = 3;
                flags = if op1 == 0x3A {
                    F_MODRM | F_IMM8
                } else {
                    F_MODRM
                };
            } else if op1 == 0x0F {
                op_len = 2;
                flags = F_MODRM | F_IMM8;
            } else {
                op_len = 2;
                flags = T2[op1 as usize];
            }
        } else {
            op_len = 1;
            flags = T1[op0 as usize];
        }
        pos += op_len;
        if pos > len {
            break;
        }
        let is_rel8 = (flags & F_REL8) != 0;
        let is_rel32 = (flags & F_REL32) != 0;
        let mut modrm_reg = 0u8;
        if (flags & F_MODRM) != 0 {
            if pos < len {
                modrm_reg = (buf[pos] >> 3) & 7;
            }
            if op_len == 1 && pos < len {
                let rreg = modrm_reg;
                let rmod = (buf[pos] >> 6) & 3;
                let mut invalid = false;
                if op0 == 0xFE {
                    invalid = rreg > 1;
                } else if op0 == 0xFF {
                    invalid = (rreg == 7) || ((rreg == 3 || rreg == 5) && rmod == 3);
                } else if op0 == 0x8F {
                    invalid = rreg != 0;
                } else if op0 == 0xC6 || op0 == 0xC7 {
                    invalid = !(rreg == 0 || (rreg == 7 && rmod == 3));
                } else if (0xD8..=0xDF).contains(&op0) && rmod == 3 {
                    let idx = (buf[pos] - 0xC0) as u32;
                    if (X87M3[(op0 - 0xD8) as usize] & (1u64 << idx)) == 0 {
                        invalid = true;
                    }
                }
                if invalid {
                    pos = start + 1;
                    continue;
                }
            }
            if !decode_modrm(buf, &mut pos, len, &mut rip_disp) {
                pos = start + 1;
                continue;
            }
        }
        let mut imm_size: usize = 0;
        if op_len == 1 && (op0 == 0xF6 || op0 == 0xF7) {
            if modrm_reg == 0 || modrm_reg == 1 {
                imm_size = if op0 == 0xF6 {
                    1
                } else if pfx66 {
                    2
                } else {
                    4
                };
            }
        } else {
            if flags & F_IMM8 != 0 {
                imm_size += 1;
            }
            if flags & F_IMM16 != 0 {
                imm_size += 2;
            }
            if flags & F_IMM32 != 0 {
                imm_size += 4;
            }
            if flags & F_IMMZ != 0 {
                imm_size += if pfx66 { 2 } else { 4 };
            }
            if flags & F_IMM64 != 0 {
                if (0xA0..=0xA3).contains(&op0) {
                    imm_size += if pfx67 { 4 } else { 8 };
                } else {
                    imm_size += 8;
                }
            }
            if op_len == 1 && (0xB8..=0xBF).contains(&op0) && rexw {
                imm_size = 8;
            }
        }
        if is_rel32 {
            imm_size = 4;
        } else if is_rel8 {
            imm_size = 1;
        }
        pos += imm_size;
        if is_rel32 && pos <= len {
            let rel_pos = pos - 4;
            let tgt = buf_base
                .wrapping_add(pos as u64)
                .wrapping_add(read_i32(buf, rel_pos) as i64 as u64);
            if tgt < buf_base || tgt >= buf_base.wrapping_add(len as u64) {
                fix.push(rel_pos);
            }
        }
        push_rip_fix(buf, buf_base, mod_lo, mod_hi, pos, rip_disp, &mut fix);
        if pos > len {
            break;
        }
    }
    fix
}
