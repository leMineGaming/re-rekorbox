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
| `xml` | `rekordbox.xml`: Modell, Writer (Attributreihenfolge, Zahlenformate), Reader (Toleranz- und Validierungsregeln), Location-URL, Escaping, Beatgrid-Export-Modi 0/1/2, Cue-Nummern | §5e |
| `rbf` (CLI) | `anlz`, `check`, `xml`, `xml-rewrite` | – |

## Bauen und testen

Voraussetzung: Rust (stable, getestet mit 1.99.1).

```bash
cargo test
```

32 Tests auf **synthetischen** Testdaten (keine Originaldateien im Repository):
23 für ANLZ, 9 für rekordbox.xml. Erwartete Bytes und Regeln stammen jeweils aus der SPEC.

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

**Validierung gegen eine echte Bibliothek** (lokal, 2026-10-16; Ausgabe in `analysis/check_report.txt`):
7307 Dateien (1831 DAT, 1827 EXT, 1826 2EX, 1823 3EX), **0 Fehler** – alle 5484 Container byte-identisch
zurückgeschrieben, alle typisierten Tags identisch neu kodiert, alle 23 431 Phrasen mit Label.

## Grenzen

- **rekordbox.xml** ist nur aus dem Dekompilat spezifiziert; es lag keine echte XML-Datei vor (SPEC U20–U22).
  Bewusste Designentscheidungen, wo das Original nicht bestimmt wurde:
  Einrückung (2 Leerzeichen), `\` → `/` und Entfernen eines führenden `/` in `Location`,
  `\n`/`\r` als `&#10;`/`&#13;`, `PRODUCT Name="rekordbox"` als Vorgabe.
- **Unbekannte Feldbedeutungen** werden unverändert mitgeführt, aber nicht interpretiert:
  `PMAI`-Felder, `PCPT`-Felder außer Slot, `PQT2`-Nutzdaten, `PSSI u16[5..8]`, `PWV4`-Einträge, Farbsemantik
  von `PWV5 c0..c2` (SPEC §7).
- `PVBR`, `PCO2`, `PWVC`, `PKEY`, `PVDI` werden roh durchgereicht.
- Hot-Cue-Farben beim XML-Import: rekordbox ordnet RGB einer 64-Farben-Palette zu; diese Palette ist nicht
  extrahiert, daher bleibt RGB unverändert.

## Bewusst nicht nachgebaut

- Lizenzierung, Aktivierung, Kopierschutz (`lsapiw64.dll`, `clatc.dll`).
- Entschlüsselung von `master.db` / `product.db` – kein Schlüssel wird extrahiert oder eingebettet.
- Cloud-/Account-Dienste und deren Protokolle.
- Audio-Engine, DSP, UI, ML-Modelle (Beat-/Cue-Erkennung, Stem-Separation).
- `export.pdb` (Geräte-Datenbank): noch nicht analysiert (SPEC U23).

## Projektstruktur

```
SPEC.md                    Verhaltensspezifikation mit Belegen (REA-Evidence, Ghidra-Adressen, Beobachtungen)
src/anlz/                  ANLZ-Implementierung
src/xml/                   rekordbox.xml-Implementierung
src/bin/rbf.rs             CLI
tests/                     Tests mit synthetischen Daten
analysis/scripts/          Auswerteskripte für die Black-Box-Beobachtung (Python)
analysis/ghidra_scripts/   Ghidra-Abfrageskript RbQuery.java; analysis/rbq.sh als Aufrufer
```

`analysis/ghidra/` (persistentes Ghidra-Projekt, ≈1,1 GB) und `analysis/out/` (Dekompilat-Rohausgaben) sind vom
Originalprogramm abgeleitet und per `.gitignore` ausgeschlossen.
