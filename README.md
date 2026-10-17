# rekordbox-formats

Clean-Room-Reimplementierung von rekordbox-Datenformaten in Rust – entwickelt **aus der
Verhaltensspezifikation [`SPEC.md`](SPEC.md)**, nicht aus dem Originalcode. Ziel: Verständnis,
Interoperabilität und eigene Erweiterungen für eine Installation von rekordbox 7.2.19, die dem Nutzer gehört.

## Umfang

| Modul | Inhalt | SPEC |
|---|---|---|
| `anlz` | Container `ANLZ0000.DAT/.EXT/.2EX` (PMAI + Tags), byte-genauer Round-Trip, unbekannte Tags bleiben erhalten | §5a–§5c |
| `anlz::tags` | `PPTH` (Pfad), `PQTZ` (Beatgrid), `PCOB`/`PCPT` (Cue-Listen), `PQT2`, `PVB2` | §5b, §5c |
| `anlz::waveform` | `PWAV`/`PWV2` (inkl. Längen-Toleranz), `PWV3`–`PWV7`, Bit-Layouts Mono/Farbe | §5b, §5d |
| `anlz::phrase` | `PSSI` mit Header-Prüfungen, Mood, **Phrasen-Labels** (Player-Umrechnung), Fill-in | §5c, §5d |
| `anlz::threex` | `.3EX` (MessagePack, `embedding`) dekodieren/kodieren | §5a |
| `xml` | `rekordbox.xml`: Modell, Writer (Attributreihenfolge, Zahlenformate, Original-Layout), Reader (Toleranz- und Validierungsregeln), Location-URL, Escaping, Beatgrid-Export-Modi 0/1/2 und Import-Expansion, Cue-Nummern, Hot-Cue-Palette | §5e |
| `midi` | MIDI-Mapping-CSV (`MidiMappings/*.midi.csv`): Loader-Regeln, Optionen, Typ-Codes inkl. Pad-Auflösung je Gerät, Nachrichten je Deck (Hex/Kanal, `Dual`), Writer | §5f |
| `pad` | Performance-Pad-Dateien (`pad/*.pad.csv`): Zeilenformat, Benutzervarianten, Modusliste | §5g |
| `midi_input` | Verarbeitung eingehender MIDI-Nachrichten je Steuerelement-Typ (Button, Pads, Jog, Rotary mit Beschleunigung, KnobSlider/Value-Skalierung, 14-Bit MSB/LSB) | §5f-2 |
| `midi_device` | WinMM-Zugriff (Windows): MIDI-Geräte auflisten, Eingang öffnen | §5f-2 |
| `devicesql` | DeviceSQL-String-Kodierung (kurz ASCII / lang UTF-16 mit Markern) und `.edb`-Container | §5h |
| `djlink` | Pro DJ Link: Kopfformate, Port 50000 (Discovery, ID-Anfragen, Keep-alive, Disconnect), Beat (50001), CDJ- und Mixer-Status (50002), Gerätetabelle mit Zwei-Deck-Regel | §5i |
| `djlink_participant` | Teilnahme am DJ-Link-Netz als Zustandsmaschine: Discovery, Nummernvergabe (0x11, 0x12, 0x29–0x2C), Keep-alive alle 2 s, Konflikt, Alterung | §5i |
| `djlink_net` | UDP-Betrieb der Teilnahme (Ports 50000–50002), Empfang fremder Beat-/Statuspakete, eigene Beat-Pakete, Beat-Uhr | §5i |
| `netif` | IPv4-Netzwerkadapter mit MAC und Broadcast-Adresse (Windows `GetAdaptersAddresses`) | – |
| `library` | Track-Bibliothek aus dem rekordbox-Analyseordner (ohne Datenbank): Zuordnung zu Audiodateien über den Dateinamen, Beatgrid, Cues (falls gespeichert), Phrasen, Länge; Suche | §5a |
| `rbf` (CLI) | `anlz`, `check`, `xml`, `xml-rewrite`, `midi`, `midi-route`, `midi-decode`, `midi-devices`, `midi-monitor`, `pad`, `edb`, `djlink-listen`, `library` | – |

Dazu im Workspace: **[`engine/`](engine/README.md)** – Zwei-Deck-Audio-Engine (`rbx-engine`, Programm `rbx-play`),
Eigenentwurf für das MVP, nicht aus dem rekordbox-Code abgeleitet: EQ, Sound Color FX, 14 Beat FX, Pad FX,
Sampler, Kopfhörer-Cue, Browser; der XDJ-RX3 ist mit allen 220 Mapping-Funktionen angebunden (`--midi`).
Außerdem eine **eigene Track-Analyse** (`rbx-analyze`, automatisch beim Laden): Beatgrid, Tonart und alle
Wellenformen, geschrieben als ANLZ-Dateien im rekordbox-Format (SPEC §5j, Genauigkeit §8a).

**[`ui/`](ui/README.md)** – die Oberfläche (`rbx-ui`, egui): Trackliste mit Analyse-Status, zwei Decks mit
Wellenformen, Mixer, Effekte, Sampler, Controller und DJ Link – damit lässt sich das MVP direkt ausprobieren:

```bash
cargo run --release -p rbx-ui
```

## Bauen und testen

Voraussetzung: Rust (stable, getestet mit 1.99.1).

```bash
cargo test --workspace
```

77 Tests auf **synthetischen** Testdaten (keine Originaldateien im Repository):
25 für ANLZ, 14 für rekordbox.xml, 9 für MIDI-Mappings, 9 für die MIDI-Eingangsverarbeitung, 5 für Pad-Dateien,
4 für DeviceSQL, 3 für die Bibliothek, 21 für DJ Link (davon 2 über echte UDP-Sockets auf 127.0.0.1), 1 Doku-Test; dazu 61 Tests der Audio-Engine inkl. Effekten und Controller-Anbindung (`engine/`).

## CLI

```bash
cargo run --release --bin rbf -- check "%APPDATA%\Pioneer\rekordbox\share\PIONEER\USBANLZ"
```

| Befehl | Wirkung |
|---|---|
| `rbf anlz <datei>` | Tags einer ANLZ-Datei anzeigen (Beatgrid, Wellenformen, Phrasen mit Labels; `.3EX` als Wert-Baum) |
| `rbf check <ordner>` | alle ANLZ-Dateien parsen, Container- und Tag-Round-Trip prüfen, Statistik ausgeben |
| `rbf xml <datei>` | rekordbox.xml zusammenfassen (inkl. verworfener Elemente) |
| `rbf xml-rewrite <ein> <aus>` | rekordbox.xml einlesen und mit dieser Implementierung neu schreiben |
| `rbf midi <datei\|ordner>` | MIDI-Mappings parsen, Zuordnungen je Deck berechnen, Round-Trip prüfen |
| `rbf pad <ordner>` | Pad-Ordner prüfen (Version, Modi, alle Pad-Dateien, Round-Trip) |
| `rbf edb <datei>` | DeviceSQL-`.edb`-Datei lesen, Zeilen nach Tabelle gruppieren, Strings dekodieren |
| `rbf midi-route <mapping>` | Eingangsnachrichten eines Mappings je Typ zählen, Zeilen ohne Eingangsverarbeitung nennen |
| `rbf midi-decode <mapping> <hex>…` | MIDI-Bytes offline verarbeiten, z. B. `B0 22 41 95 00 7F` |
| `rbf midi-devices` | MIDI-Ein-/Ausgänge auflisten (Windows) |
| `rbf midi-monitor <mapping> [gerät] [s]` | Live-MIDI eines Controllers mit einem Mapping dekodieren (Windows) |
| `rbf djlink-listen [s] [--raw]` | DJ-Link-Verkehr auf UDP 50000–50002 nur mithören und dekodieren, Gerätetabelle ausgeben |
| `rbf library [analyse] [musik] [suche]` | Bibliothek aus dem Analyseordner bauen (Standard: rekordbox-Ordner und Musik-Ordner), Statistik und Suche |

### Pioneer DJ XDJ-RX3

- **MIDI** (USB, Modus „PC/Mac (MIDI)“ am Gerät): `rbf midi-route` lädt das mitgelieferte
  `MidiMappings/XDJ-RX3.midi.csv` mit 730 Eingangsnachrichten (u. a. 116 Pad-, 82 Button-, 15 KnobSlider-Zeilen);
  nur die Gerätewerte `JogRatency` und `MidiOutInterval` (Typ `Parameter`) haben keine Eingangsverarbeitung.
  Live-Test, sobald das Gerät angeschlossen ist:
  `rbf midi-monitor "C:\Program Files
ekordbox
ekordbox 7.2.19\MidiMappings\XDJ-RX3.midi.csv"`
  (das Eingangsgerät wird über den Namen „XDJ-RX3“ gesucht).
- **DJ Link** (LAN): `rbf djlink-listen 60 --raw > xdj.txt` zeichnet auf, was das Gerät sendet; damit lassen sich
  SPEC U28–U30 (Gerätenummer, Ladeinfo, Zustandsnamen) bestätigen. rekordbox selbst sollte dabei geschlossen sein
  (Ports 50000–50002), Windows fragt beim ersten Start ggf. nach einer Firewall-Freigabe.
- Nicht geprüft: Beim Bau war kein XDJ-RX3 angeschlossen (`rbf midi-devices` zeigte nur den Windows-Synthesizer);
  alles oben ist aus dem Dekompilat und mit synthetischen Paketen getestet.

**Validierung gegen eine echte Bibliothek** (lokal, 2026-10-16; Ausgabe in `analysis/check_report.txt`):
7307 Dateien (1831 DAT, 1827 EXT, 1826 2EX, 1823 3EX), **0 Fehler** – alle 5484 Container byte-identisch
zurückgeschrieben, alle typisierten Tags identisch neu kodiert, alle 23 431 Phrasen mit Label.
Bibliothek (2026-10-17): 1831 Analyseordner, 1825 Audiodateien zugeordnet, 1805 mit Beatgrid (265 mit
Tempowechsel), 1794 mit Phrasen, keine gespeicherten Cues. Die 69 mitgelieferten MIDI-Mappings: alle gültig, 13 505 Einträge, 69/69 Round-Trips identisch
(`analysis/midi_report.txt`).

## Grenzen

- **rekordbox.xml** ist aus Dekompilat und Assembly spezifiziert, aber nicht gegen eine echte Exportdatei geprüft.
  Aus dem Code übernommen: Ausgabeformat (CRLF, Einrückung 2, Attribut-Umbruch > 60 Bytes), Escaping
  (Steuerzeichen inkl. Zeilenumbrüche in Attributen entfernt), Location-Regeln, Abschneiden von `AverageBpm`,
  TEMPO-Import als Beatgrid-Expansion, 64-Farben-Hot-Cue-Palette.
  Offen bzw. Designentscheidung: Datumsformat von `DateAdded` (SPEC U20), Vorgabe `PRODUCT Name="rekordbox"`.
- **Unbekannte Feldbedeutungen** werden unverändert mitgeführt, aber nicht interpretiert:
  `PMAI`-Felder, `PCPT`-Felder `@1E` und `@28…@34`, `PQT2`-Nutzdaten, `PSSI u16[5..8]`, Farbsemantik
  von `PWV5 c0..c2` (SPEC §7). Der Inhalt von `PWV4`/`PWV6`/`PWV7` ist durch Kalibrierung zugeordnet (SPEC §5j).
- `PCO2`, `PWVC`, `PKEY`, `PVDI` werden roh durchgereicht; die PVDI-Verschleierung wird bewusst nicht nachgebaut.
- **MIDI**: Toggle, Parameter, JogRotateAbsPos und JogTimestamp haben keine Eingangsverarbeitung; die
  Tempo-Polynomkurve des DDJ-SX2 (Typ 8 auf `B?04`) ist nicht nachgebaut; System-Exclusive wird nicht empfangen.
- **DJ Link**: Teilnahme (Nummernvergabe, Keep-alive, eigene Beat-Pakete) ist umgesetzt, aber nur über Loopback
  getestet, nicht in einem echten Netz mit Geräten. Eigene CDJ-Statuspakete (für Sync von Geräten auf dieses
  Programm) werden nicht gesendet; weitere Status-Pakettypen (z. B. 0x05, 0x33, 0x56) werden erkannt, aber nicht
  dekodiert.

## Bewusst nicht nachgebaut

- Lizenzierung, Aktivierung, Kopierschutz (`lsapiw64.dll`, `clatc.dll`).
- Entschlüsselung von `master.db` / `product.db` – kein Schlüssel wird extrahiert oder eingebettet.
- Cloud-/Account-Dienste und deren Protokolle.
- Audio-Engine, DSP, UI (Eigenentwürfe statt Nachbau); ML-Modelle und Gewichte (Beat-/Cue-Erkennung,
  Stem-Separation) werden nicht verwendet – die Analyse nutzt eigene, öffentliche Verfahren.
- NFS-Dateifreigabe für DJ-Link-Geräte (`PSvNFSd.exe`, `NFSDaemon.dll`) und die Mobilgeräte-/Track-Übertragung.
- `export.pdb` (Geräte-Datenbank): Engine, Strings und Zeilen sind belegt, das Seitenformat noch nicht – dafür wird ein
  USB-/Ordner-Export als Testdatei benötigt (SPEC U23).

## Projektstruktur

```
SPEC.md                    Verhaltensspezifikation mit Belegen (REA-Evidence, Ghidra-Adressen, Beobachtungen)
src/anlz/                  ANLZ-Implementierung
src/xml/                   rekordbox.xml-Implementierung
src/midi*.rs, src/djlink.rs  MIDI-Mappings, MIDI-Eingang, DJ Link
src/bin/rbf.rs             CLI
tests/                     Tests mit synthetischen Daten
engine/                    Audio-Engine, Analyse, Controller, Sitzung (rbx-play, rbx-analyze)
ui/                        Oberfläche (rbx-ui)
analysis/scripts/          Auswerteskripte für die Black-Box-Beobachtung (Python)
analysis/ghidra_scripts/   Ghidra-Abfrageskript RbQuery.java; analysis/rbq.sh als Aufrufer
```

`analysis/ghidra/` (persistentes Ghidra-Projekt, ≈1,1 GB) und `analysis/out/` (Dekompilat-Rohausgaben) sind vom
Originalprogramm abgeleitet und per `.gitignore` ausgeschlossen.
