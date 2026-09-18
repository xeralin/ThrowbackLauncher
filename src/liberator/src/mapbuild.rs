use crate::tables::BUILD_SEASONS;

pub fn canonical_build_name(scanned: &str) -> &'static str {
    let num = match scanned.rfind('_') {
        Some(i) => &scanned[i + 1..],
        None => scanned,
    };
    BUILD_SEASONS
        .iter()
        .find(|(b, _)| b.strip_suffix(num).is_some_and(|p| p.ends_with('_')))
        .map_or("None", |(b, _)| b)
}
