# Phase 6 – USB-Ebene: RTL8152-Framing, `EthernetLink` und MIDI-SysEx

Grundlage: `xdj-rx3-link-export-protokoll.md` §4 (USB-Ebene) und §11.5 (USB-MIDI-SysEx). Die
Netzseite (Pro DJ Link, Portmapper/mountd, NFS, dbserver, Deck-Status) ist seit Phase 1–5 vollständig;
offen war nur die USB-Schicht unterhalb der Sockets.

## Was neu ist: `link/src/usb/`

| Modul | Inhalt |
|---|---|
| `usb::rtl8152` | Reines RTL8152/8153-Deskriptor-Framing (Protokoll §4.2): TX = 8-Byte-`tx_desc` + Frame, RX = 24-Byte-`rx_desc` je Frame (Länge inkl. FCS, 8-Byte-aligned), Checksum-Offload-Flags, GET_REGS-Konstanten. Keine Abhängigkeiten, golden getestet gegen die Beispiele der Spec (`0xC0000062` für 98 Byte, Transfer 368 B → Frame 334 B). |
| `usb::midi` | Das SysEx-Keepalive (Protokoll §11.5): `F0 00 40 05 00 00 03 0D 00 50 01 F7`, alle 200 ms. Paketierung in USB-MIDI-32-Bit-Wörter (`04 F0 00 40 | 04 05 00 00 | 04 03 0D 00 | 07 50 01 F7`), golden getestet. |
| `usb::EthernetLink` | Trait für einen rohen Ethernet-Link (Frames ohne FCS), die Austauschbarkeit hinter der Protokoll-Entscheidung „Weg A zuerst, Weg B dahinter kapseln“ (§4.3). |
| `usb::device` (Feature `usb`) | Weg B: Userspace-NIC `Rtl8152Nic` (Bulk OUT `0x02` / IN `0x81`) und `SysexSender` (Bulk OUT `0x04`) über [`nusb`](https://docs.rs/nusb). |

## Entscheidungen

- **`nusb` statt `rusb`.** Das Protokoll schlägt `rusb` vor („Crates (Vorschlag)“). `nusb` ist das
  reine-Rust-Äquivalent: kein libusb-C-Build, läuft auf Windows/Linux/macOS ohne Zusatzwerkzeug.
  Damit bleibt der Windows-Build des Workspace unberührt (das Feature ist aus, siehe unten).
- **Weg A bleibt Standard.** Der bestehende Pfad über die Betriebssystem-NIC (`net.rs`, normale
  Sockets) ist unverändert der Standard; `EthernetLink`/`usb::device` sind der Austauschpfad Weg B.
- **Feature-Gating.** Die Hardware-Anbindung liegt hinter `--features usb`; die reinen Codecs
  (`rtl8152`, `midi`) und der Trait sind immer kompiliert und getestet. So bricht weder
  `cargo test --workspace` noch `packaging/build-windows.ps1`.

## Tests (`link/tests/usb.rs` + Unit-Tests)

- TX-Deskriptor `opts1 = frame_len | FS | LS` → `0xC0000062` für 98 Byte; Transferlänge 8 + Frame.
- RX: Transfer 368 B = 24 + 338 (334 Frame + 4 FCS), auf 8 aufgerundet → ein 334-Byte-Frame; mehrere
  Frames je Transfer; Null-Längen-Deskriptor als Ende; abgeschnittene Transfers ohne Panik.
- Checksum-Flags: `UDP_CS|IPV4_CS = 0xA0000000`, `IPV4_CS = 0x20000000`, `TCP_CS|IPV4_CS = 0x60000000`.
- SysEx → genau die vier beobachteten USB-MIDI-Pakete; beliebige Längen werden korrekt chunked/beendet.

## Offen / nur am Gerät

- **`usb::device` ist ungetestet** (kein XDJ-RX3 beim Bau). Unter Windows ist das RTL8152-Gerät
  normalerweise an den Herstellertreiber gebunden (Weg A); für Weg B muss es auf WinUSB umgestellt
  werden (z. B. Zadig), unter Linux vom `r8152`-Treiber gelöst (`new_id`). Dasselbe gilt für das
  Audio/MIDI-Gerät beim SysEx.
- **Voller Userspace-IP-Stack (smoltcp) auf `EthernetLink`** ist die aufwändige Konsequenz von Weg B
  (Protokoll §4.3: „Aufwändiger“) und bewusst noch nicht gebaut – Weg A deckt den Betrieb ab. Das
  Trait ist der Einstiegspunkt dafür.
- **SysEx einbinden**: Protokoll §11.7.2 sagt „ab dem Linkstart alle 200 ms senden, zuerst mit und
  ohne testen“. `SysexSender` ist bereit, wird aber nicht automatisch mitgestartet (Zweck unklar,
  separates Gerät). Ein Test am Pult entscheidet, ob es an `net::start` gehängt wird.
- Der USB-Mitschnitt-Auswerter (USBPcap → Ethernet-Frames nach §4.2, Protokoll §10 Punkt 1) fehlt
  noch; dafür wird ein Mitschnitt als Fixture benötigt. Die Aufnahmen liefert auf Windows
  `packaging/windows/Link-Sniff.cmd` (`link-sniff.ps1`, USBPcap + dumpcap/pktmon, Szenarien §9).
