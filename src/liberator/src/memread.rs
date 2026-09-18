pub fn mr_read_pointer<R: FnMut(u64, &mut [u8])>(rd: &mut R, addr: u64) -> u64 {
    let mut b = [0u8; 8];
    rd(addr, &mut b[..6]);
    u64::from_le_bytes(b)
}

pub fn mr_read_u16<R: FnMut(u64, &mut [u8])>(rd: &mut R, addr: u64) -> u16 {
    let mut b = [0u8; 2];
    rd(addr, &mut b);
    u16::from_le_bytes(b)
}

pub fn mr_read_ascii<R: FnMut(u64, &mut [u8])>(rd: &mut R, addr: u64, len: usize) -> Vec<u8> {
    let mut buf = vec![0u8; len];
    rd(addr, &mut buf);
    let mut out: Vec<u8> = Vec::new();
    let mut flag = false;
    for &c in &buf {
        if flag && c == 0 {
            break;
        }
        if c == 0 {
            flag = true;
        } else {
            flag = false;
            out.push(c);
        }
    }
    out
}
