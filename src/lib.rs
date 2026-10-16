//! Clean-room implementation of rekordbox data formats.
//!
//! Everything here is implemented from the behavioural specification in `SPEC.md`
//! (section references are given as `§5b` etc.), not from the original program code.
//!
//! * [`anlz`] – track analysis files `ANLZ0000.DAT` / `.EXT` / `.2EX` (container, typed tags)
//!   and the MessagePack based `.3EX` embedding file.
//! * [`xml`] – the `rekordbox.xml` library exchange format (model, writer, reader).
//! * [`midi`] – MIDI controller mapping CSV files (`MidiMappings/*.midi.csv`).

pub mod anlz;
pub mod midi;
pub mod xml;
