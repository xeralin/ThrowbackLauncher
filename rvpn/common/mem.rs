use core::ffi::{c_char, c_void};

#[allow(dead_code)]
#[cfg_attr(
    target_arch = "x86",
    link(
        name = "ntdll.dll",
        kind = "raw-dylib",
        modifiers = "+verbatim",
        import_name_type = "noprefix"
    )
)]
#[cfg_attr(
    not(target_arch = "x86"),
    link(name = "ntdll.dll", kind = "raw-dylib", modifiers = "+verbatim")
)]
unsafe extern "C" {
    #[cfg_attr(target_arch = "x86", link_name = "_memcpy")]
    fn memcpy(dest: *mut c_void, src: *const c_void, count: usize) -> *mut c_void;
    #[cfg_attr(target_arch = "x86", link_name = "_memset")]
    fn memset(dest: *mut c_void, value: i32, count: usize) -> *mut c_void;
    #[cfg_attr(target_arch = "x86", link_name = "_memcmp")]
    fn memcmp(a: *const c_void, b: *const c_void, count: usize) -> i32;
    #[cfg_attr(target_arch = "x86", link_name = "_strlen")]
    fn strlen(s: *const c_char) -> usize;
}
