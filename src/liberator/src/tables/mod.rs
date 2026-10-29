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

pub struct RendererBuild {
    pub build: &'static str,
    pub rvas: &'static [(u64, u64)],
    pub patches: &'static [RendererPatch],
}

pub struct RendererPatch {
    pub rva: u64,
    pub dx: &'static [u8],
    pub alt: &'static [u8],
}

pub struct ClassicTimer {
    pub build: &'static str,
    pub global: u64,
    pub role: u64,
    pub state: u64,
    pub now: u64,
    pub start: u64,
    pub end: u64,
    pub length: u64,
    pub freeze: u64,
    pub armed: u64,
    pub countup: u64,
    pub second: u32,
}

pub struct TreeParams {
    pub build: &'static str,
    pub root_base: u64,
    pub root_offs: &'static [u64],
    pub name_off: i32,
    pub stride: i32,
    pub children_off: i32,
    pub root_count: u64,
    pub i_gym: i32,
    pub i_vr: i32,
    pub i_ob: i32,
    pub situ_adv: bool,
    pub event_mode: &'static str,
    pub remove: &'static [usize],
}

include!("patches.rs");
include!("seasons.rs");
include!("feature_chains.rs");
include!("idle.rs");
include!("tree_params.rs");
include!("names.rs");
include!("shadow_regions.rs");
include!("unlock.rs");
include!("vulkan.rs");
include!("dx11.rs");
include!("timers.rs");
