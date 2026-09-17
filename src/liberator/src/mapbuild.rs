use crate::tables::BUILD_SEASONS;

fn ends_with_num(name: &str, num: &str) -> bool {
    let ln = name.len();
    let lnum = num.len();
    if ln < lnum + 1 {
        return false;
    }
    if name.as_bytes()[ln - lnum - 1] != b'_' {
        return false;
    }
    &name[ln - lnum..] == num
}

pub fn canonical_build_name(scanned: &str) -> &'static str {
    let num = match scanned.rfind('_') {
        Some(i) => &scanned[i + 1..],
        None => scanned,
    };
    for &(b, _) in BUILD_SEASONS {
        if ends_with_num(b, num) {
            return b;
        }
    }
    "None"
}
