//! IPv4 network adapters with MAC address (needed for DJ Link keep-alives). Windows:
//! `GetAdaptersAddresses` (iphlpapi).

use std::net::Ipv4Addr;

/// One IPv4 address of an adapter.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Interface {
    pub name: String,
    pub description: String,
    pub ip: Ipv4Addr,
    pub prefix_len: u8,
    pub mac: [u8; 6],
    /// Adapter is up.
    pub up: bool,
    pub loopback: bool,
}

impl Interface {
    pub fn netmask(&self) -> Ipv4Addr {
        let bits = if self.prefix_len == 0 { 0 } else { u32::MAX << (32 - u32::from(self.prefix_len.min(32))) };
        Ipv4Addr::from(bits)
    }

    pub fn broadcast(&self) -> Ipv4Addr {
        Ipv4Addr::from(u32::from(self.ip) | !u32::from(self.netmask()))
    }
}

/// Pick an interface: by (part of) name, description or IP; else the first adapter that is up and not
/// loopback, preferring a DJ unit's own USB network adapter (Pioneer DJ / AlphaTheta), then private addresses, then link-local ones (169.254.x.x – what DJ gear and the PC
/// use on a direct cable without DHCP), then any other.
pub fn choose<'a>(list: &'a [Interface], wanted: Option<&str>) -> Option<&'a Interface> {
    if let Some(w) = wanted {
        let w = w.to_lowercase();
        return list.iter().find(|i| {
            i.name.to_lowercase().contains(&w) || i.description.to_lowercase().contains(&w) || i.ip.to_string() == w
        });
    }
    let usable: Vec<&Interface> = list.iter().filter(|i| i.up && !i.loopback).collect();
    // a DJ unit connected by USB brings its own network adapter ("Pioneer DJ Network Adapter")
    let dj = |i: &&&Interface| {
        let d = i.description.to_lowercase();
        d.contains("pioneer") || d.contains("alphatheta")
    };
    usable
        .iter()
        .find(dj)
        .or_else(|| usable.iter().find(|i| i.ip.is_private()))
        .or_else(|| usable.iter().find(|i| i.ip.is_link_local()))
        .or_else(|| usable.first())
        .copied()
}

#[cfg(windows)]
mod win {
    use super::Interface;
    use std::net::Ipv4Addr;

    #[repr(C)]
    struct SocketAddress {
        sockaddr: *const u8,
        len: i32,
    }

    #[repr(C)]
    struct Unicast {
        length: u32,
        flags: u32,
        next: *const Unicast,
        address: SocketAddress,
        prefix_origin: i32,
        suffix_origin: i32,
        dad_state: i32,
        valid_lifetime: u32,
        preferred_lifetime: u32,
        lease_lifetime: u32,
        on_link_prefix_length: u8,
    }

    #[repr(C)]
    struct Adapter {
        length: u32,
        if_index: u32,
        next: *const Adapter,
        adapter_name: *const u8,
        first_unicast: *const Unicast,
        first_anycast: *const u8,
        first_multicast: *const u8,
        first_dns: *const u8,
        dns_suffix: *const u16,
        description: *const u16,
        friendly_name: *const u16,
        physical_address: [u8; 8],
        physical_address_length: u32,
        flags: u32,
        mtu: u32,
        if_type: u32,
        oper_status: u32,
    }

    #[link(name = "iphlpapi")]
    extern "system" {
        fn GetAdaptersAddresses(family: u32, flags: u32, reserved: *const u8, buf: *mut u8, size: *mut u32) -> u32;
    }

    const AF_INET: u32 = 2;
    const SKIP_ANYCAST_MULTICAST_DNS: u32 = 0x2 | 0x4 | 0x8;
    const IF_TYPE_LOOPBACK: u32 = 24;
    const OPER_UP: u32 = 1;

    unsafe fn wide(p: *const u16) -> String {
        if p.is_null() {
            return String::new();
        }
        let mut n = 0;
        while *p.add(n) != 0 {
            n += 1;
        }
        String::from_utf16_lossy(std::slice::from_raw_parts(p, n))
    }

    pub fn list() -> Vec<Interface> {
        let mut size: u32 = 16 * 1024;
        let mut buf: Vec<u64> = Vec::new();
        for _ in 0..4 {
            buf = vec![0u64; size as usize / 8 + 1];
            let r = unsafe {
                GetAdaptersAddresses(AF_INET, SKIP_ANYCAST_MULTICAST_DNS, std::ptr::null(), buf.as_mut_ptr() as *mut u8, &mut size)
            };
            match r {
                0 => break,
                111 => continue, // ERROR_BUFFER_OVERFLOW: size was updated
                _ => return Vec::new(),
            }
        }
        let mut out = Vec::new();
        let mut a = buf.as_ptr() as *const Adapter;
        // SAFETY: the list is laid out by the OS inside `buf` and terminated by a null `next`.
        unsafe {
            while !a.is_null() {
                let ad = &*a;
                let mut mac = [0u8; 6];
                let n = (ad.physical_address_length as usize).min(6);
                mac[..n].copy_from_slice(&ad.physical_address[..n]);
                let mut u = ad.first_unicast;
                while !u.is_null() {
                    let un = &*u;
                    let sa = un.address.sockaddr;
                    if !sa.is_null() && un.address.len >= 8 && u16::from_le_bytes([*sa, *sa.add(1)]) == AF_INET as u16 {
                        out.push(Interface {
                            name: wide(ad.friendly_name),
                            description: wide(ad.description),
                            ip: Ipv4Addr::new(*sa.add(4), *sa.add(5), *sa.add(6), *sa.add(7)),
                            prefix_len: un.on_link_prefix_length,
                            mac,
                            up: ad.oper_status == OPER_UP,
                            loopback: ad.if_type == IF_TYPE_LOOPBACK,
                        });
                    }
                    u = un.next;
                }
                a = ad.next;
            }
        }
        out
    }
}

/// All IPv4 addresses of all adapters (empty on unsupported platforms).
pub fn list() -> Vec<Interface> {
    #[cfg(windows)]
    {
        win::list()
    }
    #[cfg(not(windows))]
    {
        Vec::new()
    }
}
