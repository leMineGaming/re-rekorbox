//! Weg B (userspace NIC) device access, behind the `usb` feature (`xdj-rx3-link-export-protokoll.md`
//! §4.3). Uses [`nusb`] — pure Rust, so it builds on Windows without a C toolchain, unlike `rusb`.
//!
//! This is the hardware glue the protokoll lists as the more effortful Weg B; it is **untested on
//! real hardware** (no XDJ-RX3 at build time). On Windows the RTL8152 device is normally bound to
//! the vendor network driver (Weg A); to use it from here it must be switched to WinUSB (e.g. with
//! Zadig), and on Linux detached from the `r8152` driver. The same applies to the audio/MIDI device
//! for the SysEx keepalive. None of this affects Weg A, which is the default.

use std::io::{Read, Write};
use std::time::Duration;

use nusb::transfer::{Bulk, ControlIn, ControlType, Direction, Out, Recipient};
use nusb::{DeviceInfo, MaybeFuture};

use crate::error::LinkError;
use crate::usb::{midi, rtl8152, EthernetLink, PID_AUDIO_MIDI_HID, PID_NIC, VID};

/// Transfer timeout for blocking USB operations.
const TIMEOUT: Duration = Duration::from_secs(1);
/// Buffer size for the bulk-IN reader (one Ethernet frame plus its descriptor fits easily).
const RX_BUF: usize = 2048;

fn usb(e: impl std::fmt::Display) -> LinkError {
    LinkError::Usb(e.to_string())
}

/// Find an XDJ-RX3 device by product id.
fn find(pid: u16) -> Result<DeviceInfo, LinkError> {
    nusb::list_devices()
        .wait()
        .map_err(usb)?
        .find(|d| d.vendor_id() == VID && d.product_id() == pid)
        .ok_or_else(|| LinkError::Usb(format!("device {VID:04X}:{pid:04X} not found")))
}

/// A userspace NIC on the RTL8152 bulk endpoints (protokoll §4.2, §4.3 Weg B).
pub struct Rtl8152Nic {
    iface: nusb::Interface,
    mac: [u8; 6],
    mtu: usize,
}

impl Rtl8152Nic {
    /// Open the network device and claim interface 0 (the Ethernet function, protokoll §4.1).
    pub fn open(mac: [u8; 6]) -> Result<Rtl8152Nic, LinkError> {
        let device = find(PID_NIC)?.open().wait().map_err(usb)?;
        let iface = device.claim_interface(0).wait().map_err(usb)?;
        Ok(Rtl8152Nic {
            iface,
            mac,
            mtu: 1500,
        })
    }

    /// Read a 16-bit tally register (vendor GET_REGS, protokoll §4.2 statistics polling).
    pub fn read_reg(&self, reg: u16) -> Result<u16, LinkError> {
        let data = self
            .iface
            .control_in(
                ControlIn {
                    control_type: ControlType::Vendor,
                    recipient: Recipient::Device,
                    request: rtl8152::REQ_GET_REGS,
                    value: reg,
                    index: 0,
                    length: 2,
                },
                TIMEOUT,
            )
            .wait()
            .map_err(usb)?;
        let lo = data.first().copied().unwrap_or(0);
        let hi = data.get(1).copied().unwrap_or(0);
        Ok(u16::from_le_bytes([lo, hi]))
    }
}

impl EthernetLink for Rtl8152Nic {
    fn mac(&self) -> [u8; 6] {
        self.mac
    }

    fn mtu(&self) -> usize {
        self.mtu
    }

    fn send(&mut self, frame: &[u8]) -> Result<(), LinkError> {
        let transfer = rtl8152::tx_transfer(frame, 0);
        let ep = self
            .iface
            .endpoint::<Bulk, Out>(rtl8152::EP_BULK_OUT)
            .map_err(usb)?;
        let mut w = ep.writer(transfer.len().max(512));
        w.write_all(&transfer).map_err(usb)?;
        w.flush().map_err(usb)?;
        Ok(())
    }

    fn recv(&mut self) -> Result<Option<Vec<u8>>, LinkError> {
        let ep = self
            .iface
            .endpoint::<Bulk, nusb::transfer::In>(rtl8152::EP_BULK_IN)
            .map_err(usb)?;
        let mut r = ep.reader(RX_BUF);
        r.set_read_timeout(Duration::from_millis(1));
        let mut raw = [0u8; RX_BUF];
        match r.read(&mut raw) {
            Ok(n) => Ok(rtl8152::rx_frames(&raw[..n])
                .first()
                .map(|frame| frame.to_vec())),
            Err(e)
                if matches!(
                    e.kind(),
                    std::io::ErrorKind::TimedOut | std::io::ErrorKind::WouldBlock
                ) =>
            {
                Ok(None)
            }
            Err(e) => Err(usb(e)),
        }
    }
}

/// Sends the MIDI SysEx keepalive (protokoll §11.5) on the audio/MIDI device's bulk-OUT `0x04`.
pub struct SysexSender {
    iface: nusb::Interface,
}

impl SysexSender {
    /// Open the audio/MIDI device and claim the interface that carries the MIDI bulk-OUT endpoint.
    pub fn open() -> Result<SysexSender, LinkError> {
        let info = find(PID_AUDIO_MIDI_HID)?;
        let device = info.open().wait().map_err(usb)?;
        let index = midi_interface(&device)?;
        let iface = device.claim_interface(index).wait().map_err(usb)?;
        Ok(SysexSender { iface })
    }

    /// Write the keepalive once.
    pub fn send_once(&self) -> Result<(), LinkError> {
        let ep = self
            .iface
            .endpoint::<Bulk, Out>(midi::EP_BULK_OUT)
            .map_err(usb)?;
        let mut w = ep.writer(16);
        for pkt in midi::usb_midi_packets(&midi::SYSEX) {
            w.write_all(&pkt).map_err(usb)?;
        }
        w.flush().map_err(usb)?;
        Ok(())
    }

    /// Blocking loop sending the keepalive every 200 ms. Returns on a USB error (e.g. unplug).
    pub fn run(&self) {
        loop {
            if let Err(e) = self.send_once() {
                tracing::warn!("SysEx keepalive stopped: {e}");
                return;
            }
            std::thread::sleep(midi::INTERVAL);
        }
    }
}

/// Index of the interface whose alt settings contain the MIDI bulk-OUT endpoint `0x04`.
fn midi_interface(device: &nusb::Device) -> Result<u8, LinkError> {
    for config in device.configurations() {
        for iface in config.interfaces() {
            let number = iface.interface_number();
            for alt in iface.alt_settings() {
                for ep in alt.endpoints() {
                    if ep.transfer_type() == nusb::descriptors::TransferType::Bulk
                        && ep.direction() == Direction::Out
                        && ep.address() == midi::EP_BULK_OUT
                    {
                        return Ok(number);
                    }
                }
            }
        }
    }
    Err(LinkError::Usb(
        "MIDI streaming interface (bulk OUT 0x04) not found".into(),
    ))
}
