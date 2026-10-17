//! Clean-room implementation of rekordbox data formats.
//!
//! Everything here is implemented from the behavioural specification in `SPEC.md`
//! (section references are given as `§5b` etc.), not from the original program code.
//!
//! * [`anlz`] – track analysis files `ANLZ0000.DAT` / `.EXT` / `.2EX` (container, typed tags)
//!   and the MessagePack based `.3EX` embedding file.
//! * [`xml`] – the `rekordbox.xml` library exchange format (model, writer, reader).
//! * [`midi`] – MIDI controller mapping CSV files (`MidiMappings/*.midi.csv`).
//! * [`djlink`] – Pro DJ Link packets (announce/keep-alive, beat, status) and device table (§5i).
//! * [`midi_input`] – processing of incoming MIDI messages per control type (§5f-2).
//! * [`pad`] – performance pad definitions (`pad/*.pad.csv`).

pub mod anlz;
pub mod capture;
pub mod dbserver;
pub mod devicesql;
pub mod djlink;
pub mod djlink_net;
pub mod djlink_participant;
pub mod midi;
pub mod midi_input;
pub mod library;
pub mod netif;
#[cfg(windows)]
pub mod midi_device;
pub mod pad;
pub mod xml;
