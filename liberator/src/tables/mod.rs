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

#[derive(Clone, Copy)]
pub struct ShadowRegion {
    pub offset: u64,
    pub patch: &'static [u8],
}

pub struct ShadowSeason {
    pub build: &'static str,
    pub regions: &'static [ShadowRegion],
}

pub const RAX: u8 = 0;
pub const RCX: u8 = 1;
pub const RBX: u8 = 3;
pub const RSI: u8 = 6;
pub const RDI: u8 = 7;
pub const R13: u8 = 13;
pub const R15: u8 = 15;

pub enum PlaylistSend {
    Block {
        vptr: u64,
        ctor: u64,
        fill: u64,
        msg: u64,
        send: u64,
        dtor: u64,
        frame: u32,
    },
    Channel {
        vptr: u64,
        ctor: u64,
        fill: u64,
        msg: u64,
        send: u64,
        dtor: u64,
        cookie: u64,
    },
    Call(u64),
}

pub struct PlaylistSync {
    pub build: &'static str,
    pub hook: u64,
    pub moved: &'static [u8],
    pub gm: u8,
    pub alt_gm: u8,
    pub host: u32,
    pub net: u32,
    pub playlist: u32,
    pub send: PlaylistSend,
    pub late: bool,
}

pub struct WarmupGate {
    pub build: &'static str,
    pub site: u64,
    pub moved: &'static [u8],
    pub mode: u8,
    pub kind: u32,
    pub asset: u32,
    pub no: u64,
    pub back: u64,
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
include!("playlist_sync.rs");
include!("warmup_gates.rs");
include!("unlock.rs");
include!("vulkan.rs");
include!("dx11.rs");
include!("timers.rs");
