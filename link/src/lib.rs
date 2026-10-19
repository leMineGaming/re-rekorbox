//! Pro DJ Link for the XDJ-RX3 in LINK EXPORT mode: rekordbox-rs takes rekordbox's place on the
//! network (see `docs/link/`): presence (device number, keep-alives, device table, answers on port
//! 50002), portmapper/mountd, the database server for browsing, and the network thread with hot-plug.
//!
//! * [`packets`] – pure codecs (bytes ↔ values)
//! * [`devices`] – the time-driven presence state machine
//! * [`rpc`] – portmapper (UDP 50111) and mountd: export list and MNT
//! * [`catalog`] – the library as the players see it
//! * [`dbserver`] – database session (menus, metadata, analysis data), pure
//! * [`server`] – port query (TCP 12523) and database connections
//! * [`net`] – sockets, adapter choice, thread, events
//! * [`usb`] – the USB layer (RTL8152 Ethernet-over-USB framing, MIDI SysEx, [`usb::EthernetLink`])

pub mod catalog;
pub mod dbserver;
pub mod devices;
pub mod error;
pub mod net;
pub mod nfs;
pub mod packets;
pub mod rpc;
pub mod server;
pub mod status;
pub mod usb;

pub use error::{LinkError, PacketError};
