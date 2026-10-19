# XDJ-RX3 Link-Export über USB – Protokoll-Spezifikation für die Rust-Implementierung

Stand: 2026-10-19 · Quelle: einmaliger USB-Mitschnitt (`mittschnitt_usb.pcapng`, USBPcap/Windows, 14,9 s, 53 397 Pakete) einer rekordbox-Sitzung mit einem Pioneer XDJ-RX3 im **Link-Export**-Modus (Pult spielt ab, Software liefert die Bibliothek).

Dieses Dokument ist die Arbeitsgrundlage für den Coding-Agent. Es trennt strikt:

- **[BEOBACHTET]** = direkt aus dem Mitschnitt belegt (Bytes liegen vor)
- **[HYPOTHESE]** = plausibel (Vergleich mit öffentlicher Doku/Treiber), im Mitschnitt nicht bewiesen → per Test prüfen
- **[OFFEN]** = im Mitschnitt nicht enthalten → neuer Mitschnitt nötig

Öffentliche Referenz für Pro DJ Link / dbserver: Deep Symmetry „dysentery" (https://djl-analysis.deepsymmetry.org). Namen von Nachrichtentypen daraus nur als Hinweis verwenden, die Bytes dieses Dokuments haben Vorrang.

---

## 1. Ziel und Grenzen

**Ziel:** Eine eigene Rust-Software tritt gegenüber dem XDJ-RX3 so auf wie rekordbox im Link-Export: Sie stellt Bibliothek (Metadaten, Waveforms, Audiodateien) bereit; das Pult lädt Tracks und spielt sie selbst ab (Audio kommt immer aus dem Pult).

**Nicht-Ziele / harte Grenzen:**
- Kein Umgehen, Entschlüsseln oder Nachbauen von rekordbox-Lizenzprüfung oder „Hardware Unlock". Im Mitschnitt gibt es dafür keinen Hinweis (siehe 2). Stößt der Agent auf eine Authentifizierung/Challenge, **stoppt er und berichtet**, statt sie zu knacken.
- Privater, nicht-kommerzieller Gebrauch (Interoperabilität mit dem eigenen Gerät).

---

## 2. Kryptografie: Befund

**Im Mitschnitt kommt keine Verschlüsselung oder Authentifizierung vor.** Alles ist Klartext.

- Kein TLS/SSH/PEM, keine Handshake-Muster.
- Hohe Entropie der NFS-Daten (~7,8 bit/Byte) = komprimiertes Audio. Beleg: gelesene Datei beginnt mit `fLaC`-Header (STREAMINFO, 44,1 kHz) – eine normale FLAC-Datei.
- Pro DJ Link, dbserver und NFS (`AUTH_UNIX`/`AUTH_NULL`) sind von sich aus unverschlüsselt.

**Einschränkung:** Der Mitschnitt deckt nur den Link-Export-Pfad ab. Der Controller-Modus (HID/MIDI, Pad/Jog/Waveform-Anzeige, „Hardware Unlock") ist **[OFFEN]** – dort könnte es eine Freischaltung geben. Siehe Abschnitt 9.

---

## 3. Gesamtarchitektur

```
 Rust-App (Host)                                   XDJ-RX3
 ┌─────────────────────────────┐   USB    ┌──────────────────────────────┐
 │ Bibliothek / Audiodateien   │          │ interner USB-Hub 2b73:003e   │
 │                             │          │  ├ Dev B 2b73:0007  NIC      │
 │  UDP 50000/50001/50002 ◄────┼──────────┼──┤   (Ethernet über USB)     │
 │  TCP 12523  (dbserver-Port) │          │  └ Dev A 2b73:003d  Audio+   │
 │  TCP <dyn>  (dbserver)      │          │       MIDI + HID             │
 │  UDP 50111  (Portmapper)    │          └──────────────────────────────┘
 │  UDP 36251  (MOUNT v1)      │
 │  UDP 2049   (NFS v2)        │   IPs: Host 169.254.x.y, XDJ 169.254.67.96
 └─────────────────────────────┘   Maske 255.255.0.0 (Link-Local)
```

Rollen: Das **Pult ist Client** (fragt Metadaten per dbserver, liest Dateien per NFS), der **Host ist Server** plus Pro-DJ-Link-Teilnehmer.

Schichten zum Implementieren:
1. USB → Ethernet-Frames (Abschnitt 4)
2. IP-Stack/Sockets (Abschnitt 5)
3. Pro DJ Link UDP (6)
4. NFS-Server (7)
5. dbserver (8)

---

## 4. USB-Ebene

### 4.1 Geräte [BEOBACHTET]

| Bus-Adr. | VID:PID | Funktion |
|---|---|---|
| 1 | 2B73:003E | Hub (Klasse 9), 2 Ports |
| 2 | 2B73:0007 | **Ethernet-over-USB** (Interface-Klasse FF/FF). Config: Interface 0 mit Bulk-IN `0x81` (512 B), Bulk-OUT `0x02` (512 B), Interrupt-IN `0x83` (2 B, Intervall 8) |
| 3 | 2B73:003D | Audio (Isochron `0x01` OUT / `0x82` IN, vendorspezifisch), MIDI-Streaming (Bulk OUT `0x04`, IN `0x85`), HID (Interrupt OUT `0x06`, IN `0x87`) |

Vollständiger Config-Descriptor von Gerät 3 (Hex, aus dem Mitschnitt):
`0902a100040100c0000904000000ff0000020904000102ff000002070501050004040705820500040409040100000101000409240100010900010209040200020103000007240100013d000624020101000624020202000924030203010101000924030104010201000705040200020005250101010705850200020005250101040904030002030000060921110100012234000705060340000307058703140004`
(Zugehörige Zuordnung der Interfaces bitte beim Implementieren aus dem Descriptor neu parsen.)

### 4.2 Ethernet-over-USB-Framing (Gerät 2) – [HYPOTHESE, sehr wahrscheinlich]

Das Layout passt exakt zum **Realtek RTL8152/8153-Treiber (`r8152`)**: Vendor-Control-Requests `C0 05 …` (GET_REGS) auf Adressen `0xE908` und `0xE890` (Tally-Zähler, ~150× pro 15 s als Statistik-Polling), TX-/RX-Deskriptoren wie unten. Gegenprüfung: Linux `drivers/net/usb/r8152.c` (`struct tx_desc`, `struct rx_desc`).

**Host → Pult (Bulk OUT `0x02`)**: pro Ethernet-Frame ein 8-Byte-TX-Deskriptor, dann der Frame.

```
u32 LE opts1 = frame_len | (1<<31 /*FS*/) | (1<<30 /*LS*/)   // z. B. 0xC0000062 für 98 Byte
u32 LE opts2 = Checksum-Offload-Flags                         // beobachtet: 0xA0000000, 0x20000000, 0x60000000 (TCP/UDP-Offload)
[frame_len Bytes Ethernet-Frame, ohne FCS]
```
Alle 3 203 beobachteten OUT-Transfers: `len(Transfer) == 8 + frame_len`. Die Offload-Flags: wenn nicht implementiert, Prüfsummen in Software berechnen und `opts2 = 0` senden [zu testen].

**Pult → Host (Bulk IN `0x81`)**: mehrere Frames pro Transfer möglich, je 24-Byte-RX-Deskriptor:

```
u32 LE opts1: Bits 0..14 = Länge inkl. 4 Byte FCS, d. h. Frame = len-4
... (weitere 20 Byte Deskriptor, u. a. Checksum-Status)
[Frame, len-4 Byte][FCS 4 Byte]
nächster Deskriptor beginnt 8-Byte-aligned
```
Beleg: Transfer 368 B = 24 + 338 (= 334 Frame + 4 FCS = 362) → auf 8 aufgerundet = 368 ✓. Es gibt auch Transfers mit mehreren Frames (z. B. 0x0a-Status-Pakete und NFS-Anfragen gemischt).

**Interrupt IN `0x83`:** Link-Status [OFFEN, nicht ausgewertet].

### 4.3 Zwei Umsetzungswege (Entscheidung für den Agent)

- **Weg A – Betriebssystem-NIC (empfohlen, zuerst):** Unter Windows installiert der Hersteller-Treiber einen virtuellen Ethernet-Adapter (im Mitschnitt: Host-MAC `C8:3D:FC:11:43:61`, Host-IP per APIPA `169.254.226.215`). Die Rust-App nutzt dann nur normale Sockets, kein USB-Code. Aufgaben: Adapter finden, IP prüfen, Sockets daran binden, Broadcast `169.254.255.255` senden.
- **Weg B – Userspace-NIC (plattformunabhängig):** `rusb` + Userspace-TCP/IP (`smoltcp`) + Deskriptor-Framing aus 4.2. Aufwändiger; macht keine Treiberabhängigkeit. Unter Linux ist zusätzlich testenswert, das Gerät per `r8152`-`new_id` zu binden (**[HYPOTHESE]**, ungetestet).

Weg A zuerst, Weg B als Austauschbarkeit hinter einem `trait EthernetLink` kapseln.

---

## 5. Netzwerkebene [BEOBACHTET]

| | MAC | IP |
|---|---|---|
| XDJ-RX3 | `C8:3D:FC:11:43:60` | `169.254.67.96` (letzte zwei Bytes = MAC-Ende `43:60`) |
| Host (rekordbox) | `C8:3D:FC:11:43:61` | `169.254.226.215` (APIPA, Maske `255.255.0.0`) |

- Broadcast: `169.254.255.255`. Außerdem einmalig mDNS (`224.0.0.251`) vom Host – irrelevant.
- Host-Geräte-ID im Pro-DJ-Link: **0x11 (17)**. XDJ-RX3 meldet sich mit den IDs `0x0B`/`0x0C` (11/12) [Bedeutung der beiden IDs: HYPOTHESE, Decks/Kanäle].
- Der Host muss eine freie 169.254.x.x-Adresse haben. Die XDJ-Seite ist fix.

---

## 6. Pro DJ Link (UDP 50000 / 50001 / 50002)

### 6.1 Gemeinsamer Header [BEOBACHTET]
```
0x00..0x09  "Qspt1WmJOL"            (10 Byte Magic)
0x0A        Paket-Typ
0x0B..0x1E  Gerätename, 20 Byte, nullterminiert ("rekordbox", "XDJ-RX3")
            (Port 50000-Keepalive: Byte 0x0B = 0x00, Name beginnt bei 0x0C)
0x1F        0x01
0x20..      typabhängig
```

### 6.2 Beobachtete Pakete

| Dir | Port | Typ | Länge | Rate | Bedeutung |
|---|---|---|---|---|---|
| Host→Broadcast | 50000 | `0x06` | 54 | alle ~2 s (7× in 15 s) | Keepalive „ich bin Gerät 0x11": `...01 03 00 36 11 01 <MAC 6B> <IP 4B> 03 01 00 00 04 08` |
| Host→Broadcast | 50002 | `0x29` | 56 | **10 Hz** (100 ms) | Host-Status |
| XDJ→Host | 50002 | `0x0A` | 292 | **10 Hz** | Pult-Status (Deck-/Wiedergabestatus, BPM, Position …) |
| XDJ→Host | 50002 | `0x05` | 48 | selten | Media-Slot-Anfrage (enthält Host-IP, Slot-Info) |
| Host→XDJ | 50002 | `0x06` | 192 | als Antwort darauf | Media-Slot-Antwort, Name `rekordbox` als UTF-16**BE** |
| Host→XDJ | 50002 | `0x16` | 48 | selten | ? |
| XDJ→Host | 50002 | `0x30` | 36 | selten | ? |
| Host→XDJ | 50002 | `0x31` | 44 | Antwort auf `0x30` | ? |
| XDJ→Host | 50002 | `0x1A` | 40 | selten | ? |

Vollständige Beispiel-Hexdumps (als Test-Fixtures verwenden):

```
Host 0x06 Keepalive (50000):
5173707431576d4a4f4c060072656b6f7264626f780000000000000000000000010300361101c83dfc114361a9fee2d7030100000408

Host 0x29 Status (50002, Broadcast, 10 Hz):
5173707431576d4a4f4c2972656b6f7264626f7800000000000000000000000101110038110000c00010000000000000001000000009ff00

Host 0x16 (50002):
5173707431576d4a4f4c1672656b6f7264626f7800000000000000000000000101110000000000000000000000000000

Host 0x31 (50002):
5173707431576d4a4f4c3172656b6f7264626f78000000000000000000000001031100080600000000000000

Host 0x06 Media-Slot-Antwort (50002, 192 B):
5173707431576d4a4f4c0672656b6f7264626f780000000000000000000000010111009c000000110000000400720065006b006f007200640062006f00780000...(Nullen)...71f000000101000000140000000000000000000000...

XDJ 0x05 Slot-Anfrage (48 B):
5173707431576d4a4f4c0558444a2d5258330000000000000000000000000001000b000ca9fe43600000001100000004

XDJ 0x0A Status (292 B, Ruhezustand/Track geladen, Beginn):
5173707431576d4a4f4c0a58444a2d5258330000000000000000000000000001050c01000c000100...
```

### 6.3 Umsetzungsstrategie
1. Mit Replay der Host-Pakete beginnen (Keepalive 1/s, `0x29` mit 10 Hz). Feldaufbau nur dort ändern, wo nötig (z. B. IP/MAC im Keepalive aus dem eigenen Adapter ableiten).
2. XDJ-Pakete (`0x0A`) parsen und loggen: Welche Felder ändern sich bei Play/Cue/Pitch? Das ist die Basis für „Steuerung beidseitig" im Link-Modus.
3. Geräte-Claim-Phase (`0x00/0x02/0x04` auf Port 50000) liegt **vor** Mitschnittbeginn und ist **[OFFEN]**: neuen Mitschnitt ab Kabel-Einstecken aufnehmen (Abschnitt 9).
4. Die XDJ-Broadcast-Pakete (Ziel-MAC `ff:ff:ff:ff:ff:ff`, Keepalives/Beats) sind im Mitschnitt enthalten, wurden hier aber noch nicht ausgewertet.

---

## 7. NFS-Dateizugriff (Host = Server, UDP)

Das Pult liest die Audiodateien **direkt per NFS v2** vom Host. Beobachtete Dienste:

| Dienst | Programm | Version | Port |
|---|---|---|---|
| Portmapper | 100000 | 2 | **UDP 50111** (nicht 111!) |
| MOUNT | 100005 | 1 | UDP 36251 (vom Host frei gewählt, per GETPORT bekanntgegeben) |
| NFS | 100003 | 2 | UDP 2049 |

Beobachteter Ablauf (alles ONC-RPC über UDP, XDR, Credentials `AUTH_UNIX`/`AUTH_NULL`):

1. **PMAP GETPORT (proc 3)** an Host:50111, Argumente `[prog, vers, proto=17 (UDP), port=0]`.
   - prog 100005 v1 → Antwort `0x00008D9B` (36251)
   - prog 100003 v2 → Antwort `0x00000801` (2049)
2. **MOUNT EXPORT (proc 5)**: Antwort = Exportliste, im Mitschnitt zwei Einträge: `/C/` und `/D/` (Laufwerksbuchstaben des Windows-Hosts), Gruppe je `"169.254.226.215/255.255.0.0"` (eigene IP/Netzmaske als Text).
3. **MOUNT MNT (proc 1)** mit Pfad `/C/` bzw. `/D/` → Status 0 + 32-Byte-Filehandle.
4. **NFS LOOKUP (proc 4)** Komponente für Komponente den Pfad entlang, dann **NFS READ (proc 6)**: 8192 Byte pro Anfrage, sequenziell aufsteigende Offsets (569 READs ≈ 4,6 MB für eine FLAC). Andere Prozeduren (GETATTR, READDIR, …) kamen nicht vor.

### 7.1 Wichtige Eigenheiten [BEOBACHTET]

- **Pfade/Dateinamen sind UTF-16LE** in den XDR-Strings: `"/C/"` = Länge 6, Bytes `2f 00 43 00 2f 00`; LOOKUP-Name `"Users"` = Länge 10, `55 00 73 00 65 00 72 00 73 00`. XDR-Länge zählt **Bytes**, nicht Zeichen; auf 4 Byte auffüllen.
- Der Pfad in LOOKUP/MNT entspricht dem Pfad aus dem dbserver-Datensatz (`C:/Users/…/Track.flac` → Mount `/C/`, dann Komponenten `Users`, …).
- **Filehandle** (32 Byte, undurchsichtig): der Host kodiert `[id u32 BE][parent_id u32 BE][Nullen]`, z. B. `Users`=(2,0), danach (3,2), (4,3), (5,4), (6,5), (7,6), Datei (8,7). Root `/C/`=(0,0). Das Schema ist frei wählbar; die Tabelle `id → Pfad` ist die einfachste zustandsbehaftete Lösung. Das Pult schickt Handles nur unverändert zurück.
- **fattr** (17 Wörter, XDR): Verzeichnis `type=2`, `mode=0x000041B6` (040666); Datei `type=1`, `mode=0x000081A4` (0100644). `nlink=1`, `uid=gid=0`, `size`, `blocksize`, `rdev`, `blocks`, `fsid`, `fileid`, atime/mtime/ctime (Sekunden+µs). Genaue Werte bitgleich aus den Fixtures übernehmen (LOOKUP-/READ-Antworten im Mitschnitt).
- LOOKUP-Antwort: `status(0) | fh[32] | fattr(68 B)`. READ-Antwort: `status(0) | fattr(68 B) | len | data (4-aligned)`.
- Eine READ-Antwort mit 8192 Byte Nutzdaten ist größer als die MTU und wird als **IP-Fragmente** gesendet (erstes Fragment: 1 472 Byte UDP-Payload, insgesamt ca. 6 Fragmente; 5,1 MB Bulk-OUT für 569 Antworten). Der Sender muss Fragmentierung beherrschen (OS-Sockets: ok; Weg B: smoltcp prüfen).
- Mindestumfang des Servers: `NULL`(0), `GETATTR`(1), `LOOKUP`(4), `READ`(6); zur Sicherheit zusätzlich `STATFS`(17) und `READLINK`(5) mit Fehlerstatus bedienen. Fehler `NFSERR_NOENT` (2) bei unbekanntem Namen.
- Nur lesender Zugriff. Keine Schreib-Prozeduren implementieren (`NFSERR_ROFS`/`ACCES`).

---

## 8. dbserver (Metadaten, TCP)

### 8.1 Port-Ermittlung [BEOBACHTET]
1. XDJ verbindet zu Host **TCP 12523** und sendet `00 00 00 0F "RemoteDBServer\0"` (4-Byte-Länge 15 + ASCII).
2. Host antwortet mit **2 Byte Port (Big-Endian)**: im Mitschnitt zunächst zweimal `FF FF` (nicht bereit), dann `E8 18` = **59416**. Die eigentliche Verbindung läuft zu diesem dynamischen Port (frei wählbar).
3. Das Pult öffnet **mehrere parallele Verbindungen** (im Mitschnitt zwei, Quellports 41166/41168), je ein eigener Nachrichtenstrom.

### 8.2 Feld-Kodierung [BEOBACHTET]
Big-Endian, jedes Feld mit Typ-Tag:

| Tag | Inhalt |
|---|---|
| `0F` | 1 Byte Zahl |
| `10` | 2 Byte Zahl |
| `11` | 4 Byte Zahl |
| `14` | Blob: `u32 Länge` + Daten |
| `26` | String: `u32 Anzahl UTF-16BE-Zeichen (inkl. Nullterminator)` + Daten |

**Verbindungsaufbau:** erstes Feld jeder Verbindung ist `11 00000001`. Danach Nachrichten:

```
11 872349AE                  Magic
11 <txid u32>                Transaktions-ID (Antworten spiegeln sie)
10 <type u16>                Nachrichtentyp
0F <argc u8>                 Argumentanzahl
14 00 00 00 0C <12 Byte>     Argumenttypen-Tabelle (06 = Zahl, 02 = String, 03 = Blob, Rest 00)
<argc Argumente>             je nach Tabelle als Feld kodiert
```
- Argumenttyp `06` → `11 <u32>`; `02` → `26 <len><utf16be>`; `03` → **zwei** Felder: `11 <blob_size>` und, **nur wenn size > 0**, `14 <len><data>`.
- Erste Nachricht (`type 0x0000`, txid `0xFFFFFFFE`) enthält die Geräte-ID des Pults (`11`/`12`), Antwort `0x4000` mit Args `[0, 17]` (17 = Host-ID).

### 8.3 Beobachtete Nachrichtentypen

| Typ (Pult→Host) | Antwort | Inhalt/Bedeutung |
|---|---|---|
| `0x0000` | `0x4000` | Intro, Geräte-ID |
| `0x1000` | `0x4000` (+Menü) | Hauptmenü anfordern |
| `0x3000` | `0x4001` + n×`0x4101` + `0x4201` | „Menü rendern": Header (Anzahl Einträge), Einträge, Abschluss |
| `0x1002` | `0x4000` | Track-/Listenmenü anfordern (z. B. Playlist, Alle Tracks) |
| `0x1102` | `0x4000` | Menü für Track-ID |
| `0x1202` | `0x4000` | Menü (z. B. Titel/Album) |
| `0x2002` | `0x4000`/`0x4101` | Track-Metadaten (Titel, Artist, Dateipfad `C:/…`, BPM …) |
| `0x2003` | `0x4002` | Album-Cover (Blob; im Mitschnitt Größe 0) |
| `0x2004` | `0x4402` | **Waveform-/Vorschau-Blob** (904 B im Mitschnitt, wirkt wie 1-Byte-pro-Spalte-Vorschau) [HYPOTHESE] |
| `0x2102` | ... | weitere Track-Infos |
| `0x2C04` | ... | weitere Analysedaten (Wellenform/Beatgrid/Cues) [HYPOTHESE] |

Erstes Argument von Anfragen = `u32` aus 4 Byte: `[Anfragende Geräte-ID][Menü/Kontext][Slot][Track-Typ]`, z. B. `0x0B010101` (Gerät 11, Menü 1, Slot 1, Track-Typ 1 = rekordbox). Erwarte, dass die Bedeutung der Bytes 2–4 im Test verifiziert werden muss.

`0x4101`-Menüeintrag hat 12 Argumente (Typtabelle `060606020602060606060606`): `[parent_id, item_id, label1_bytes, label1_string, label2_bytes, label2_string, item_type, …]`. Beispiele aus dem Mitschnitt: Hauptmenü-Labels `\uFFFAARTIST\uFFFB`, `ALBUM`, `TRACK`, `KEY`, `PLAYLIST`, `HISTORY`, `SEARCH`, `MATCHING`, `DATE ADDED` – die Zeichen `U+FFFA`/`U+FFFB` sind Anfang/Ende eines Label-Markers.

### 8.4 Ablauf „Track laden" (aus dem Mitschnitt rekonstruiert)
1. Pult browst: `0x1000` → `0x3000` (Hauptmenü) → `0x1002/0x1102/0x1202` (Playlist/Track) → `0x3000`.
2. Pult wählt Track-ID und fragt Metadaten (`0x2002`), Cover (`0x2003`) und Waveform (`0x2004`, `0x2C04`).
3. Metadaten enthalten den **Dateipfad** (`C:/…/Titel.flac`).
4. Pult mountet NFS (`/C/`) und liest die Datei per `READ` (Abschnitt 7).
5. Wiedergabe startet im Pult; Status läuft über `0x0A`-Pakete (Abschnitt 6).

### 8.5 Bekannter Parser-Stolperstein
Blob-Argument: Größe `11 <n>` **immer**, `14 <n> <data>` nur bei `n > 0`. Wer das nicht beachtet, verliert die Synchronisation beim nächsten Magic `11 87 23 49 AE`.

---

## 9. Lücken und nächste Mitschnitte

Aufnahme jeweils mit USBPcap (Windows), rekordbox + XDJ-RX3 per USB:

| # | Szenario | Was damit geklärt wird |
|---|---|---|
| 1 | Ab **Kabel einstecken** aufnehmen, ohne Aktion | Geräte-Claim (`0x00/0x02/0x04` auf Port 50000), komplette USB-Enumeration, ggf. Authentifizierung |
| 2 | Track laden, **Play, Cue, Pitch ändern, Loop** | Struktur der `0x0A`-Statuspakete; Steuerbefehle (falls vorhanden) |
| 3 | Mehrere Playlists, Suche, große Bibliothek | Menüstruktur der dbserver-Nachrichten, Paging bei `0x3000` |
| 4 | Track mit Beatgrid, Cues, farbiger Waveform | Formate hinter `0x2004`/`0x2C04` (Waveform, Beatgrid, Cues) |
| 5 | **Controller-Modus** (HID/MIDI), Jog/Pads/Waveform auf Pult-Display | HID-Reports (Interface 3 von Gerät 3), SysEx; dort ist ein „Hardware Unlock" erst zu belegen |

Aus dem Mitschnitt bekannt für MIDI: einmalig SysEx `F0 00 40 05 00 00 03 0D 00 50 01 F7` (Hersteller-ID `00 40 05`, 74× gesendet, 16 Byte pro USB-MIDI-Transfer) – Bedeutung **[OFFEN]**.

---

## 10. Umsetzungsplan für den Agent (Rust)

**Crates (Vorschlag):** `tokio`, `bytes`, `thiserror`, `tracing`; Weg B zusätzlich `rusb`, `smoltcp`; Tests: `pcap-file`/eigener pcapng-Reader.

**Module:**
```
src/
  link/        trait EthernetLink + os_nic.rs (Weg A) + usb_rtl.rs (Weg B)
  djlink/      packet.rs (Header, Typen), keepalive.rs, status.rs
  nfs/         xdr.rs, rpc.rs, portmap.rs, mount.rs, nfsv2.rs, fs_table.rs
  dbserver/    fields.rs, message.rs, server.rs, handlers/ (menu, metadata, art, waveform)
  library/     Adapter zur bestehenden Track-Bibliothek der App
```

**Reihenfolge (jeweils mit eigenem Akzeptanztest):**
1. **Fixtures bauen:** Den pcapng in `fixtures/` ablegen, kleines Tool schreiben, das Frames (USB → Ethernet nach 4.2) extrahiert und je Protokoll dumpt. Golden-Tests gegen die Hexdumps in diesem Dokument.
2. **Pro DJ Link:** Keepalive/Status senden, XDJ-Pakete empfangen und loggen. Akzeptanz: Pult zeigt den Host als Gerät (Link-Erkennung).
3. **NFS:** Portmapper(50111) + MOUNT + NFSv2 (NULL/GETATTR/LOOKUP/READ). Akzeptanz: Datei-Roundtrip gegen die Fixture (gleiche Bytes für gleiche Offsets), danach reales Pult.
4. **dbserver:** Port-Handshake (12523), Feld-Codec, Intro, Hauptmenü, Playlist-/Track-Listen, Metadaten, Cover (Größe 0 erlaubt), Waveform-Platzhalter. Akzeptanz: Pult zeigt Bibliothek und lädt einen Track.
5. **Waveforms/Beatgrid/Cues:** Formate nach neuem Mitschnitt (Szenario 4) umsetzen.
6. **Statusauswertung `0x0A`** für die Anzeige in der eigenen UI; Steuerrichtung erst nach Szenario 2.

**Robustheit:** UDP-Fragmentierung für NFS-Antworten; Timeouts/Retransmits (RPC-xid wiederholbar beantworten); mehrere gleichzeitige dbserver-Verbindungen; alle Strings UTF-16 (LE bei NFS, BE bei dbserver/Pro DJ Link); Byte-Order pro Schicht prüfen (NFS/dbserver/Pro DJ Link Big-Endian, USB-Deskriptoren Little-Endian).

**Pflicht-Logging:** Rohbytes jedes unbekannten Pakets/Nachrichtentyps mit Zeitstempel in eine Datei schreiben, damit unbekannte Typen aus dem Live-Betrieb nachträglich analysiert werden können.

---

## Anhang: Beobachtete Zahlen (zur Plausibilitätsprüfung)

- 53 397 USB-Pakete, 14,9 s; 4 095 Ethernet-Frames extrahiert (Unicast/Host-Seite), davon 3 203 Host→Pult.
- NFS: 583 READ-Aufrufe (`proc 6`), 7 LOOKUP (`proc 4`); dbserver: zwei Nachrichtenströme mit je Pult-Anfragen und Host-Antworten; Pro DJ Link: `0x29` und `0x0A` jeweils ~100 ms.
- Entropie NFS-Nutzdaten: 7,68–7,86 bit/Byte (FLAC).

---

## 11. Nachtrag: Mitschnitt 2 (`mitschnittmithandshake.pcapng`, Start vor rekordbox, 29,2 s, 94 782 Pakete)

Dieser Nachtrag **überschreibt** die Aussagen „[OFFEN]" zu Geräte-Claim und Authentifizierung in den Abschnitten 6 und 9.

### 11.1 Kryptografie-Befund (Handshake)
**Auch im Handshake gibt es keine Kryptographie, keine Challenge/Response und kein Geheimnis.** Alles Klartext, jede Wiederholung bitgleich oder nur mit Zählern:
- Keine TLS-/SSH-/X.509-/PEM-Muster (0 Treffer).
- Alle Handshake-Pakete haben niedrige Entropie (Pro DJ Link ≈ 2,4 bit/Byte, dbserver ≈ 3,5–4,5). Hohe Entropie nur in den Audiodaten (NFS/FLAC, ≈ 7,4).
- NFS-RPC nutzt `AUTH_UNIX` (Flavor 1) mit leeren Feldern (uid 0, gid 0, kein Rechnername), also keine Authentisierung.
- Die einzige „magische" Konstante ist `0x12345678` im Paket `0x47` (siehe 11.3), konstant und kein Nonce.
- Das MIDI-SysEx wird 64× **identisch** alle 0,2 s gesendet (siehe 11.4). Ein Challenge/Response würde variieren.

Der „Hardware Unlock" von rekordbox findet sich im Link-Export-Pfad nicht auf dem Draht; er scheint reine Software-Lizenzlogik von rekordbox zu sein (nicht belegt, nur nicht beobachtbar).

### 11.2 Zeitablauf (relativ zum Mitschnittbeginn)
| t [s] | Ereignis |
|---|---|
| 0,0 | USB-Descriptor-Reads (Geräte schon enumeriert). Gerätename-String: `AlphaTheta Corporation`, Produkt `XDJ-RX3` |
| 0,97 / alle 2 s | XDJ sendet Keepalive `0x06` an Broadcast:50000 (96-Byte-Frame, Geräte-ID `0x0B`) |
| 7,0 | rekordbox startet: **Claim Stufe 1** (`0x00`, 3×) |
| 8,40 | USB-Audio: `SET_INTERFACE` (Gerät 3, Alt 1) + UAC `SET_CUR` Abtastrate `44100` (`22 01 00 01 82 00 03 00 44 AC 00`) |
| 9,3–12,3 | Windows sendet SSDP (`M-SEARCH`, 239.255.255.250:1900) – irrelevant |
| 11,26–16,84 | **Claim Stufe 2** (`0x02`, 36×) |
| 16,37 | erstes MIDI-SysEx (Gerät 3), danach alle 0,2 s |
| 16,94 | Host schickt `0x06`-Keepalive als Gerät `0x11` → Claim abgeschlossen |
| 16,95–17,05 | Austausch `0x30/0x31`, `0x10/0x11`, `0x46`, `0x16`; danach `0x0A`-Status (XDJ) und `0x29` (Host) je 10 Hz |
| 18,4 | Media-Slot `0x05`/`0x06`, `0x31` |
| 18,8 | dbserver-Portabfrage (TCP 12523), dann dbserver |
| 22,0 | `0x46` → `0x47` |
| 25,2 | NFS (Portmap → MOUNT → LOOKUP/READ), 469 Aufrufe bis 29 s |

### 11.3 Pro-DJ-Link Geräteanmeldung (Port 50000, Broadcast `169.254.255.255`) [BEOBACHTET]
Gemeinsamer Header wie in 6.1; Paketkörper ab Offset `0x20`:

- **Stufe 1, Typ `0x00`, 44 Byte, 3×** (t = 7,0 / 8,5 / 11,2 s): `01 03 00 2C <n> 04 <MAC 6B>`, mit Zähler `n = 1,2,3`.
- **Stufe 2, Typ `0x02`, 50 Byte, 36×** (6 Runden × 6 Kandidaten, Abstand ≈ 0,1 s): `01 03 00 32 <IP 4B> <MAC 6B> <Geräte-ID-Kandidat> <Runde 1..6> 04 01`.
  Kandidaten in dieser Reihenfolge: `0x11, 0x12, 0x29, 0x2A, 0x2B, 0x2C` (17, 18, 41, 42, 43, 44). Der Host versucht also eine Nummer zu belegen (automatische Wahl); `0x11` war am Ende die verwendete.
- **Abschluss:** reguläre Keepalives `0x06` (54 Byte, alle 2 s): `01 03 00 36 11 01 <MAC> <IP> 03 01 00 00 04 08` – beim ersten Mal mit `...<IP> 01 01 00 00 04 08`, danach `03 01 ...` (Feld = Zählerstand/Peer-Anzahl, Bedeutung HYPOTHESE).
- Die Stufe-3-Pakete (`0x04`) fehlen in diesem Ablauf; rekordbox springt nach Stufe 2 direkt in Keepalive. **Nachbau: exakt diese Folge senden, Timing wie in der Tabelle** und auf Konfliktantworten des Pults achten (keine beobachtet).
- XDJ-Keepalive (`0x06`, 96 Byte, Quellport variabel, Ziel Broadcast:50000): `01 03 00 36 0B 02 <MAC XDJ> <IP XDJ> ...`.

### 11.4 Neue Pakettypen auf Port 50002 (Unicast) [BEOBACHTET]
Alle Klartext, keine Zufallsfelder:

| Typ | Dir | Länge | Körper ab 0x20 | Anmerkung |
|---|---|---|---|---|
| `0x10` | XDJ→Host | 36 | `00 0B 00 00` | „Hallo" des Pults (Geräte-ID 0x0B) |
| `0x11` | Host→XDJ | 296 | `01 11 01 04 11 01 00 00 00 <UTF-16BE-Rechnername, Nullen aufgefüllt>` | Host stellt sich vor: Geräte-ID `0x11`, Rechnername (UTF-16BE) |
| `0x46` | XDJ→Host | 40 | `00 0B 00 04 0B 04 00 00` | 2× (16,97 s und 22,0 s) |
| `0x47` | Host→XDJ | 72 | `01 11 00 24 11 04 00 00 12 34 56 78 00 00 00 01 01 01 03 01 01 01 00 …` | Antwort auf `0x46` (22,0 s); enthält konstanten Marker `0x12345678` |
| `0x30` / `0x31` | XDJ→Host / Host→XDJ | 36 / 44 | `03 0B 00 00` / `03 11 00 08 06 00 00 00 00 00 00 00` | wie in Mitschnitt 1 |
| `0x16` | Host→XDJ | 48 | `01 11 00 …` (alles 0) | wie in Mitschnitt 1 |

Das Feld „Rechnername" im `0x11`-Paket ist ein beliebiger String (hier der Windows-Rechnername); die eigene Software darf einen eigenen Namen senden (z. B. UTF-16BE `"RustDJ"`). Ob das Pult daran etwas knüpft, ist zu testen.

### 11.5 USB-/MIDI-Ebene [BEOBACHTET]
- Einzige Steuerung über Gerät 3: **SysEx** `F0 00 40 05 00 00 03 0D 00 50 01 F7` (USB-MIDI-Pakete `04 F0 00 40 | 04 05 00 00 | 04 03 0D 00 | 07 50 01 F7`), **64× identisch, alle 200 ms**, Start t = 16,37 s (kurz vor dem Link-Handshake), danach bis Ende. Wirkt wie ein Keepalive/Statusbefehl. Keine MIDI-Antwort des Pults im Mitschnitt (Endpoint `0x85` ohne Verkehr). Hersteller-ID `00 40 05`.
- HID-Interface (Gerät 3, Interrupt `0x06`/`0x87`) und Interrupt-IN `0x83` von Gerät 2 wurden **nicht benutzt**: auch in diesem Szenario kein Controller-Verkehr.
- Vendor-Control `C0 05 08E9 …` (Gerät 2): ca. alle 0,7 s Link-Status-Polling des NIC-Chips, unabhängig vom Handshake.

### 11.6 dbserver (Ergänzung)
Gleiche Mechanik wie Abschnitt 8. Unterschiede:
- Portabfrage (TCP 12523): nur eine Verbindung, Antwort `EF 50` = Port 61264 (dynamisch, bei jeder Sitzung anders).
- Zusätzlicher Nachrichtentyp **`0x3007`** (2×, Pult→Host) neben `0x3000`; Inhalt noch nicht ausgewertet.
- Wie zuvor: keine Auth-Felder; erstes Argument (`0x00` / `0x11`-Geräte-ID) nur zur Zuordnung.

### 11.7 Konsequenz für die Umsetzung
1. Es gibt **nichts zu knacken**: Claim-Folge (11.3) + Keepalive + `0x10/0x11`, `0x30/0x31`, `0x46/0x47` genau so senden wie rekordbox.
2. Das SysEx aus 11.5 ab dem Linkstart alle 200 ms senden (Gerät 3, USB-MIDI Bulk OUT `0x04`). Zweck unklar, daher zuerst mit und ohne testen, ob Pult die Library-Anfragen sendet.
3. Offen bleiben weiterhin **Controller-Modus (HID/MIDI-Steuerung)**, Wiedergabe-/Cue-Steuerung und Waveform-Formate: dafür sind die Szenarien 2–5 aus Abschnitt 9 nötig. Szenario 1 (Handshake) ist mit diesem Nachtrag **erledigt**.
