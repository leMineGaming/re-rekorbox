//! Pro DJ Link for the XDJ-RX3 in LINK EXPORT mode: rekordbox-rs takes rekordbox's place on the
//! network (see `docs/link/`). Phase 1: packets, presence (device number, keep-alives, device table,
//! answers on port 50002) and the network thread with hot-plug.
//!
//! * [`packets`] – pure codecs (bytes ↔ values)
//! * [`devices`] – the time-driven presence state machine
//! * [`rpc`] – portmapper (UDP 50111) and mountd: export list and MNT
//! * [`net`] – sockets, adapter choice, thread, events

pub mod devices;
pub mod error;
pub mod net;
pub mod packets;
pub mod rpc;

pub use error::{LinkError, PacketError};
