pub struct FeatureChain {
    pub feature: &'static str,
    pub build: &'static str,
    pub base_offset: u64,
    pub offsets: &'static [u64],
}

pub struct Patch {
    pub feature: &'static str,
    pub build: &'static str,
    pub branch: &'static str,
    pub addr: u64,
    pub bytes: &'static [u8],
}

pub struct ShadowRegion {
    pub offset: u64,
    pub patch: &'static [u8],
}

pub struct ShadowSeason {
    pub build: &'static str,
    pub regions: &'static [ShadowRegion],
}

pub struct TreeParams {
    pub build: &'static str,
    pub root_base: u64,
    pub root_offs: &'static [u64],
    pub name_off: i32,
    pub stride: i32,
    pub children_off: i32,
    pub root_count: i32,
    pub i_mp: i32,
    pub i_th: i32,
    pub i_situ: i32,
    pub i_mm: i32,
    pub i_gym: i32,
    pub i_vr: i32,
    pub i_ob: i32,
    pub situ_adv: i32,
    pub event_mode: &'static str,
    pub remove: &'static [i32],
}

include!("patches_data.rs");
include!("build_season.rs");
include!("feature_chains.rs");
include!("idle.rs");
include!("season_names.rs");
include!("tree_params.rs");
include!("map_names.rs");
include!("gametype_names.rs");
include!("feature_values.rs");
include!("shadow_regions.rs");
