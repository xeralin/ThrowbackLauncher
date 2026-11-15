use core::net::Ipv4Addr;

pub fn is_private_v4(host: u32) -> bool {
    let ip = Ipv4Addr::from_bits(host);
    ip.is_loopback() || ip.is_private() || ip.is_link_local() || host & 0xFFC00000 == 0x64400000
}
