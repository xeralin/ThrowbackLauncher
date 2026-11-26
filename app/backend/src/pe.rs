use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::path::Path;

const HEADERS: u64 = 4096;
const SECTION: usize = 40;
const FIXED_FILE_INFO: &[u8; 4] = b"\xbd\x04\xef\xfe";

fn field(bytes: &[u8], at: usize, width: usize) -> Option<u64> {
    let field = bytes.get(at..at.checked_add(width)?)?;
    Some(
        field
            .iter()
            .rev()
            .fold(0, |value, byte| value << 8 | u64::from(*byte)),
    )
}

fn resources(head: &[u8]) -> Option<(u64, u64)> {
    let pe = usize::try_from(field(head, 0x3c, 4)?).ok()?;
    if head.get(pe..pe.checked_add(4)?)? != b"PE\0\0" {
        return None;
    }
    let count = usize::try_from(field(head, pe + 6, 2)?).ok()?;
    let table = pe + 24 + usize::try_from(field(head, pe + 20, 2)?).ok()?;
    (0..count).find_map(|index| {
        let entry = table + SECTION * index;
        if head.get(entry..entry + 8)? != b".rsrc\0\0\0" {
            return None;
        }
        Some((field(head, entry + 20, 4)?, field(head, entry + 16, 4)?))
    })
}

fn fixed_version(data: &[u8]) -> Option<String> {
    let mut at = 0;
    loop {
        at += data
            .get(at..)?
            .windows(4)
            .position(|window| window == FIXED_FILE_INFO)?;
        if let Some(info) = data.get(at + 4..at + 16)
            && let [struc_version, ms, ls] = info.as_chunks::<4>().0
            && u32::from_le_bytes(*struc_version) == 0x0001_0000
        {
            let (ms, ls) = (u32::from_le_bytes(*ms), u32::from_le_bytes(*ls));
            return Some(format!("{}.{}.{}", ms >> 16, ms & 0xffff, ls >> 16));
        }
        at += 4;
    }
}

pub fn file_version(file: &Path) -> Option<String> {
    let mut exe = File::open(file).ok()?;
    let mut head = Vec::new();
    exe.by_ref().take(HEADERS).read_to_end(&mut head).ok()?;
    let (offset, size) = resources(&head)?;
    exe.seek(SeekFrom::Start(offset)).ok()?;
    let mut section = Vec::new();
    exe.take(size).read_to_end(&mut section).ok()?;
    fixed_version(&section)
}

#[cfg(any(test, feature = "testing"))]
pub fn versioned_exe(version: &str) -> Vec<u8> {
    let mut parts = version
        .split('.')
        .map(|part| part.parse::<u32>().unwrap_or(0));
    let mut number = || parts.next().unwrap_or(0);
    let (major, minor, patch) = (number(), number(), number());
    let pe: usize = 0x40;
    let table = pe + 24;
    let data = table + SECTION;
    let mut exe = vec![0; data];
    exe[..2].copy_from_slice(b"MZ");
    exe[0x3c] = pe as u8;
    exe[pe..pe + 4].copy_from_slice(b"PE\0\0");
    exe[pe + 6] = 1;
    exe[table..table + 5].copy_from_slice(b".rsrc");
    exe[table + 16] = 16;
    exe[table + 20] = data as u8;
    exe.extend(FIXED_FILE_INFO);
    exe.extend(0x0001_0000_u32.to_le_bytes());
    exe.extend((major << 16 | minor).to_le_bytes());
    exe.extend((patch << 16).to_le_bytes());
    exe
}

#[cfg(test)]
mod tests {
    use std::fs;

    use super::*;

    fn version_info(struc_version: u32, ms: u32, ls: u32) -> Vec<u8> {
        let mut data = FIXED_FILE_INFO.to_vec();
        for field in [struc_version, ms, ls] {
            data.extend(field.to_le_bytes());
        }
        data
    }

    #[test]
    fn version_comes_from_the_fixed_file_info() {
        let mut data = b"MZ\xbd\x04\xef".to_vec();
        data.extend(version_info(0x0001_0000, 0x0000_0001, 0x0008_0002));
        assert_eq!(fixed_version(&data).as_deref(), Some("0.1.8"));
        let mut data = version_info(0x0002_0000, 0x0009_0009, 0x0009_0000);
        data.extend(version_info(0x0001_0000, 0x0001_0002, 0x0003_0004));
        assert_eq!(fixed_version(&data).as_deref(), Some("1.2.3"));
        assert_eq!(
            fixed_version(&version_info(0x0001_0000, 0xffff_ffff, 0xffff_0000)).as_deref(),
            Some("65535.65535.65535")
        );
    }

    #[test]
    fn version_scan_steps_four_bytes_and_needs_sixteen() {
        let full = version_info(0x0001_0000, 0x0001_0000, 0);
        assert_eq!(fixed_version(&full[..15]), None);
        assert_eq!(fixed_version(b""), None);
        assert_eq!(fixed_version(b"\xbd\x04\xef"), None);
        let mut nested = b"\xbd\x04\xef\xfe\x00".to_vec();
        nested.extend(&full);
        assert_eq!(fixed_version(&nested).as_deref(), Some("1.0.0"));
        let mut overlap = b"\xbd\x04\xef\xfe\xbd\x04".to_vec();
        overlap.extend(&full);
        assert_eq!(fixed_version(&overlap).as_deref(), Some("1.0.0"));
        let mut hidden = b"\xbd\x04\xef\xfe\x00\x00".to_vec();
        hidden.extend(&full[..4]);
        hidden.extend([0, 0, 1, 0, 5, 0, 4, 0, 0, 0, 3, 0]);
        assert_eq!(fixed_version(&hidden).as_deref(), Some("4.5.3"));
    }

    #[test]
    fn the_version_is_read_from_the_resource_section_only() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("program.exe");
        assert_eq!(file_version(&file), None);
        fs::write(&file, versioned_exe("0.2.5")).unwrap();
        assert_eq!(file_version(&file).as_deref(), Some("0.2.5"));
        fs::write(&file, version_info(0x0001_0000, 0x0000_0002, 0x0005_0000)).unwrap();
        assert_eq!(file_version(&file), None);
        let mut renamed = versioned_exe("0.2.5");
        renamed[0x58..0x5d].copy_from_slice(b".data");
        fs::write(&file, renamed).unwrap();
        assert_eq!(file_version(&file), None);
        let mut cut = versioned_exe("0.2.5");
        cut.truncate(0x90 - 4);
        fs::write(&file, cut).unwrap();
        assert_eq!(file_version(&file), None);
    }
}
