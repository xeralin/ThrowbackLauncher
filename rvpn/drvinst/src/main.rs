#![no_std]
#![no_main]
#![windows_subsystem = "console"]

use core::arch::asm;
use core::panic::PanicInfo;

#[panic_handler]
fn panic(_: &PanicInfo) -> ! {
    unsafe { asm!("ud2", options(noreturn)) }
}

#[unsafe(no_mangle)]
extern "C" fn mainCRTStartup() -> u32 {
    0
}
