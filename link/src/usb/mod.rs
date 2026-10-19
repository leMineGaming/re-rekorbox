//! USB layer for LINK EXPORT (`xdj-rx3-link-export-protokoll.md` §4): the XDJ-RX3 exposes the
//! Pro DJ Link network as an **RTL8152/8153 Ethernet-over-USB** device (VID:PID `2B73:0007`), plus a
//! separate audio/MIDI/HID device (`2B73:003D`) behind an internal hub (`2B73:003E`).
//!
//! There are two ways to reach the network (protokoll §4.3):
//!
//! * **Weg A – OS NIC (default):** the OS binds the RTL8152 chip and exposes a normal network
//!   adapter; the rest of this crate ([`crate::net`]) then just uses plain sockets. No USB code is
//!   needed at all.
//! * **Weg B – userspace NIC:** claim the raw USB device (with the `usb` feature, via [`device`])
//!   and drive the [`rtl8152`] descriptors behind the [`EthernetLink`] trait, then run a userspace
//!   IP stack (e.g. smoltcp) on top of it.
//!
//! Weg A is used first (protokoll §4.3); Weg B is interchangeable behind [`EthernetLink`]. The
//! framing codecs ([`rtl8152`], [`midi`]) are pure and always compiled, so they can be tested against
//! the capture without any device.

use crate::error::LinkError;

pub mod midi;
pub mod rtl8152;

#[cfg(feature = "usb")]
pub mod device;

/// The vendor id of AlphaTheta/Pioneer DJ (the XDJ-RX3 in particular).
pub const VID: u16 = 0x2B73;
/// The internal USB hub.
pub const PID_HUB: u16 = 0x003E;
/// The Ethernet-over-USB (RTL8152) device carrying Pro DJ Link.
pub const PID_NIC: u16 = 0x0007;
/// The audio + MIDI + HID device ("SOFTWARE CONTROL").
pub const PID_AUDIO_MIDI_HID: u16 = 0x003D;

/// A raw Ethernet link (Weg B): sends and receives whole Ethernet frames **without** the 4-byte FCS.
///
/// This is the interchange point between the OS network stack (Weg A, no [`EthernetLink`]) and a
/// userspace NIC that feeds a userspace TCP/IP stack (Weg B). A user who runs smoltcp on top of the
/// USB device implements or picks one implementation here ([`device::Rtl8152Nic`]).
pub trait EthernetLink {
    /// The station MAC used on this link.
    fn mac(&self) -> [u8; 6];

    /// Maximum frame payload (Ethernet MTU, usually 1500).
    fn mtu(&self) -> usize;

    /// Send one Ethernet frame (without FCS). The descriptor framing is the implementation's job.
    fn send(&mut self, frame: &[u8]) -> Result<(), LinkError>;

    /// Receive one Ethernet frame (without FCS), or `Ok(None)` when none is pending.
    fn recv(&mut self) -> Result<Option<Vec<u8>>, LinkError>;
}
