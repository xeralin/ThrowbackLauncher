fn match_core(s: &[u8], i: usize) -> Option<usize> {
    let n = s.len();
    let mut p = i;
    if p >= n || s[p] != b'C' {
        return None;
    }
    p += 1;
    let mut d0 = p;
    while p < n && s[p].is_ascii_digit() {
        p += 1;
    }
    if p == d0 || p >= n || s[p] != b'_' {
        return None;
    }
    p += 1;
    if p >= n || s[p] != b'D' {
        return None;
    }
    p += 1;
    d0 = p;
    while p < n && s[p].is_ascii_digit() {
        p += 1;
    }
    if p == d0 || p >= n || s[p] != b'_' {
        return None;
    }
    p += 1;
    if p >= n || s[p] != b'S' {
        return None;
    }
    p += 1;
    d0 = p;
    while p < n && s[p].is_ascii_digit() {
        p += 1;
    }
    if p == d0 || p >= n || s[p] != b'_' {
        return None;
    }
    p += 1;
    let mut cnt = 0;
    while p < n && s[p].is_ascii_digit() && cnt < 8 {
        p += 1;
        cnt += 1;
    }
    if cnt < 7 {
        return None;
    }
    Some(p)
}

fn match_yprefix(s: &[u8], i: usize) -> Option<usize> {
    let n = s.len();
    let mut p = i;
    if p >= n || s[p] != b'Y' {
        return None;
    }
    p += 1;
    if p >= n || !s[p].is_ascii_digit() {
        return None;
    }
    p += 1;
    if p >= n || s[p] != b'S' {
        return None;
    }
    p += 1;
    if p >= n || !s[p].is_ascii_digit() {
        return None;
    }
    p += 1;
    while p < n && (s[p].is_ascii_digit() || s[p] == b'.') {
        p += 1;
    }
    if p >= n || s[p] != b'_' {
        return None;
    }
    p += 1;
    Some(p)
}

pub fn scan_build(s: &[u8]) -> Option<String> {
    let n = s.len();
    for i in 0..n {
        if let Some(yend) = match_yprefix(s, i) {
            if let Some(end) = match_core(s, yend) {
                return Some(String::from_utf8_lossy(&s[i..end]).into_owned());
            }
        }
        if let Some(end) = match_core(s, i) {
            return Some(String::from_utf8_lossy(&s[i..end]).into_owned());
        }
    }
    None
}
