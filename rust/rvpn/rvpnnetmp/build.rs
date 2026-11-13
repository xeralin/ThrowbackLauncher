fn main() {
    println!("cargo::rustc-link-arg-cdylib=/SUBSYSTEM:NATIVE");
    println!("cargo::rustc-link-arg-cdylib=/ENTRY:DriverEntry");
}
