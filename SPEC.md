# rekordbox 7.2.19 – Verhaltensspezifikation (Clean-Room)

Status (Stand 2026-10-16): Target-Überblick (Abschn. 1–5) und **ANLZ-Format** (5a Beobachtung, 5b–5d Dekompilat)
spezifiziert, **rekordbox.xml** (5e) aus dem Dekompilat. Noch offen: export.pdb, MIDI-CSV.

Legende für Belege:
- **[REA:…]** = REA-Evidence-ID (Tool-Ergebnis mit Hash/Limitations)
- **[LOK:…]** = lokale, statische Beobachtung ohne REA (eigenes PE-Import-Skript, `grep -a`, Dateilisting).
  Diese Belege sind schwächer: Treffer in Strings beweisen nicht, dass bzw. wie der Code sie nutzt.
- **Beobachtung / Inferenz / Unbekannt** werden explizit getrennt.

---

## 1. Target-Identität

| Feld | Wert | Beleg |
|---|---|---|
| Datei | `rekordbox.exe`, 103 540 656 Bytes | [REA:ev_63944b29…] |
| SHA-256 | `c9ed23e974c51c75e2ec069fbd2679a3dbe606be9e79e9cbbd13d6418ab11c37` | [REA:ev_63944b29…] |
| Format | PE32+, Machine 0x8664 (x86-64) | [REA:ev_63944b29…] |
| Version | FileVersion 7.2.19.0, AlphaTheta Corporation | [LOK:VersionInfo] |
| Ressourcen | 6 Icons (16…256 px), 1 Icon-Gruppe, VERSIONINFO (lang 1041/JP), Manifest | [REA:ev_63944b29…] |
| Sektionen | .text 55,6 MB, .rdata 42,8 MB, .data 4,0 MB, .pdata, _RDATA, .fptable, .rsrc, .reloc | [LOK:PE-Parser] |

## 2. Sprache / Runtime / Framework

- **Beobachtung:** C++ (MSVC-RTTI-Namen wie `.?AV?$Array@UANLZBlock@analyzer@@VDummyCriticalSection@ju…`). [LOK:grep]
- **Beobachtung:** String `JUCE v8.0.8` → UI/Audio-Framework JUCE 8.0.8, statisch gelinkt (keine JUCE-DLL). [LOK:grep]
- **Inferenz:** Rendering über OpenGL (342 Importe aus OPENGL32) + Direct2D/D3D11/DComp (JUCE-8-Renderer). [LOK:PE-Imports]
- Kein Qt, kein wxWidgets gefunden (bounded search: nur ASCII-Literal-Suche). [LOK:grep]

## 3. Komponenten im Installationsverzeichnis

| Komponente | Rolle (Inferenz aus Namen/Importen/Metadaten) | Beleg |
|---|---|---|
| `rekordbox.exe` | Hauptanwendung (Bibliothek, Decks, Analyse, Export, MIDI/HID) | [LOK] |
| `rekordboxAgent-win32-x64/` | Electron-App (`resources/app.asar`, 109 MB), Version 3.3.0 – „Cloud Agent“ laut Firewall-Skript | [LOK:updatefwrule.bat] |
| `rbHttpServer.exe` | lokaler HTTP-Server (Firewall-Regel „http server process“) | [LOK:updatefwrule.bat] |
| `edb_streamd.exe` | Streaming-Daemon (Firewall-Regel „edb streamd“) | [LOK] |
| `NFSDaemon.dll` | NFS-Server für Pro DJ Link (Exports: `GetMountNum`, `GetExportListEntryArray`, `StopNFSDaemon` …) | [LOK:PE-Imports] |
| `PSvLinkSysMgrINI.xml` | Link-Konfiguration: DeviceId 17, Type 4, Timer Discovery/Id/RunWait=100, Random=300, Aging=10 … | [LOK:Datei] |
| `sqlite3.dll` 3.45.3 + String `sqlcipher_export(...)` | Datenbank-Engine, **verschlüsselte** DBs (SQLCipher-artig) | [LOK] |
| `master.db`, `product.db` | Header nicht `SQLite format 3\0` → verschlüsselt; REA: „file is not a database“ | [REA:inspect_sqlite_database-Fehler] |
| `tensorflow_cc.dll`, `onnxruntime.dll`, `DirectML.dll`, `models/` | ML: Beat-Detection (`detect_beat/model7, model8`, Keras SavedModel), Cue-Vorhersage (`samplecnn.onnx`, `cue_prediction_model_no_abs_position.onnx`), `embed_model` | [LOK] |
| `Demucs3.dll`, `Spleeter.dll` | Stem-Separation | [LOK] |
| `lsapiw64.dll` (THALES, 10.1.100.20) | Sentinel-Lizenzierung | [LOK:VersionInfo] |
| `clatc.dll` (105 MB) | obfuskierte C++-Exporte (`clatc::eAePjpgUaZ`, `PlatformDelegate`) – vermutlich Lizenz/Schutz | [LOK:PE-Imports] |
| `libcrypto-3`, `libssl-3`, `cacert.pem` | TLS / Signaturprüfung (`EVP_VerifyFinal`) | [LOK] |
| `MidiMappings/*.midi.csv` (69) | Controller-Mappings, CSV-Klartext | [LOK:Datei] |
| `pad/*.pad.csv` (35) | Performance-Pad-Funktionen, `function=…,value=…` | [LOK:Datei] |
| `locale/*.lang` (18) | UI-Texte, Klartext (`@code`, `language: …`) | [LOK:Datei] |
| `skins/` (2431 Dateien), `drivers/`, `firmware/` | UI-Skins, Treiber-Installer, Firmware-Updater | [LOK] |

## 4. System-Abhängigkeiten (Importe von rekordbox.exe) [LOK:PE-Imports]

Audio/MIDI: `WINMM` (midiIn/midiOut), `MFReadWrite`/`MF` (Media Foundation Decoding), `libmpg123.dll` (im Ordner).
Hardware: `HID.DLL`, `SETUPAPI` (Geräte-Erkennung/Eject), `wlanapi`, `IPHLPAPI` (Netz-Interfaces für Pro DJ Link).
Netz: `WS2_32`, `WININET`, OpenSSL. Archiv: `zip.dll`, `zlib.dll`. Krypto: `bcrypt`, `ADVAPI32` (CryptoAPI), `CRYPT32`.

## 5. Interoperable Datenformate (Kandidaten für die Reimplementierung)

Alle folgenden Punkte sind **nur durch Strings belegt** – die Struktur ist noch NICHT spezifiziert.

| Format | Belegende Strings | Beleg |
|---|---|---|
| ANLZ-Analysedateien (`.DAT`, `.EXT`, `.2EX`) | `PIONEER/USBANLZ/P001/0000001A/ANLZ0000.DAT`, `ANLZ%04X.DAT`, `ANLZBlock@analyzer` | [LOK:grep] |
| ANLZ-Tag-Kennungen | `PMAI`, `PPTH`, `PVBR`, `PQTZ`, `PQT2`, `PCOB`, `PCO2`, `PSSI`, `PKEY`, `PWAV`, `PWV2`–`PWV7`, `PWVC` | [LOK:grep] |
| Geräte-Export-DB | `PIONEER/rekordbox/export.pdb`, `exportExt.pdb` | [LOK:grep] |
| XML-Bibliothek | `rekordbox.xml`, `DJ_PLAYLISTS`, `COLLECTION`, `TRACK`, `TrackID`, `AverageBpm`, `Tonality`, `POSITION_MARK`, `PlayCount` | [LOK:grep] |
| Legacy-Playlists | `masterPlaylists6.xml` | [LOK:grep] |
| MIDI-Mapping-CSV | Header `#name,function,type,input,deck1..4,output,deck1..4,option,comment` | [LOK:Datei] |

## 5a. ANLZ-Analysedateien – Spezifikation (Stand: Black-Box-Beobachtung)

**Methode:** Statische Auswertung der Dateien, die rekordbox 7 selbst erzeugt hat (eigene Bibliothek des Nutzers unter
`%APPDATA%\Pioneer\rekordbox\share\PIONEER\USBANLZ\…`), mit eigenen Skripten (`anlz_survey.py`, `anlz_hdr.py`,
`anlz_sem.py`, `mp.py`). Stichprobe: **7307 Dateien** (1831 `.DAT`, 1827 `.EXT`, 1826 `.2EX`, 1823 `.3EX`).
Belegtyp **[OBS:anlz-survey]** = Beobachtung am Ausgabeformat des Programms (kein Code). Semantik ist **Inferenz**,
bis sie durch das Dekompilat bestätigt ist (Ghidra-Analyse läuft).

### Speicherort
- Lokal: `share/PIONEER/USBANLZ/<3 Hex-Zeichen>/<UUID-Rest>/ANLZ0000.{DAT,EXT,2EX,3EX}` [OBS]
- Auf Exportmedien laut Strings: `PIONEER/USBANLZ/P001/0000001A/ANLZ0000.DAT`, Muster `ANLZ%04X.DAT` [LOK:grep]

### Container (DAT/EXT/2EX) – Beobachtung, 0 Abweichungen in 5484 Dateien
Alle Ganzzahlen **Big-Endian**.

| Offset | Typ | Bedeutung | Beleg |
|---|---|---|---|
| 0 | 4×ASCII | `PMAI` | alle 5484 |
| 4 | u32 | Header-Länge = **28** | alle |
| 8 | u32 | Gesamtlänge = **Dateigröße** | alle |
| 12 | 16 Bytes | konstant `00000001 00010000 00010000 00000000` (Bedeutung unbekannt, U8) | 1 Variante |
| 28 | Tags | Folge von Tags bis Dateiende | lückenlos |

Jeder Tag: `fourcc[4]`, `u32 header_len`, `u32 total_len`, dann `header_len-12` Bytes Header-Rest, dann
`total_len-header_len` Bytes Nutzdaten. Der nächste Tag beginnt bei `offset + total_len`. Nie `total_len < header_len`.

**Beobachtete Tag-Reihenfolgen** (häufigste):
- `.DAT`: `PPTH PVBR PQTZ PWAV PWV2 PCOB PCOB` (1831/1831)
- `.EXT`: `PPTH PWV3 PCOB PCOB PCO2 PCO2 PQT2 PWV5 PWV4 PVB2 PSSI` (1731); Varianten ohne `PVB2`, `PSSI`, `PQT2`
- `.2EX`: `PPTH PWV7 PWV6 PWVC` (1826/1826)

### Tags (Header-Rest = Wörter w0, w1, … ab Tag-Offset 12)

| Tag | header_len | Header-Felder (beobachtet) | Nutzdaten | Status |
|---|---|---|---|---|
| `PPTH` | 16 | w0 = Länge der Nutzdaten in Bytes | Pfad, **UTF-16BE**, NUL-terminiert (3658/3658); beginnt bei lokalen Dateien mit `?/` | Beobachtung |
| `PVBR` | 16 | w0 = 0 | immer 1604 Bytes; in Stichprobe Nullen am Anfang (Inferenz: VBR-Seek-Tabelle, U9) | teilweise |
| `PQTZ` | 24 | w0=0, w1=`0x00080000`, w2 = Anzahl N | N × 8 Bytes: `u16 beat`, `u16 tempo`, `u32 time` | siehe unten |
| `PWAV` | 20 | w0 = Anzahl Bytes (400), w1 = `0x00010000` | 400 Bytes; obere 3 Bit 0…7, untere 5 Bit 0…31 | Inferenz: Vorschau-Wellenform (Farbe/Höhe) |
| `PWV2` | 20 | w0 = 100, w1 = `0x00010000` | 100 Bytes | Inferenz: Mini-Wellenform |
| `PWV3` | 24 | w0 = 1 (Bytes/Eintrag), w1 = N, w2 = `0x00960000` | N × 1 Byte | Inferenz: Detail-Wellenform, 150 Einträge/s (0x96=150) |
| `PWV4` | 24 | w0 = 6, w1 = 1200, w2 = 0 | 1200 × 6 Bytes | Inferenz: Farb-Vorschau |
| `PWV5` | 24 | w0 = 2, w1 = N, w2 = `0x00960305` | N × 2 Bytes | Inferenz: Farb-Detail-Wellenform |
| `PWV6` | 20 | w0 = 3, w1 = 1200 | 1200 × 3 Bytes | Inferenz: 3-Band-Vorschau |
| `PWV7` | 24 | w0 = 3, w1 = N, w2 = `0x00960000` | N × 3 Bytes | Inferenz: 3-Band-Detail |
| `PWVC` | 14 | 2 Bytes `0000` | 6 Bytes | unbekannt (U10) |
| `PCOB` | 24 | w0 = 1 bzw. 0 (Inferenz: Hot-Cue- bzw. Memory-Liste), w1 = 0, w2 = `0xFFFFFFFF` | **immer leer** (3662+3654) | Cues liegen in rekordbox 7 offenbar in der DB (Inferenz) |
| `PCO2` | 20 | w0 = 1/0, w1 = 0 | immer leer | wie PCOB |
| `PQT2` | 56 | w1 ∈ {`0x01000002`, `0x02000002`}, w7 = N | N × 2 Bytes | Inferenz: erweitertes Beatgrid (U11) |
| `PVB2` | 32 | w0=w1=0, w2 variabel (Inferenz: Gesamtzahl Samples, z. B. 7 056 000 = 160 s × 44 100), w3 = 400 (Anzahl), w4 = 20 (Eintragsgröße) | 400 × 20 Bytes | Inferenz |
| `PSSI` | 32 | w0 = 24 (Eintragsgröße), w1 = `count<<16 \| x` (x=1 beob.), w3 variabel, w4 ∈ {`0x00010000`,`0x01010000`} | count × 24 Bytes, im Klartext (keine Maskierung beobachtet) | Inferenz: Song-Struktur/Phrasen (U12) |

### PQTZ (Beatgrid) – geprüfte Hypothesen über 1805 nicht-leere Dateien
- `beat` liegt immer in 1…4 und zählt zyklisch 1,2,3,4,1,… (1805/1805)
- `time` ist streng steigend (1805/1805); Inferenz: Millisekunden (Abstand 403 ms bei Tempo 14898 → 60000/148,98 = 402,7 ms ✔)
- `tempo` liegt in 6000…25000 → **BPM × 100** (Inferenz, durch die Rechnung oben gestützt)
- 1537/1805 Dateien haben ein konstantes Tempo; die übrigen sind variable Beatgrids
- 26 Dateien haben N = 0 (leeres Beatgrid)

### `.3EX` – kein PMAI, sondern MessagePack (1823/1823 vollständig dekodiert, 0 Rest-Bytes)
Struktur: `{"embedding": {"d2": str, "d3": str(32 Hex = 128 bit), "d4": f64, "d5": [[…]×7], "d6": [f64×64],
"d7": [f64×2], "d8": f64, "d9": int, "d10": int, "d11": f64, "d12": f64}}`.
Inferenz: Merkmalsvektor (Embedding) für die Track-Empfehlung (`networkRecommend.db`, `models/embed_model`);
`d11` sah im Beispiel wie BPM aus (148,98). Semantik der Schlüssel unbekannt (U13).

## 5b. ANLZ – Code-Belege aus dem Dekompilat (Ghidra 12.1.4)

Belegtyp **[GH:addr]** = Ghidra-Dekompilat von `rekordbox.exe` (SHA-256 `c9ed23e9…1c37`), Funktion an `addr`.
Erzeugt mit `analysis/rbq.sh` (persistentes Projekt, `-readOnly -noanalysis`); Rohausgaben unter `analysis/out/`.
String-Fundstellen zusätzlich als REA-Evidence **[REA:ev_38b70296…]** (gleiche Ghidra-Version, gleiche Adressen).
Funktionsnamen sind Ghidra-Platzhalter (`FUN_…`); es gibt keine PDB.

### Lokalisierung
- Tag-Kennungen stehen nicht als Strings im Binary, sondern als 32-Bit-Konstanten im Code. Suche nach allen 18
  Kennungen in beiden Byte-Reihenfolgen (`analysis/out/imm_tags.txt`) ergibt zwei Code-Cluster:
  - **Cluster A `0x14126f190–0x14127f3a0`**: Stream-Reader (je Tag eine Funktion) + Default-Konstruktoren.
  - **Cluster B `0x1412b8bb0–0x1412c3280`**: Puffer-basierte Konvertierung (`CMP` gegen Kennung + Byte-Swap), z. B. für Exportmedien (enthält Funktion mit String `USBANLZ` @ `0x1412bf3de`).
- Alle Kennungen liegen im Speicher als Little-Endian-`u32` vor (z. B. `PMAI` = `0x49414d50`) → in der Datei stehen die vier ASCII-Bytes in Lesereihenfolge.
- Pfad-Strings: `ANLZ0000.DAT`/`.EXT` [GH:0x141223d90], `ANLZ%04X.DAT` [GH:0x1412241a0, 0x141224580].

### Lesehilfen [GH:0x14126e4c0, 0x14126e4a0]
- `be_u32(p) = p[0]<<24 | p[1]<<16 | p[2]<<8 | p[3]`, `be_u16(p) = p[0]<<8 | p[1]` → **Big-Endian bestätigt**.
- `FUN_14126ddb0(stream, buf, n)` liest n Bytes, Rückgabe = gelesene Anzahl.
- Fehlercodes der Reader: **−8** = zu wenig Bytes, **−2** = falsche Kennung, **−1** = unerwarteter Listentyp, **−7** = Speicher.

### PMAI [GH:0x1412703d0 Reader, GH:0x14126f190 Default]
Reader liest 28 Bytes: `magic`, `u32 header_len @4`, `u32 total_len @8`, **sechs `u16` @0x0C…0x16**, `u32 @0x18`.
Default-Werte: header_len = 0x1C; u16-Felder = `0, 1, 1, 0, 1, 0`; u32 @0x18 = 0 → genau die in allen Dateien
beobachteten Bytes `00000001 00010000 00010000 00000000`. Bedeutung der Felder weiter unbekannt (U8).

### PPTH [GH:0x141270520]
Header 16 Bytes: `u16 @0x0C` (Default 0), **`u16 @0x0E` = Pfadlänge in Bytes**. Danach `len/2` UTF-16-Code-Units,
jeweils mit `be_u16` gelesen → **UTF-16BE bestätigt**. Default: header_len = total_len = 0x10 (leerer Pfad).

### PVBR [GH:0x14126f490 Default, GH:0x141270770 Reader]
Header 16 Bytes (`u16@0C`). Nutzlänge L: ist L ≥ 0x640, werden **400 × `be_u32`** gelesen (Tabelle); ist L = 0x644 oder
L = 4, folgt (bzw. besteht L aus) **ein weiteres `be_u32`** („extra“); andere Längen ≠ 0x640: Tabelle und extra = 0, Nutzdaten
übersprungen. Default: 0x640 Nullbytes. [OBS] In allen 1831 Dateien L = 0x644, Tabelle vollständig 0 (Inferenz:
VBR-Suchtabelle, nur bei VBR-Dateien gefüllt), extra ≠ 0 in 18 Dateien. Bedeutung von extra offen (U9).

### DAT-Lesealgorithmus [GH:0x14126fb80]
Anders als EXT/2EX **streng sequenziell**: PMAI, PPTH, PVBR, PQTZ, PWAV, PWV2, PCOB (Typ 1, sonst −1), PCOB (Typ 0,
sonst −1) – jeder Tag muss in genau dieser Reihenfolge folgen, sonst −2 (bzw. −8 bei zu wenig Bytes). Ein Flag
des Aufrufers beendet das Lesen nach PPTH (Nur-Pfad-Modus). Unbekannte Tags sind in `.DAT` damit nicht zulässig.

### PQTZ [GH:0x141270980 Reader, GH:0x141270ac0 Einträge]
Header 24 Bytes: `u16 @0x0C`, `u16 @0x0E`, **`u16 @0x10` (Default 8)**, `u16 @0x12`, **`u32 @0x14` = Anzahl N**.
Einträge: N × 8 Bytes, je `be_u16, be_u16, be_u32`; gelesen in Blöcken von höchstens 256 Einträgen (0x800 Bytes).
Default `u16 @0x10` = 8 = Eintragsgröße (Inferenz). Semantik (beat / tempo×100 / time ms) aus Abschnitt 5a (Beobachtung).

### PWAV / PWV2 [GH:0x141270cf0, GH:0x141270e80]
Header 20 Bytes: `u16 @0x0C`, **`u16 @0x0E` = Anzahl** (Default 400 bzw. 100), **`u16 @0x10`** (Default 1).
Nutzlänge = total_len − header_len. **Nur wenn sie genau 400 (PWAV) bzw. 100 (PWV2) beträgt, wird sie gelesen**;
sonst wird der Puffer genullt und die Nutzdaten übersprungen (tolerantes Lesen).

### PCOB / PCPT – Cue-Listen [GH:0x141271020, 0x1412714a0, 0x141271190, 0x141271bb0]
- PCOB-Header 24 Bytes: `u16 @0x0C`, **`u16 @0x0E` = Listentyp** (Reader 0x141271020 verlangt 1, Reader 0x1412714a0 verlangt 0),
  **`u16 @0x12` = Anzahl Einträge**, `u16 @0x10`, `u16 @0x14`, `u16 @0x16`. Default: Typ 0, `@0x10..0x13` = 0, `@0x14..0x17` = `0xFFFFFFFF`.
- Inferenz: Typ 1 = Hot Cues, Typ 0 = Memory Cues (durch Slot-Logik gestützt, nicht durch Namen).
- **Neuer Tag `PCPT`** (Cue-Eintrag), Länge 0x38 = 56 Bytes, Header 0x1C:

| Offset | Typ | Beobachtung im Code |
|---|---|---|
| 0x00 | `PCPT` | Kennung geprüft |
| 0x04 / 0x08 | u32 / u32 | header_len (0x1C) / total_len (0x38) |
| 0x0C | u16 | 0 (Builder) |
| 0x0E | u16 | **Hot-Cue-Slot**: 0 = Memory Cue, 1… = A…; aus interner Art: 1–3 → 1–3, 5 → 4, 6…17 → Art − 1 [GH:0x141fc4720]; der Reader legt 1…3 in feste Slots |
| 0x10 | u32 | 0; = 4 bei interner Art 4 [GH:0x141fc4720] |
| 0x14 | u32 | 1 (Builder-Vorgabe) |
| 0x18 | u32 | 0xFFFFFFFF (Builder-Vorgabe) |
| 0x1C | u8 | **Typ: 1 = Cue, 2 = Loop** (2, wenn ein Ende gesetzt ist) |
| 0x1D | u8 | 0 |
| 0x1E | u16 | 1000 (Builder-Vorgabe, Bedeutung unbekannt) |
| 0x20 | u32 | **Start in ms** (interne Cue `+0x10`, dieselbe Quelle wie XML `Start`) |
| 0x24 | u32 | **Loop-Ende in ms**, bei Cues 0xFFFFFFFF (interne Cue `+0x18`, wie XML `End`) |
| 0x28…0x34 | 4 × u32 | interne Cue `+0x20`, `+0x28`, `+0x24`, `+0x2C` (Inferenz: Frame-/MPEG-Positionen von Start/Ende) |

- Typ-1-Liste: Speicher für max(Anzahl, 3) Einträge; nur drei feste Slots → Inferenz: PCOB bildet höchstens 3 Hot Cues (A–C) ab, mehr erst über `PCO2` (U15).
- Der DAT-Default-Konstruktor legt 10 vorbelegte PCPT-Einträge im Speicher an; in den Dateien sind die PCOB-Listen trotzdem leer (Abschn. 5a).

## 5c. ANLZ – EXT und 2EX aus dem Dekompilat

### Lesealgorithmus der Container [GH:0x14127b650 EXT, GH:0x1412779f0 2EX]
1. Datei öffnen; **PMAI** lesen [GH:0x1412703d0], dann **PPTH** [GH:0x141270520]; beide Pflicht, genau in dieser Reihenfolge.
2. Schleife, solange `pos + 12 ≤ Dateilänge`: 12 Bytes vorauslesen, zurück auf `pos`, `total_len = be_u32(@8)`.
   - `total_len == 0` → Abbruch, Fehler −8. `pos + total_len > Dateilänge` → Fehler −2.
   - Verteilung nach Kennung (Tabelle unten); danach Sprung auf `pos + total_len` (Header-Längen-Überschüsse werden so übersprungen).
3. Tags, die **nur beim ersten Vorkommen** ausgewertet werden: EXT `PWV3`, `PKEY`, `PQT2`, `PWV5`, `PWV4`; 2EX `PWV6`, `PWV7`, `PVDI`.
   Weitere Vorkommen sowie **alle unbekannten Tags** gehen an [GH:0x14127da50]: der komplette Tag (inkl. Header)
   wird als Rohbytes an einen Puffer angehängt → **unbekannte Tags bleiben erhalten** (Round-Trip-fähig).
4. `PCOB` und `PCO2` werden bei jedem Vorkommen ausgewertet (je eine Hot-Cue- und eine Memory-Liste, siehe Typfeld).
5. Der Zugriff ist durch eine Critical Section geschützt (Inferenz: Reader-Objekt wird threadübergreifend genutzt).

### Felder je Tag (Offsets ab Tag-Beginn, alles Big-Endian)

| Tag | Reader / Default | header_len (geprüft) | Felder | Nutzdaten |
|---|---|---|---|---|
| `PWV3` | GH:0x14127b990 / GH:0x14127ac90 | 0x18 | `u16@0C`, `u16@0E` (Default 1), **`u32@10` = Anzahl Bytes**, `u16@14` (Default 150), `u16@16` | `u32@10` Bytes |
| `PWV4` | GH:0x14127cfd0 / GH:0x14127acc0 | 0x18, `u16@0C`=0 Pflicht | **`u16@0E` = Eintragsgröße** (Default 6), **`u32@10` = Anzahl**, `u16@14`, `u16@16` | Größe × Anzahl |
| `PWV5` | GH:0x14127d240 / GH:0x14127acf0 | 0x18, `u16@0C`=0 Pflicht | `u16@0E` = Eintragsgröße (2), `u32@10` = Anzahl, `u16@14` (150), **`u8@16`, `u8@17`** (Default 3, 5) | Größe × Anzahl |
| `PQT2` | GH:0x14127cc00 / GH:0x14127ad20 | **0x38 oder 0x30**, `u16@0C`=0 Pflicht | `u16@0E`; **`u8@10`** (beob. 1/2), `u8@11`; **`u16@12` = Eintragsgröße** (2); `u16@14`, `u16@16`; `u16@18`,`u16@1A`,`u32@1C`; `u16@20`,`u16@22`,`u32@24`; **`u32@28` = Anzahl**; `u32@2C`; bei 0x38 zusätzlich `u32@30`, `u32@34` | Größe × Anzahl |
| `PVB2` | GH:0x14127d4a0 / GH:0x14127ad60 | 0x20, `u16@0C`=0 Pflicht | `u16@0E`; **`u64@10`** (Inferenz: Gesamtzahl Samples); **`u32@18` = Anzahl** (400); **`u32@1C` = Eintragsgröße** (20) | Anzahl × Größe |
| `PSSI` | GH:0x14127d700 / GH:0x14127ad90 | **0x20 Pflicht**, `u16@0C`=0, **`u16@0E` = 0x18 Pflicht** | **`u16@10` = Anzahl**, `u16@12`; `@14..@19` werden **nicht gelesen** (auf 0 gesetzt); `u16@1A`; `u8@1C`, `u8@1D` | **Prüfung `total_len == 0x20 + 24·Anzahl`**, sonst −2; Einträge als `be_u16`-Folge (12 je Eintrag) |
| `PCOB` (EXT) | GH:0x14127bc10 / GH:0x14127abf0 | 0x18 | wie DAT (Abschn. 5b); Default `u16@0E` = Typ-Parameter | PCPT-Einträge |
| `PCO2` | GH:0x14127c070 / GH:0x14127ac30 | **0x14 Pflicht**, `u16@0C`=0 | **`u16@0E` = Typ**: 1 → Hot-Cue-Liste, 0 → Memory-Liste, sonst −1; **`u16@10` = Anzahl**; `u16@12` | Anzahl × Eintrag (PCP2), Reader GH:0x14127c300 |
| `PKEY` | GH:0x14127b650 | – | nicht zerlegt | **ganzer Tag roh gespeichert** (nur erstes Vorkommen) |
| `PWV6` | GH:0x141277d20 | **0x14 Pflicht**, `u16@0C`=0 | `u16@0E` = Eintragsgröße (3), `u32@10` = Anzahl (1200) | Größe × Anzahl |
| `PWV7` | GH:0x141277f30 | **0x18 Pflicht**, `u16@0C`=0 | `u16@0E` = Eintragsgröße (3), `u32@10` = Anzahl, `u16@14`, `u16@16` | Größe × Anzahl |
| `PWVC` | inline in GH:0x1412779f0 | (beob. 0x0E) | `u16@0C`, `u16@0E`, `u16@10`, `u16@12` – beobachtet `0, 80, 80…100, 80…105` (1102 Varianten) | – (Inferenz: Skalierung je Band, U17) |
| `PVDI` | GH:0x141278280 | **0x18 Pflicht** | `u16@0C` (Flag, Bit 15 siehe unten), `u16@0E`, `u16@10`, **`u16@12` = Eintragsgröße**, **`u32@14` = Anzahl** | Größe × Anzahl; **in keiner Datei beobachtet** |

**PVDI-Verschleierung:** Ist Bit 15 von `u16@0C` gesetzt, wird die Nutzlast nach dem Lesen mit einer XOR-Verschleierung
umgewandelt (GH:0x1412bbb50, Schlüssel aus einem eingebetteten String abgeleitet) und das Flag gelöscht. Als
Schutzmaßnahme analog zum DB-Schlüssel wird das Verfahren **bewusst weder spezifiziert noch nachgebaut**; die
Implementierung reicht PVDI unverändert durch.

### PCP2-Builder [GH:0x141fc47e0] (Struktur-Offsets = Datei-Offsets, Inferenz analog zu den anderen Tags)
Kopf: `PCP2`, header_len 0x10, total_len 0x58 + Kommentar. `u16@0E` = Slot (wie PCPT), `u8@10` = Typ (wie PCPT `@1C`),
`u16@12` = PCPT `@1E` (1000), `u32@14` = Start ms, `u32@18` = Ende ms, `u8@1C` = Farbindex + 1 (nur < 8), `u8@1D` = 1,
`u16@1E`/`u16@20` = zwei Werte < 1000, `u16@24`/`u16@26` = Wertepaar (nur beide ≠ 0; Inferenz: Loop-Länge als Bruch),
**`u16@2A` = Kommentarlänge**, Kommentar als UTF-16 angehängt (total_len wächst um die Länge),
**`u8@2C` = Hot-Cue-Farbe als Palettenindex 1…64, `u8@2D/2E/2F` = R/G/B** (aus der Palette bzw. Standardfarbe des Slots),
`@30…@57` = optionale Zusatzdaten (nur wenn die Quelle sie hat). Der Reader (unten) liest das Kommentar-Längenfeld bei
Objekt-Offset 0x32 = Datei-Offset 0x2A (Objekt beginnt mit 8-Byte-vtable-Zeiger) → konsistent.

### PCP2-Einträge (Inhalt von PCO2) [GH:0x14127c300, Header-Reader GH:0x14127c750]
Pro Eintrag: Header (GH:0x14127c750, nicht weiter zerlegt), dann ein **`u16`-Längenfeld + UTF-16BE-Kommentar**
(Inferenz: Cue-Kommentar), danach bis zu 0x2C Bytes: `4 × u8`, `4 × u64`, `2 × u32`; ein darüber hinausgehender Rest
wird roh erhalten. In allen beobachteten Dateien ist PCO2 leer → geringe Priorität (U15).

### Abgleich Beobachtung ↔ Code (EXT/2EX)
- Alle beobachteten Header-Längen bestehen die Pflichtprüfungen der Reader (PWV3/4/5 0x18, PQT2 0x38, PVB2 0x20, PSSI 0x20, PCO2 0x14, PWV6 0x14, PWV7 0x18).
- Die Beobachtung `PSSI w1 = count<<16 | 1` ist jetzt erklärt: `u16@10` = Anzahl, `u16@12` = 1.
- Die Beobachtung `PWV5 w2 = 0x00960305` ist erklärt: `u16@14` = 150, `u8@16` = 3, `u8@17` = 5.
- Beispiel PQT2 (Inferenz): `@18..@1F` = (1, 14898, 3) und `@20..@27` = (4, 14898, 126867) → gleiche Struktur
  wie ein PQTZ-Eintrag (Beat, Tempo×100, Zeit ms), vermutlich erster und letzter Beat.

## 5d. ANLZ – Feldsemantik (PSSI, Wellenformen, Cue-Listen)

### PSSI – Song-Struktur / Phrasen
**Header** (ergänzt zu 5c; Statistik über 1794 Dateien [OBS], Code [GH:0x14127d700, 0x1412c2ba0]):

| Feld | Beobachtung | Deutung |
|---|---|---|
| `u16@10` | 1…n | Anzahl Einträge (Code) |
| `u16@12` | nur 1, 2, 3 (1553 / 235 / 6) | **mood** (Inferenz: gleicher Wertebereich wie Parameter `mood` der Label-Funktion unten) |
| `@14..@19` | immer 0 | vom Reader A ignoriert; Reader B behandelt `@14` als u32, `@18` als u16 |
| `u16@1A` | immer > letzter Eintrags-Beat (1794/1794) | **End-Beat** (Inferenz) |
| `u8@1C` | 0 oder 1 | unbekannt (Flag) |
| `u8@1D` | immer 1 | unbekannt |

**Eintrag** (24 Bytes = 12 × `be_u16`, kein XOR/keine Maskierung – Reader B kopiert nur mit Byte-Swap [GH:0x1412bb7e0]).
Feldzuordnung belegt durch die Kopierroutine **[GH:0x1412812a0]** (EXT-PSSI → Player-Struktur) und die Umrechnung
**[GH:0x141919ef0]** (`djplay::SongStructInformation`); Namen der Felder sind Inferenz aus ihrer Verwendung:

| u16-Index | Beobachtung | Verwendung im Code | Name (Inferenz) |
|---|---|---|---|
| 0 | genau 1…n (1794/1794) | nicht kopiert | `index` |
| 1 | streng steigend | → Start-Beat (−1 → 0-basiert); Phrasensuche nach Beat [GH:0x141918810] | `beat` |
| 2 | 1…10 | → Art (−1), Eingang der Label-Umrechnung | `kind` |
| 3 | 0/1 | bool → Varianten-Flag (mood 1: INTRO/CHORUS/OUTRO 1 vs 2) | `k1` |
| 4 | 0/1 | bool → Varianten-Flag (mood 1: UP 3) | `k2` |
| 5 | 0/1 | u16 (−1), nur durchgereicht | `b` (unklar) |
| 6, 7, 8 | 0 oder Beats; `[6]≠0` genau wenn `[4]=1`, `[7],[8]≠0` genau wenn `[5]=1` | u16 (−1), nur durchgereicht | `beat2…beat4` (unklar) |
| 9 | 0/1 | bool → Varianten-Flag (mood 1: UP 2) | `k3` |
| 10 | 0/1 | bool → **Fill-in vorhanden** | `fill` |
| 11 | Beat | → **Fill-in-Start-Beat** (−1); 5701/5703: liegt 1…4 Beats vor der nächsten Phrase | `fill_beat` |

Header-Zuordnung durch dieselbe Kopierroutine: `u16@12` → mood, `u16@10` → Anzahl, `u8@1C` / `u8@1D` → bool,
`u16@1A` → End-Beat (−1).

**Label-Umrechnung im Player [GH:0x141919ef0]** – `k = kind − 1`; Tabellen werden zur Laufzeit initialisiert
[GH:0x140566e00 (mood 1), 0x140566ee0 (mood 2), 0x140566fc0 (mood 3)]:

| mood | Regel `k (+Flags) → Label` |
|---|---|
| 1 (High) | k=0: `k1` ? INTRO 1 : INTRO 2 · k=1: `k2` ? UP 3 : (`k3` ? UP 2 : UP 1) · k=2: DOWN · k=3: UP 2 · k=4: `k1` ? CHORUS 1 : CHORUS 2 · k=5: `k1` ? OUTRO 1 : OUTRO 2 · sonst: kein Label |
| 2 (Mid) | k=0…9 direkt: INTRO, VERSE1, VERSE2, VERSE3, VERSE4, VERSE5, VERSE6, BRIDGE, CHORUS, OUTRO · sonst: kein Label |
| 3 (Low) | k=0: INTRO · 1–3: VERSE 1 · 4–6: VERSE 2 · 7: BRIDGE · 8: CHORUS · 9: OUTRO · sonst: kein Label |

Bei mood 3 wird die Art dabei auf den Tabellenindex normiert (0–5). Fill-in: Liegt ein Beat im Bereich
`[fill_beat, nächster Phrasen-Beat)`, liefert der Player den Schritt `beat − fill_beat + 1` (1…d, d = Fill-Länge);
bei d > 4 liefert er 0 [GH:0x141918810; Fallunterscheidung d = 1…4 nachgerechnet].

**Gegenprobe [OBS]:** Die Regeln ordnen alle 23 431 Einträge der 1794 Dateien zu, keiner bleibt ohne Label.
Häufigste Labels: mood 1 CHORUS 1 (7279), UP 1 (4313), DOWN (2399); mood 2 CHORUS (1136); mood 3 CHORUS (59).

**Lighting-Modul (separat)** [GH:0x14132bc70] – eigene Label-Funktion `label(mood, k)` mit 1-basiertem `k` für
die Lichtsteuerung nach Phrasen (Nachbarn: `MoodLastId`, `BankLastId`, `PhraseLastId`); Tabellen:

| mood | max | Labels für k = 1, 2, 3, … |
|---|---|---|
| 1 | 11 | INTRO 1, INTRO 2, UP 1, UP 2, UP 3, CHORUS 1, CHORUS 2, DOWN, OUTRO 1, OUTRO 2, Fill IN |
| 2 | 10 | INTRO, VERSE 1, VERSE 2, VERSE 3, VERSE 4, VERSE 5, VERSE 6, CHORUS, BRIDGE, OUTRO |
| 3 | 6 | INTRO, VERSE 1, VERSE 2, CHORUS, BRIDGE, OUTRO |
| sonst | – | leerer String |

Achtung: In der Lighting-Tabelle steht bei mood 2 CHORUS vor BRIDGE, in der Player-Tabelle umgekehrt. Die
Lighting-Werte `k` stammen aus der Lighting-Datenbank (`phrase_data`, Initialisierung GH:0x141331fb0), nicht direkt aus
PSSI. Für das ANLZ-Format ist die **Player-Umrechnung** maßgeblich.

### Wellenformen – Bit-Layout
| Tag | Eintrag | Layout | Beleg |
|---|---|---|---|
| `PWAV`, `PWV3` | 1 Byte | **Bit 7–5 = Wert w (0…7)**, **Bit 4–0 = Höhe h (0…31)** | Generator GH:0x1412b9f10: `dst = src[4]·0x20 + (src[0] & 0x1F)` |
| `PWV2` | 1 Byte | Bit 4–0 = Höhe (beob. 0…15), Bit 7–5 immer 0 | [OBS] |
| `PWV5` | `be_u16` | **Bit 15–13 = c0, 12–10 = c1, 9–7 = c2 (je 0…7), Bit 6–2 = Höhe (0…31), Bit 1–0 = 0** | Generator GH:0x1412bd6e0 (Byte-Swap von `v<<2`, `v = c0<<11 \| c1<<8 \| c2<<5 \| h`); Bit 1–0 = 0 in 5 940 606/5 940 606 Einträgen |
| `PWV4` | 6 Byte | b0, b1: 0…255; b2…b5: 0…127 | [OBS] – Semantik unbekannt (U14) |
| `PWV6` | 3 Byte | je 0…~103 | [OBS] – Inferenz: 3 Bänder, Vorschau |
| `PWV7` | 3 Byte | je 0…127 (7 Bit) | [OBS] – Inferenz: 3 Bänder, Detail |

- **Gegenprobe:** Die Höhen-Verteilung von PWV5 (h=0: 502 075, h=31: 382 432) entspricht fast genau der von PWV3
  (h=0: 508 419, h=31: 383 007) → PWV3 (einfarbig) und PWV5 (farbig) tragen dieselbe Höhenfolge bei 150 Einträgen/s.
- Ob c0/c1/c2 Rot/Grün/Blau bzw. w ein Weiß-/Intensitätswert ist, ist **Inferenz** – der Code packt nur die Bits (U14).

### Cue-Listen (PCOB / PCPT) – Kapazität und Header
- Konstruktor GH:0x14126f870: Typ 1 → Puffer für **3** PCPT-Einträge (0xA8), Typ 0 → **10** (0x230); jeder Eintrag
  vorbelegt mit `PCPT`, header_len 0x1C, total_len 0x38, `u16@0C` = 0.
- Export-Writer GH:0x1412bf630: PCOB `u16@0E` = 1, `u16@10` = 0, **`u16@12` = Anzahl**, `u32@14` = 0xFFFFFFFF,
  **`total_len = 0x18 + 0x38 · Anzahl`**; Einträge werden als fertige 56-Byte-Blöcke kopiert.
- PCPT-Feldsemantik: nicht im ANLZ-Code bestimmbar (entsteht in der Cue-Verwaltung der DB); da rekordbox 7 lokal
  nur leere Listen schreibt, niedrige Priorität (U15).

### Abgleich Beobachtung ↔ Code (DAT)
Alle Header-Längen (PMAI 28, PPTH 16, PVBR 16, PQTZ 24, PWAV 20, PWV2 20, PCOB 24) und Default-Werte aus dem
Konstruktor 0x14126f190 stimmen mit den 1831 beobachteten `.DAT`-Dateien überein. Keine Widersprüche gefunden.

## 5e. rekordbox.xml – Bibliotheks-Austauschformat (Dekompilat)

Keine Beispieldatei auf dem System vorhanden → Spezifikation **nur aus Code** [GH] (Dekompilat und, wo der Decompiler
unscharf war, Assembly), ohne Beobachtungs-Gegenprobe.
Modul: Writer „Export Collection in xml format“ [GH:0x141eb6750], Reader „Loading XML library“ [GH:0x141eb11e0].
XML-Bibliothek ist ein JUCE-artiger DOM (Elemente mit geordneten Attribut-Listen).

### Dokumentaufbau (Writer)
```
<?xml version="1.0" encoding="UTF-8"?>                       [GH:0x141eb6b20, Strings 0x143cac208/0x143cac2f0]
<DJ_PLAYLISTS Version="1.0.0">                                [GH:0x141eb6cc0; "1.0.0" GH:0x140bfd660]
  <PRODUCT Name="…" Version="…" Company="AlphaTheta"/>        [GH:0x141eb6db0; Name/Version = Aufrufer-Parameter]
  <COLLECTION Entries="n"> TRACK* </COLLECTION>               [GH:0x141eb7140; n = Anzahl exportierter Tracks]
  <PLAYLISTS>                                                 [GH:0x141eb73c0]
    <NODE Type="0" Name="ROOT" Count="k"> NODE* </NODE>
  </PLAYLISTS>
</DJ_PLAYLISTS>
```
- Ordner: `<NODE Name="…" Type="0" Count="k">` + k Kind-NODEs (rekursiv) [GH:0x141eb8ca0].
- Playlist: `<NODE Name="…" Type="1" KeyType="0" Entries="m">` + m × `<TRACK Key="TrackID"/>` [GH:0x141eb8ca0].
- COLLECTION überspringt Tracks, für die GH:0x141f3e6f0 wahr liefert (Inferenz: nicht exportierbare Einträge).

### TRACK-Attribute (Writer-Reihenfolge) [GH:0x141eb76d0]
| Attribut | Wert / Format |
|---|---|
| `TrackID` | Ganzzahl (interne ID) |
| `Name`, `Artist`, `Composer`, `Album`, `Grouping`, `Genre` | Text |
| `Kind` | Formatcode → Text: 1 `MP3 File`, 2 `AAC File`, 3 `MP4 File`, 4 `M4A File`, 5 `FLAC File`, 6 `ALAC File`, 11 `WAV File`, 12 `AIFF File`, sonst `Unknown Format` |
| `Size` | Ganzzahl (Inferenz: Bytes) |
| `TotalTime` | Ganzzahl (u16), Sekunden (Reader-Vergleich, siehe unten) |
| `DiscNumber`, `TrackNumber`, `Year` | Ganzzahl |
| `AverageBpm` | `BPM×100 · 0.01`, Format `%.02f` (Konstante 0.01 @0x1457ff5d8) |
| `DateAdded` | Text, unverändert aus der DB (Format unbekannt, U20) |
| `BitRate`, `SampleRate` | Ganzzahl |
| `Comments` | Text |
| `PlayCount` | Ganzzahl |
| `Rating` | **Sterne (0–5) × 51** → 0, 51, 102, 153, 204, 255 |
| `Location` | URL, siehe unten |
| `Remixer`, `Tonality`, `Label`, `Mix` | Text (`Tonality` = Tonart als Text) |
| `Colour` | nur bei Farbindex 1–8: `0xFF007F`, `0xFF0000`, `0xFFA500`, `0xFFFF00`, `0x00FF00`, `0x25FDE9`, `0x0000FF`, `0x660099`; bei 0 entfällt das Attribut |

Danach folgen als Kind-Elemente TEMPO* und POSITION_MARK* (nur wenn Analysedaten vorhanden, GH:0x141f33670).

### TEMPO (Beatgrid) [GH:0x141eb81b0]
`<TEMPO Inizio="s" Bpm="b" Metro="4/4" Battito="n"/>`
- Quelle: Beatgrid-Einträge `(beat u16, tempo×100 u16, time_ms u32)` – gleiches Format wie ANLZ `PQTZ`.
- `Inizio` = `time_ms · 0.001`, `%.03f`; `Bpm` = `tempo · 0.01`, `%.02f`; `Metro` fest `"4/4"`; `Battito` = Beat (1–4).
- Einstellung **`bridgeExportBeatGrid`**: 0 → kein TEMPO; 2 → ein TEMPO pro Beat; 1 → nur bei Tempowechsel oder wenn der
  Beat nicht auf dem aus dem vorigen Tempo erwarteten Raster liegt (Erwartung `t + 6 000 000 / tempo`, Toleranz 0,5 ms).

### POSITION_MARK (Cues/Loops) [Writer GH:0x141eb8710, Reader GH:0x141eb1d20]
`<POSITION_MARK Name="…" Type="t" Start="s" [End="s"] Num="n" [Red Green Blue]/>`
- `Type` = 0 (Cue) oder 4 (Loop, wenn End ≠ 0); `Start`/`End` = ms · 0.001, `%.03f`.
- `Num` aus interner Cue-Art: 1, 2, 3 → 0, 1, 2 (Hot Cue A–C); 5…9 → 3…7 (D–H); −1, 0, 4 → −1 (Memory Cue).
- Nur bei `Num` < 8: `Red`, `Green`, `Blue` (0–255) aus der Hot-Cue-Farbe [GH:0x14120b760].
- **Reader-Regeln:** `,` in Zahlen wird als `.` gelesen; Eintrag wird verworfen, wenn `Type ∉ {0,4}`, `Start` außerhalb
  `[0, TotalTime]`, bei Loop `End < Start` oder `End > TotalTime`, oder `Num ∉ [−1, 7]`. Umkehrabbildung `Num → Art`:
  −1 → 0 (Cue) bzw. −1 (Loop); 0–2 → Num+1; 3–7 → Num+2. Position zusätzlich in Wellenform-Frames: `ms · 150 / 1000`.
  Farbe: nur wenn `Red`, `Green`, `Blue` alle vorhanden sind (Default −1) und `Num` ∈ 0…7 → Palettenindex mit dem kleinsten
  euklidischen RGB-Abstand unter den Indizes 1…64; bei Gleichstand gewinnt der erste Index.
- **Hot-Cue-Palette** [Initialisierung GH:0x140c9c4d0, Zugriff GH:0x1420426d0; Werte aus der Assembly extrahiert,
  `analysis/out/palette.txt`]: 64 RGB-Werte, Index 1 = `#305AFF` … 62 = `#6473FF`, 63 = `#000000`, 64 = `#FFFFFF`;
  Indizes außerhalb 1…64 liefern Eintrag 0 = `#000000`. Vollständige Tabelle: `HOT_CUE_PALETTE` in `src/xml/mod.rs`.

### TEMPO-Import (Beatgrid-Expansion) [GH:0x141eb1620]
Der Reader erzeugt aus den TEMPO-Ankern ein vollständiges Beatgrid (Einträge wie `PQTZ`). Konstanten: 1000, 30, 1, 60, 0.5.
1. Pro TEMPO: `Inizio` (s), `Bpm`, `Battito`, Zähler `num` = ganzzahliger Text von `Metro` vor dem `/` (ohne `/` → 0).
2. **Auffüllen vor dem Anker:** Waren BPM, Beat und num des vorigen Ankers > 0 und liegt `Inizio` hinter dem letzten Beat,
   werden Beats im Abstand `60/BPM` erzeugt (`beat = beat % num + 1`), bis der nächste Beat nach `TotalTime` läge oder der
   Anker höchstens einen halben Beat (`30/BPM`) entfernt ist.
3. **Anker selbst:** übernommen, wenn BPM, Battito, num > 0 und `letzter Beat < Inizio ≤ TotalTime`; der allererste Eintrag
   darf auch bei `Inizio = 0` stehen. Die Zustandswerte (BPM, Beat, num) werden auch dann übernommen, wenn der Anker
   verworfen wird (Eigenheit: ein Anker hinter `TotalTime` steuert trotzdem die Schlussauffüllung).
4. **Schlussauffüllung** nach dem letzten Anker bis einschließlich `TotalTime`.
5. Tempo = `(int)(BPM·100 + 0.5)`, Zeit = `(int)(s·1000 + 0.5)`.

### Weitere Reader-Umrechnungen [GH:0x141eb26d0]
- `AverageBpm` → `(int)(Wert · 100.0)` – **abgeschnitten**, nicht gerundet (Konstante 100.0 @0x1457ffc28).
- `SampleRate` wird als Fließkommazahl gelesen (mit `,`-Toleranz) und abgeschnitten; `Rating` → `Wert / 51`.
- `TotalTime` ist in **Sekunden**: Der Reader vergleicht Cue-Startzeiten in Sekunden direkt mit `TotalTime`.

### Location-Kodierung [Writer GH:0x141eb91b0, Reader GH:0x141eb47b0]
- **Writer:** Pfad als UTF-8-Bytes; erhalten bleiben alphanumerische ASCII-Zeichen (GH:0x142a6f780) und
  `! # $ ( ) * + , - . / : ; = ? @ _ ~`; alle anderen Bytes → `%` + **zwei Kleinbuchstaben-Hexziffern**;
  `file://localhost/` + Ergebnis. Der Pfad wird sonst nicht verändert (kein `/` entfernt oder ergänzt).
- **Reader:** (1) `%xx` dekodieren (Hex in beiden Schreibweisen; ungültige Sequenzen bleiben stehen), (2) Präfix
  `file://localhost/` entfernen [GH:0x142a69790], (3) **`\` → `/`** [GH:0x141e47970]. → rekordbox speichert Pfade intern
  mit `/` (z. B. `C:/Music/a.mp3`).

### XML-Escaping [GH:0x141eb9bf0, Assembly + Sprungtabelle @0x141eb9dd8]
Zeichen werden als UTF-8 dekodiert und so behandelt: `"` → `&quot;`, `&` → `&amp;`, `'` → `&apos;`, `<` → `&lt;`,
`>` → `&gt;`; `\n`/`\r`: bei gesetztem Flag **entfernt**, sonst unverändert; alle übrigen Zeichen < 0x20 **entfernt**;
Zeichen ≥ 0x80 als UTF-8. **Attributwerte** werden mit Flag = 1 geschrieben [GH:0x141eb63d0 @0x141eb64c7], Textinhalt
mit Flag = 0.

### Ausgabeformat [GH:0x141eb6b20, Element-Writer GH:0x141eb63d0]
- `<?xml version="1.0" encoding="UTF-8"?>` + 2 × Zeilenende; danach (falls gesetzt) DTD + Zeilenende; dann das
  Wurzelelement mit Einrückung 0 und **Umbruchgrenze 60**; abschließend ein Zeilenende.
- Zeilenende = **`\r\n`** (Standardwert des Ausgabestroms, Bytes `0d 0a` @0x14560c95c).
- Element: Einrückung (Leerzeichen), `<Name`; für jedes Attribut: übersteigen die bereits in dieser Zeile geschriebenen
  Attribut-Bytes 60, folgt Zeilenende + `Einrückung + 1 + Länge(Name)` Leerzeichen; dann ` Name="Wert"`.
  Ohne Kinder `/>`, sonst `>`, jedes Kind-Element auf neuer Zeile mit Einrückung + 2, danach Zeilenende + Einrückung +
  `</Name>`.

## 5f. MIDI-Mapping-CSV (`MidiMappings/*.midi.csv`)

Belege: 69 mitgelieferte Dateien [OBS] und der Loader im Dekompilat/der Assembly [GH:0x1424c4a50 Datei,
0x1424c3890 Zeile, 0x1424c35c0 Optionen, 0x1424c3c80 Interpretation, 0x1424c27c0 Typen].

### Datei [GH:0x1424c4a50]
- Zeilenweise; **leere Zeilen und Zeilen, die mit `#` beginnen, werden übersprungen** (z. B. die Spaltenzeile
  `#name,function,type,input,deck1,…` und deaktivierte Einträge wie `#JogScratch,…`).
- Kopfzeile: jede (nicht mit `#` beginnende) Zeile, die **`@file` enthält** – geprüft per Teilstring-Suche
  (`indexOf ≥ 0`, GH:0x142a6b350 → 0x140ef6df0), nicht als Präfix. An `,` zerlegt: `version` = Ganzzahl (Feld 1),
  `gerät` = Text (Feld 2). Eigenheit: eine Datenzeile, die `@file` enthält, wird ebenfalls als Kopfzeile gelesen.
- Datenzeilen werden nur verarbeitet, solange `version == 1`; die Datei gilt als gültig, wenn `version == 1`.
- [OBS] 6 der 69 Dateien beginnen mit einem UTF-8-BOM vor `@file`. Der Zeilenleser entfernt es nicht
  [GH:0x142a713e0]; wegen der Teilstring-Suche funktioniert die Kopfzeile trotzdem (BOM steht nur in Feld 0).

### Zeile [GH:0x1424c3890]
- Zerlegung an `,` **ohne Anführungszeichen-Behandlung**; mindestens **15 Felder**, sonst verworfen; Feld 0 darf nicht
  leer sein. Weitere Felder werden ignoriert.
- Spalten nach Position: `Name, Function, Type, Input, InputDeck1–4, Output, OutputDeck1–4, Option, Comment`
  (`Name` = rekordbox-Funktion, `Function` = Bedienelement am Controller).
- Zeilen, deren `Name` ein `@` enthält, verarbeitet diese Funktion nicht (Sonderzeilen) [GH:0x1424c3c80].

### Optionen [GH:0x1424c35c0]
`Option` an `;` zerlegen; je Teil: Schlüssel = Text vor dem ersten `=`, Wert = Text danach (fehlt er: `"1"`).
Bekannte Schlüssel: `RO`, `Fast`, `Blink`, `Value`, `Min`, `Max`, `Priority`, `Dual`. Verwendet als: `RO`, `Fast`,
`Dual` = Ganzzahl ≠ 0; `Blink`, `Priority` = Ganzzahl (fehlend 0); `Value`, `Min`, `Max` = Ganzzahl oder −1, wenn fehlend.
[OBS] 35 Kombinationen, z. B. `Fast`, `RO;Priority=100`, `Fast;Dual`, `Blink=600`, `RO;Min=1;Max=72;Priority=100`.

### Zahlen
- Hex (`Input`, `Output`, Deck-Spalten bei leerem Input/Output) [GH:0x140ef6f00]: alle Hex-Ziffern werden der Reihe
  nach angehängt (`v = v·16 + d`), **alle anderen Zeichen übersprungen**; Ergebnis als 16 Bit verwendet.
- Dezimal (Deck-Kanäle, Optionswerte) [GH:0x1429b15cc]: `strtol` Basis 10 (führende Leerzeichen, Vorzeichen,
  Abbruch beim ersten Nicht-Ziffer-Zeichen; leer → 0).

### Nachrichten je Deck [GH:0x1424c3c80]
- `Input` = Hex-Wert (Statusbyte·256 + Datenbyte, z. B. `B000` = Control Change Kanal 0, Controller 0; `9646` = Note On).
- **`Input` = 0 (leer):** jede nicht-leere `InputDeckN` ist eine vollständige Hex-Nachricht für Deck N.
- **`Input` ≠ 0:** jede nicht-leere `InputDeckN` ist eine Dezimalzahl k (Kanal): Nachricht = `(k << 8) | Input`;
  Ausnahme Typ `JogTimestamp` (0x19): `k | Input`.
- Ist keine Deck-Spalte belegt, aber `Input` ≠ 0: eine Zuordnung für Deck 0 (global) mit `Input`.
- `Output`/`OutputDeckN`: dieselbe Regel (ohne JogTimestamp-Ausnahme); der Fallback „Deck 0“ ist für Output nicht
  im Detail geprüft (Inferenz).
- **`Dual`:** jedes Deck erhält zusätzlich die Nachricht seines Partnerdecks (1↔3, 2↔4); ohne `Dual` ist die zweite
  Nachricht gleich der ersten.

### Registrierung [GH:0x1424c4e60]
- Abgewiesen: Deck-Index > 4 und Typ-Codes 9–11.
- **14-Bit-Steuerelemente** (`Slider` 4, `Knob` 5, `KnobSliderHiRes` 7/8): neben der Nachricht m wird zusätzlich
  **m + 0x20** registriert (LSB-Controller zum MSB-Controller, z. B. CC 0 + CC 32). Zuordnung über eine Tabelle, die
  mit der 16-Bit-Nachricht indiziert wird; eine bereits belegte Nachricht wird nicht überschrieben.

### Typen [GH:0x1424c27c0] (Text → interner Code)
`Button` 0, `Slider` 4, `Knob` 5, `KnobSlider` 6, `KnobSliderHiRes` 7 (8 bei Gerät `PIONEER DDJ-SX2` oder
`PIONEER DDJ-SX2 2IN2OUT`), `Difference` 0x0C, `Rotary` 0x0D, `RotaryAsKnob` 0x0E, `Value` 0x0F, `RawValue` 0x10,
`Indicator` 0x11, `IndicatorHiRes` 0x12, `JogIndicator` 0x13, `JogIndicatorHiRes` 0x14, `JogRotate` 0x17,
`JogRotateAbsPos` 0x18, `JogTimestamp` 0x19, `JogTouch` 0x1A, `Toggle` 0x1B, `Parameter` 0x1C,
`DdjRzPad` 0x1D, `DdjRxPad` 0x1E, `DdjSzPad` 0x1F, `DdjSx2Pad` 0x20, `DdjSxPad` 0x21; **unbekannter Text → 0 (Button)**.
`Pad` wird nach Gerät aufgelöst (Vergleich mit dem `@file`-Gerätenamen):
- `DdjRzPad`: DDJ-RZ, DDJ-RZX
- `DdjRxPad`: DDJ-RX, DDJ_RX 2IN2OUT, DDJ-RR, XDJ-XZ (1), XDJ-AZ, Pioneer DJ XDJ-RX2, XDJ-RX3, OPUS-QUAD, DDJ-XP1,
  DDJ-XP2, DDJ-1000, DDJ-800, DDJ-FLX10, DDJ-REV5, DDJ-REV7, DJM-S11, DJM-S7, PLX-CRSS12
- `DdjSzPad`: DDJ-SZ · `DdjSx2Pad`: PIONEER DDJ-SX2, PIONEER DDJ-SX2 2IN2OUT, Pioneer DJ DJM-S9
- alle anderen Geräte: `DdjSxPad`

[OBS] Verwendete Typen in den 69 Dateien: Pad 5868, Button 5190, KnobSliderHiRes 899, Indicator 381, Rotary 270,
KnobSlider 232, Value 215, JogRotate 154, Parameter 140, JogTouch 78, Difference 39, JogIndicator 25, Slider 6,
IndicatorHiRes 5, RotaryAsKnob 4, leer 4, JogIndicatorHiRes 3, RawValue 3, JogRotateAbsPos 1, JogTimestamp 1.

## 5g. Performance-Pad-Dateien (`pad/*.pad.csv`, `pad/Version`)

Belege: 36 mitgelieferte Dateien [OBS]; Zeilen-Parser [GH:0x141c16c30], Modusliste [GH:0x141c10020],
Benutzervarianten [GH:0x141c0ecf0].
- `pad/Version` enthält die Versionsnummer als Text (`1.7`).
- Jede Datei: zeilenweise; **leere Zeilen und Zeilen mit `#` am Anfang werden übersprungen** [GH:0x141c10020, `#` @0x143b29c7c].
- Zeile = ein Pad: an `,` zerlegt (keine Anführungszeichen); je Teil Schlüssel = Text vor dem ersten `=`, Wert = Text
  danach (ohne `=`: ganzer Teil = Schlüssel, Wert leer); spätere gleichnamige Schlüssel überschreiben frühere.
- [OBS] Drei Zeilenformen in 559 Zeilen: `function,value` (489), `function,value,userfunction,uservalue` (32),
  `function,type,mode,template,pad,text,select` (37, nur `Mode.pad.csv`).
- **Benutzervariante:** Sind `userfunction` und `uservalue` beide nicht leer, werden beim Übernehmen eines Pads
  `function := userfunction` und `value := uservalue` gesetzt [GH:0x141c0ecf0].
- **`Mode.pad.csv`** = Liste der Pad-Modi: `type` ∈ {`separator`, `preset`, …}, `mode`/`template`/`pad` = `on`/`off`,
  `text` = Anzeigename, `select` = Auswahlnummer (leer möglich). Je Modus legt rekordbox 16 Pad-Plätze an [GH:0x141c10020];
  die Pad-Datei eines Modus heißt `<function>.pad.csv`.
- [OBS] Benutzervarianten stehen nur in `BeatJump.pad.csv` (16 × `userfunction=UserBeatJump`) und `KeyShift.pad.csv`
  (16 × `userfunction=UserKeyShift`). Der Modus `UserBeatJump` hat **keine eigene Datei** (`UserKeyShift.pad.csv`
  existiert) → Inferenz: die User-Modi werden aus der Basisdatei über die Benutzervariante abgeleitet.
- Validierung mit `rbf pad` [OBS]: 37 Modi, 35 Pad-Dateien, 558 Pads, alle Write/Parse-Round-Trips identisch.

## 5h. DeviceSQL-Engine, `.edb`-Dateien und Stand `export.pdb`

### Engine [GH]
- Eingebettete, statisch gelinkte Datenbank-Engine (Fehlertexte `edb_dyn_Select`, `edb_dyn_SelectMaterialized`,
  `edb_dyn_Update`, `edb_dyn_Delete`, `edb_dyn_CountRowSet`, `edb_dyn_CriticalSectionOn/Off`, `dsql_get_free_id`,
  `ext_dsql_get_new_seqno`; Wrapper `db::dsql…` @0x1420b…). Code-Bereich ≈ 0x142660000–0x142690000.
- API am Beispiel `db::dsqlExistsGenre` [GH:0x1420b5320]: `CriticalSectionOn` (0x1426819b4) → Tabellen-Deskriptor
  (0x142265670, Tabelle 2 = Genre) → `Select` mit Filter-Rückruf (0x14267fe20) → nächste Zeile (0x14268050c) →
  Rowset schließen (0x1426801b0) → `CriticalSectionOff` (0x142681a24).
- Storage-Manager-Optionen (Strings im Engine-Bereich): `SM_DATA_MMAP`, `SM_DATA_PERS`, `SM_INDEX_PERS`, `PAGESZ`,
  `MAXPAGES`, `NBUFS`, `HDRSZ`, `FILESZ`, `VALIDATE`, `READ_ONLY`, `COMMIT_WRITE_ONLY`, `COMMITOPT`, `PGMGR`, `MULTITAB`.
  Prüfregeln des Seitenmanagers [GH:0x142678b98]: `PAGESZ` wird auf ein Vielfaches von 8 abgerundet und muss in
  0x2C…0xFFF0 liegen; `MAXPAGES` ∉ {1, 2, 3}; `NBUFS` = 0 → 64, sonst ≥ 4; `COMMITOPT` ∈ {0, 1}.
- Dateien: `datafile.edb`, `ExtData.edb`, `LocalData.edb`, `LocalDataExt.edb`, `export.pdb`, `exportExt.pdb`
  [Pfade GH:0x141f3a330]. Ein separater Dienst (`edb_agent_open` …, Bereich 0x14235…) beantwortet Browse-Anfragen
  von Playern im Netzwerk direkt aus `master.db`; der Rückruf GH:0x142376290 liefert dabei Texte zu Feld-IDs
  (1 Dateiname, 3 Titel, 4 Kommentar, 5 StockDate, 6 Genre, 7 Artist, 8 Album, 9 Label, 10 Tonart, 0x0B Farbe,
  0x0C Playlist, 0x0D History, 0x0E Hot-Cue-Bank, 0x0F Bildpfad, …).

### String-Kodierung [OBS: `.edb`; GH:0x142376290]
- **Kurz (ASCII):** Kennbyte `((n + 1) << 1) | 1`, danach n Bytes (n ≤ 126). Beispiel `0B 'Pink'`, `09 'Red'`.
- **Lang (UTF-16LE):** Kennbyte `0x90`, `u16` Gesamtlänge (inkl. 4-Byte-Kopf), 1 Füllbyte, dann die Code-Units.
- rekordbox umschließt UTF-16-Text mit den Markern **`0xFFFA` … `0xFFFB`** (Code baut Puffer `FFFA + Text + FFFB`).
- Weitere Kennbytes (z. B. lange ASCII-Strings) wurden nicht beobachtet und sind nicht spezifiziert.

### `.edb`-Container [OBS: `datafile.edb` 2864 B, `ExtData.edb` 3000 B]
- 0x38 Bytes Kopf (14 × `u32` LE): Wort 3 = **Dateigröße** (beide Dateien); übrige Wörter unbekannt.
- Ab 0x38: **nullterminierte `u16`-Tabelle** von Zeilen-Offsets, relativ zu **0x14**; jede Zeile reicht bis zur nächsten,
  die letzte endet genau am Dateiende (beide Dateien).
- Zeile: `u16` Tabellenkennung (untere 5 Bit 0), `u16` = laufende Zeilennummer × 0x20, dann tabellenspezifische Felder.
- Beobachtete Tabellen in `datafile.edb`: `0x02A0` (1 Zeile, String „1000“), **`0x0380` Farben** (8 Zeilen: ID in Byte 9,
  Name als kurzer String bei Byte 12 – Pink, Red, Orange, Yellow, Green, Aqua, Blue, Purple), **`0x04A0` Spalten/Sortierungen**
  (27 Zeilen: `u16` ID bei 4, `u16` bei 6, langer markierter String bei 8 – DEFAULT, ALPHABET, … REMIXER),
  `0x04C0` (22 Zeilen à 12 Bytes, Menü-Konfiguration, Felder unbekannt), `0x04E0` (1 Zeile, 1412 Bytes).
  `ExtData.edb`: `0x05E0` (30), `0x0560` (28), `0x0520` (3) mit Ordnernamen (z. B. „All Samples“, „Capture“).
- Validierung `rbf edb` [OBS]: beide Dateien vollständig gelesen, in allen 120 Zeilen ein sauber dekodierter String.

### Stand `export.pdb` (U23)
Engine, API, Storage-Optionen und Zeilen-/String-Kodierung sind belegt (oben). **Nicht** spezifiziert ist das
Seitenformat der persistenten Datei (Kopfseite, Seitenköpfe, Tabellenliste, Indexstrukturen) – der Seitenmanager ist
groß und ohne Beispieldatei nicht gegenprüfbar. Benötigt wird ein USB-/Ordner-Export aus rekordbox als Testdatei.

## 6. Bewusst ausgeschlossen (Begründung)

- **Lizenzierung / Kopierschutz** (`lsapiw64.dll`, `clatc.dll`, Aktivierung, Abo-Prüfung): wird weder analysiert noch nachgebaut oder umgangen.
- **Schlüssel der verschlüsselten DBs** (`master.db`): wird nicht extrahiert oder hart kodiert. Eine Reimplementierung darf höchstens einen vom Nutzer bereitgestellten Schlüssel entgegennehmen.
- **Cloud-/Account-Dienste** (Agent, Sync): keine Nachbildung von Server-Protokollen fremder Dienste.
- **ML-Modelle / Stem-Separation**: proprietäre Modelle werden nicht übernommen; eigene Algorithmen sind möglich.

## 7. Offene Punkte / Unknowns

| ID | Frage | Blocker |
|---|---|---|
| U1 | ~~Container-Struktur der ANLZ-Dateien~~ → **geklärt durch Beobachtung (Abschn. 5a)**; Code-Bestätigung ausstehend | Ghidra-Analyse läuft |
| U8 | Bedeutung der 16 konstanten PMAI-Header-Bytes | Dekompilat |
| U9 | PVBR: Layout geklärt (5b); offen: Semantik von Tabelle und extra | Nutzer der PVBR-Struktur |
| U10 | PWVC-Inhalt (6 Bytes) | Dekompilat |
| U11 | PQT2-Header-Felder w3…w8 und Bedeutung der 2-Byte-Einträge | Dekompilat |
| U12 | PSSI: Layout, mood, Label-Umrechnung, Fill-in geklärt (5d); offen: Bedeutung von `u16[5..8]` und `u8@1C/@1D` (nur durchgereicht) | Nutzer der djplay-Felder +0x18…+0x24 |
| U13 | Schlüssel d2…d12 im `.3EX`-Embedding | Dekompilat |
| U14 | Wellenformen: Bit-Layout PWAV/PWV3/PWV5 geklärt (5d); offen: Farbsemantik c0–c2/w, Layout PWV4, PWV6/7-Bandzuordnung | Renderer-Code |
| U15 | PCPT: Slot, Typ, Start/Ende geklärt (5b); PCP2-Builder geklärt (5c); offen: `u16@1E` = 1000, `@28…@34`, PCP2 `@30…@57` | Dekompilat der Cue-Struktur |
| U16 | PKEY: vom EXT-Code nur roh gespeichert [GH:0x14127b650], in keiner der 1827 `.EXT`-Dateien vorhanden | Nutzer der Rohdaten finden |
| U17 | PWVC-Werte (80…105) – Bedeutung | Nutzer im Wellenform-Renderer |
| U18 | PVDI: Header geklärt (5c); Inhalt verschleiert → bewusst nicht weiter untersucht | – |
| U19 | PQT2 `u32@2C`, `u32@30/34` und Bedeutung der 2-Byte-Einträge | Dekompilat der PQT2-Nutzer |
| U20 | rekordbox.xml: `TotalTime` = Sekunden geklärt (5e); offen: `DateAdded` = Text aus der DB (Inferenz: Spalte `StockDate`, GH:0x1420cb5a0), Datumsformat und Einheiten von `Size`/`BitRate` | eigener XML-Export als Testdatei |
| U21 | ~~Location-Umwandlung~~ → **geklärt** (5e: Reader `\`→`/`, Writer unverändert) | – |
| U22 | ~~Escaping~~ → **geklärt** (5e: Attribute ohne Zeichen < 0x20; Ausgabeformat CRLF, Umbruch 60) | – |
| U23 | `export.pdb`: Engine/Strings/Zeilen belegt (5h); offen: Seitenformat der persistenten Datei | USB-/Ordner-Export als Testdatei |
| U24 | ~~BOM vor `@file`~~ → **geklärt** (5f: Kopfzeile per Teilstring-Suche, BOM bleibt in Feld 0) | – |
| U25 | MIDI-CSV: Bedeutung der Typ-Codes 1–3, 9–11, 0x15, 0x16 (kein Text bekannt) und Semantik von `Fast`/`RO`/`Priority` | Nutzer der Mapping-Struktur |
| U2 | Aufbau `export.pdb` (Seiten, Tabellen, Zeilenformat) | Dekompilat nötig |
| U3 | Exakte rekordbox.xml-Attributsemantik (Einheiten, Kodierung von Farben/Ratings/Keys) | Dekompilat oder eigener Export als Testdaten |
| U4 | Rolle von `rbHttpServer.exe` / `edb_streamd.exe` | eigene Analyse jeder Binärdatei |
| U5 | Pro-DJ-Link-Protokoll (UDP-Pakete) | Dekompilat + autorisierte Laufzeitbeobachtung |
| U6 | Inhalt des Electron-Agents (`app.asar`) | REA-JS-Analyse (ohne Ghidra möglich) |
| U7 | Semantik von `PSvLinkSysMgrINI.xml`-Timern (Einheit ms?) | Dekompilat |

## 8a. Implementierung und Validierung (Stand 2026-10-16)

- Rust-Crate `rekordbox-formats` (siehe README): `anlz` (§5a–§5d) und `xml` (§5e), 32 Tests auf synthetischen Daten.
- Validierung mit `rbf check` gegen die 7307 lokalen ANLZ-Dateien [OBS]: 0 Fehler; 5484/5484 Container
  byte-identisch zurückgeschrieben; alle typisierten Tags identisch neu kodiert; 23 431/23 431 Phrasen mit Label;
  1823/1823 `.3EX` dekodiert.
- Nebenbefund: PVB2 `u64@10` = 5 600 864 bei einem ~127 s langen Track (44,1 kHz) → stützt die Inferenz „Anzahl Samples“.
- Modul `midi` (§5f), validiert mit `rbf midi` gegen die 69 mitgelieferten Mappings [OBS]: alle `version 1`,
  13 505 Einträge, 0 Fehler, 69/69 Write/Parse-Round-Trips identisch; 4 Einträge mit leerem `Type` (→ Button).

## 8. Werkzeug-Status (Grenzen dieser Analyse)

- REA 6.5.0 mit **Ghidra 12.1.4** (Provider `available`, seit 2026-10-16). Rust 1.99.1 installiert.
- Der Desktop-Client bricht MCP-Aufrufe nach ~60 s ab; die Ghidra-Erstanalyse von `rekordbox.exe` dauert länger.
  Deshalb läuft sie einmalig über die REA-CLI (`rea search … --snapshot analysis/rekordbox-7.2.19.ghidra.snapshot`,
  `REA_GHIDRA_STARTUP_TIMEOUT_MS`) – Dauer mit 12 GB Heap: 35 min. REA hält das Projekt aber nur flüchtig
  (Snapshot = 9,6 KB Metadaten), daher zusätzlich ein **persistentes Ghidra-Projekt** `analysis/ghidra/`
  (39 min, alle Standard-Analyzer, 12 GB Heap, 12 GC- / 6 JIT-Threads, `-max-cpu 16`).
  Abfragen über `analysis/rbq.sh` + `analysis/ghidra_scripts/RbQuery.java` (≈10 s pro Abfrage, Fourcc-Suche ≈80 s).
