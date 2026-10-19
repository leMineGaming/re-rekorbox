# Phase 2 – Live-Spiegel der Decks (Deck-Status 0x0A)

Grundlage: `docs/link/PRO_DJ_LINK_SPEC.md` (aus einem zweiten Mitschnitt abgeleitete Protokoll-Spec,
Netzseite, keine Kryptographie). Diese Spec deckt genau die **Netzseite** ab, die rbx-link schon
byte-genau umsetzt. Der einzige inhaltlich neue, umsetzbare Teil ist der **ausführliche Deck-Status**
(UDP 50002, Typ 0x0A) – der Live-Spiegel.

## Abgleich Spec ↔ rbx-link

| Spec | Stand in rbx-link |
|---|---|
| §3 Announce/Keep-Alive/Claim (50000) | vollständig, byte-identisch (`packets`, `devices`) |
| §3.2 Keep-Alive-Offsets 0x30–0x35 | identisch (0x30=01, 0x31=peers, 0x34=Klasse, 0x35=08) |
| §3 Kandidatenfolge 0x11,0x12,0x29,0x2a,0x2b,0x2c | identisch (`CANDIDATES`) |
| §4 Status/Steuerung 50002 (0x30/0x10/0x11/0x46/0x47/0x16/0x31/0x29) | vollständig, byte-identisch |
| §4.6 Media-Slot 0x05/0x06 | vollständig |
| §5 dbserver (Port 12523, Nachrichtenformat, Menüs, 0x4101) | vollständig (`dbserver`, `server`) |
| §6 NFS Portmapper/mountd/MNT (50111/36251/2049) | Portmapper+mountd vollständig; NFS-Dateiinhalte = Phase 5 |
| **§4.2 Deck-Status 0x0A im Detail** | **NEU umgesetzt: `status::DeckStatus`** |

## Was neu ist: `link/src/status.rs`

`DeckStatus::parse` dekodiert den 292-Byte-Status eines Decks nach den Offsets der Spec §4.2:

| Feld | Offset | Regel |
|---|---|---|
| Gerätenr. (Deck) | 0x21 | |
| Slot | 0x29 | 0 kein, 1 CD, 2 SD, 3 USB, 4 rekordbox |
| Track-Typ | 0x2A | |
| Track-Id | 0x2C | nur gültig, wenn Slot ≠ 0 |
| Play-Zustand | 0x7B | Dynamik unbestätigt (Leerlauf-Mitschnitt) |
| Flags | 0x89 | |
| Pitch | 0x8C | `pitch_percent()`, 0x100000 = ±0 % |
| BPM×100 | **0x92 (u16)** | gültig, wenn ≠ 0xFFFF |
| Beat-Zähler | 0xA0 | gültig, wenn ≠ 0xFFFFFFFF |
| Beat im Takt | 0xA6 | 1–4 |
| Hardware-Klasse | 0xCC | XDJ-RX3 = 0x1F |

**Dabei behobener Fehler:** Unser bisheriger `CdjStatus` (im Crate `rekordbox-formats`) las die BPM als
32-Bit-Wert ab 0x90 mit maskiertem Bit 31. Das ist für das XDJ-RX3 falsch: 0x90 ist ein 2-Byte-Gültig-
keitsfeld (0x7FFF), die BPM×100 steht als **u16 bei 0x92**. Bei laufendem Track hätte das Müll ergeben.
`DeckStatus` nimmt jetzt die richtigen Felder; `rekordbox-formats::CdjStatus` bleibt unangetastet.

`link-announce` gibt die Decks live aus (nur bei Änderung): `deck N play … track … bpm … pitch …% beat …`.
Damit lässt sich der Live-Spiegel am Pult prüfen (Jog/Play/Tempo bewegen).

## Tests (`link/tests/status.rs`)

- Leerlauf-Status aus dem Fixture (Deck 12): kein Track, BPM/Beat „none“, Pitch 0 %, HW-Klasse 0x1F.
- Synthetischer Playing-Status: BPM 130,00, Pitch +6,25 %, effektive BPM 138,125, Beat 42, Takt 2.
- Fehlerfälle (zu kurz, falsche Magic).

## Offen / unbestätigt

- Der Mitschnitt war Leerlauf. **Play-Zustandscodes, Beat-Dynamik und Flags sind strukturell belegt,
  aber nicht in Bewegung** – zum Bestätigen ein Mitschnitt mit laufendem Track (dann auch UDP 50001
  Beat-Pakete, in der Spec §9 offen).
- Anbindung `DeckStatus` → Engine/UI (Wellenform-Overlay) ist der nächste Integrationsschritt.
- Die LINK-Quelle am Pult hängt am USB-HID-Kanal (separater Befund, nicht Teil dieser Netz-Spec).
