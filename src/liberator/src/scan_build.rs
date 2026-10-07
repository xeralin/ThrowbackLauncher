fn match_core(s: &[u8], mut p: usize) -> Option<usize> {
    for tag in *b"CDS" {
        if s.get(p) != Some(&tag) {
            return None;
        }
        let digits = s[p + 1..].iter().take_while(|c| c.is_ascii_digit()).count();
        if digits == 0 || s.get(p + 1 + digits) != Some(&b'_') {
            return None;
        }
        p += digits + 2;
    }
    let digits = s[p..]
        .iter()
        .take(8)
        .take_while(|c| c.is_ascii_digit())
        .count();
    (digits >= 7).then_some(p + digits)
}

pub fn scan_build(s: &[u8]) -> Option<String> {
    (0..s.len())
        .find_map(|i| match_core(s, i).map(|end| String::from_utf8_lossy(&s[i..end]).into_owned()))
}
