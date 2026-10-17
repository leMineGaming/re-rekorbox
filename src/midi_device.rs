//! Windows MIDI device access through WinMM (the API the original uses, SPEC §5f-2: `midiInOpen` with a
//! callback). Only short messages are received; System Exclusive is ignored.
//!
//! [`list_inputs`]/[`list_outputs`] enumerate devices, [`InputPort::open`] starts receiving and delivers
//! `(packed message, timestamp ms)` pairs through a channel; [`OutputPort`] sends short messages
//! (LEDs).

use std::sync::mpsc::{channel, Receiver, Sender};

type Handle = isize;

#[repr(C)]
struct MidiInCaps {
    mid: u16,
    pid: u16,
    driver_version: u32,
    name: [u16; 32],
    support: u32,
}

#[repr(C)]
struct MidiOutCaps {
    mid: u16,
    pid: u16,
    driver_version: u32,
    name: [u16; 32],
    technology: u16,
    voices: u16,
    notes: u16,
    channel_mask: u16,
    support: u32,
}

type InCallback = extern "system" fn(Handle, u32, usize, usize, usize);

#[link(name = "winmm")]
extern "system" {
    fn midiInGetNumDevs() -> u32;
    fn midiInGetDevCapsW(id: usize, caps: *mut MidiInCaps, size: u32) -> u32;
    fn midiOutGetNumDevs() -> u32;
    fn midiOutGetDevCapsW(id: usize, caps: *mut MidiOutCaps, size: u32) -> u32;
    fn midiInOpen(handle: *mut Handle, id: u32, callback: InCallback, instance: usize, flags: u32) -> u32;
    fn midiInStart(handle: Handle) -> u32;
    fn midiInStop(handle: Handle) -> u32;
    fn midiInReset(handle: Handle) -> u32;
    fn midiInClose(handle: Handle) -> u32;
    fn midiOutOpen(handle: *mut Handle, id: u32, callback: usize, instance: usize, flags: u32) -> u32;
    fn midiOutShortMsg(handle: Handle, msg: u32) -> u32;
    fn midiOutReset(handle: Handle) -> u32;
    fn midiOutClose(handle: Handle) -> u32;
}

const CALLBACK_FUNCTION: u32 = 0x0003_0000;
const MIM_DATA: u32 = 0x3C3;

fn name_of(raw: &[u16; 32]) -> String {
    let end = raw.iter().position(|&c| c == 0).unwrap_or(raw.len());
    String::from_utf16_lossy(&raw[..end])
}

/// A MIDI device as reported by the driver.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeviceInfo {
    pub index: u32,
    pub name: String,
}

pub fn list_inputs() -> Vec<DeviceInfo> {
    let n = unsafe { midiInGetNumDevs() };
    (0..n)
        .filter_map(|i| {
            let mut caps: MidiInCaps = unsafe { std::mem::zeroed() };
            let r = unsafe { midiInGetDevCapsW(i as usize, &mut caps, std::mem::size_of::<MidiInCaps>() as u32) };
            (r == 0).then(|| DeviceInfo { index: i, name: name_of(&caps.name) })
        })
        .collect()
}

pub fn list_outputs() -> Vec<DeviceInfo> {
    let n = unsafe { midiOutGetNumDevs() };
    (0..n)
        .filter_map(|i| {
            let mut caps: MidiOutCaps = unsafe { std::mem::zeroed() };
            let r = unsafe { midiOutGetDevCapsW(i as usize, &mut caps, std::mem::size_of::<MidiOutCaps>() as u32) };
            (r == 0).then(|| DeviceInfo { index: i, name: name_of(&caps.name) })
        })
        .collect()
}

/// Received short message: packed bytes (`status | d1 << 8 | d2 << 16`) and driver timestamp in ms
/// since [`InputPort::open`].
pub type RawEvent = (u32, u32);

extern "system" fn on_input(_h: Handle, msg: u32, instance: usize, p1: usize, p2: usize) {
    if msg == MIM_DATA && instance != 0 {
        // SAFETY: `instance` is the Box<Sender> owned by the open InputPort, freed only after close.
        let tx = unsafe { &*(instance as *const Sender<RawEvent>) };
        let _ = tx.send((p1 as u32, p2 as u32));
    }
}

/// An open, started MIDI input; closed on drop.
pub struct InputPort {
    handle: Handle,
    sender: *mut Sender<RawEvent>,
}

impl InputPort {
    /// Open input device `index` and start receiving.
    pub fn open(index: u32) -> Result<(InputPort, Receiver<RawEvent>), String> {
        let (tx, rx) = channel();
        let sender = Box::into_raw(Box::new(tx));
        let mut handle: Handle = 0;
        let r = unsafe { midiInOpen(&mut handle, index, on_input, sender as usize, CALLBACK_FUNCTION) };
        if r != 0 {
            drop(unsafe { Box::from_raw(sender) });
            return Err(format!("midiInOpen({index}) failed with code {r}"));
        }
        let port = InputPort { handle, sender };
        let r = unsafe { midiInStart(handle) };
        if r != 0 {
            return Err(format!("midiInStart failed with code {r}"));
        }
        Ok((port, rx))
    }
}

impl Drop for InputPort {
    fn drop(&mut self) {
        unsafe {
            midiInStop(self.handle);
            midiInReset(self.handle);
            midiInClose(self.handle);
            drop(Box::from_raw(self.sender));
        }
    }
}

/// An open MIDI output; closed on drop.
pub struct OutputPort {
    handle: Handle,
}

// SAFETY: a WinMM output handle may be used from any thread; calls are not interleaved here because
// `send` takes `&self` on a value owned by one thread at a time.
unsafe impl Send for OutputPort {}

impl OutputPort {
    pub fn open(index: u32) -> Result<OutputPort, String> {
        let mut handle: Handle = 0;
        let r = unsafe { midiOutOpen(&mut handle, index, 0, 0, 0) };
        if r != 0 {
            return Err(format!("midiOutOpen({index}) failed with code {r}"));
        }
        Ok(OutputPort { handle })
    }

    /// Send a packed short message (`status | d1 << 8 | d2 << 16`).
    pub fn send(&self, packed: u32) -> Result<(), String> {
        match unsafe { midiOutShortMsg(self.handle, packed) } {
            0 => Ok(()),
            r => Err(format!("midiOutShortMsg failed with code {r}")),
        }
    }
}

impl Drop for OutputPort {
    fn drop(&mut self) {
        unsafe {
            midiOutReset(self.handle);
            midiOutClose(self.handle);
        }
    }
}
