# Pro DJ Link – Protokollspezifikation (aus Mitschnitt abgeleitet)

Quelle: `fd76cf47-mitschnitt.pcapng` (704 Pakete, 50,2 s). Zielgruppe: Coding-Agent, der in Rust einen Client für den **Pioneer XDJ-RX3** baut (bidirektional: Announce, Status lesen, Metadaten per dbserver abfragen).
Referenz-Parser (getestet gegen diesen Mitschnitt, 0 Fehler): `djl_parse.py`.

**Wichtig vorab**
- **Keine Kryptographie.** Alles ist Klartext, keine Authentifizierung, keine Signaturen.
- Alle Mehrbyte-Zahlen sind **Big Endian**, außer dort, wo ausdrücklich UTF-16LE steht (NFS-Pfade).
- Der Mitschnitt zeigt einen **Leerlauf**: beide Decks leer, nichts spielt, keine Beats. Beat-Pakete (UDP 50001), Position (50001) und Play-Zustände sind **nicht** im Mitschnitt. Felder dazu stehen nur als Verweis auf die öffentliche Dokumentation (Deep Symmetry, „DJ Link Packet Analysis“, djl-analysis.deepsymmetry.org), nicht als verifiziert.
- Legende: ✅ im Mitschnitt belegt · ◐ aus Mitschnitt + öffentlicher Doku konsistent · ❓ unbekannt/Vermutung.

---

## 1. Topologie im Mitschnitt

| Rolle | IP | MAC | Pro-DJ-Link-Gerätenr. | Name |
|---|---|---|---|---|
| XDJ-RX3 (Hardware) | 169.254.67.96 | c8:3d:fc:11:43:**60** | **11 (0x0b)** Keep-Alive; Status kommt für Deck **11 und 12** | `XDJ-RX3` |
| PC mit rekordbox | 169.254.226.215 | c8:3d:fc:11:43:**61** | **17 (0x11)** (nach Claim) | `rekordbox`, Hostname `DESKTOP-J7J3LN0` |

- Netz: **Link-Local 169.254.0.0/16**, Broadcast `169.254.255.255`, kein DHCP-Server (nur ein DHCP-Discover vom PC, unbeantwortet).
- Beide MACs liegen im selben OUI-Block (c8:3d:fc), die Verbindung RX3↔PC ist also vermutlich direkt (Kabel/USB-Ethernet). ❓
- Der RX3 sendet **ein** Keep-Alive (Nr. 11), aber **zwei** Status-Streams (Deck 11 und 12). ✅

## 2. Gesamtablauf (Zeitleiste)

| t [s] | Ereignis |
|---|---|
| 0–50 | RX3: Keep-Alive alle **2,0 s** (UDP 50000, Typ 0x06) ✅ |
| 18,0 / 18,8 / 21,4 | PC: Claim Stufe 1 (Typ 0x00), Zähler N=1,2,3 ✅ |
| 21,5–27,2 | PC: Claim Stufe 2 (Typ 0x02): 6 Kandidatennummern × N=1..6 = 36 Pakete, ~100 ms Abstand ✅ |
| 27,3 | PC: erstes Keep-Alive als Gerät **0x11**, danach alle **2,0 s** ✅ |
| 27,31 | RX3 beginnt Status (0x0a, UDP 50002) für Deck 12, ab 27,50 auch Deck 11; ca. 4 Pakete/s je Deck ✅ |
| 27,31–27,44 | RX3↔PC: Hello-/Kontrollpakete (0x30, 0x10, 0x11, 0x46) ✅ |
| 27,43 | PC: erstes 0x29-Paket (Broadcast 50002), danach alle ~0,1 s ✅ |
| 28,24–29,47 | RX3: NFS-Mount-Handshake gegen PC (Portmapper 50111, mountd 36251) ✅ |
| 28,86 | RX3 fragt Media-Slot-Info (0x05); PC antwortet (0x06) ✅ |
| 29,47 | RX3 öffnet dbserver: TCP :12523 (Port-Abfrage), dann TCP :63966 ✅ |
| 31,70 / 43,4 | RX3 durchsucht die rekordbox-Bibliothek (Menü, Artist-Liste, Album-Liste) ✅ |

Gerätenummern-Hinweis: 11 und 12 liegen **außerhalb** 1–4. In der öffentlichen Doku gilt: Bei Nummern außerhalb 1–4 funktionieren dbserver-Metadatenanfragen von *Dritt-Software* nicht. Der RX3 selbst nutzt sie hier trotzdem (als Client am PC). Für deinen Client Gerätenummer **1–4 (oder 5/6)** testen, **nicht** 11/12/17 (♦ Konflikt!). ◐

---

## 3. UDP 50000 – Announce / Keep-Alive / Nummernvergabe (Broadcast)

### 3.1 Gemeinsamer Header ✅

| Offset | Länge | Inhalt |
|---|---|---|
| 0x00 | 10 | ASCII-Magic `Qspt1WmJOL` (`51 73 70 74 31 57 6d 4a 4f 4c`) |
| 0x0a | 1 | **Pakettyp** |
| 0x0b | 1 | Subtyp, immer `00` (Broadcast) |
| 0x0c | 20 | Gerätename, ASCII, `00`-gepolstert (z. B. `XDJ-RX3`, `rekordbox`) |
| 0x20 | 1 | immer `01` |
| 0x21 | 1 | Strukturbyte, hier immer `03` |
| 0x22 | 2 | Gesamtlänge des Pakets (BE) |
| 0x24 | … | typabhängige Nutzdaten |

### 3.2 Typen

**Typ 0x06 – Keep-Alive** (Länge 0x36 = 54) ✅
| Off | Len | Feld | RX3-Wert | PC-Wert |
|---|---|---|---|---|
| 0x24 | 1 | Gerätenummer D | `0b` | `11` |
| 0x25 | 1 | Gerätetyp | `02` | `01` |
| 0x26 | 6 | MAC | c8:3d:fc:11:43:60 | …:61 |
| 0x2c | 4 | IPv4 | a9 fe 43 60 | a9 fe e2 d7 |
| 0x30 | 1 | ❓ konstant `01` | 01 | 01 |
| 0x31 | 1 | Anzahl erkannter Peers (laut Doku); PC: `01`→`03` ab t≈29,4 | **00 (konstant)** | 01, dann 03 |
| 0x32 | 2 | `00 00` | | |
| 0x34 | 1 | ❓ Hardware-Klasse | `07` | `04` |
| 0x35 | 1 | ❓ | `00` | `08` |

Beispiel RX3 (komplett):
```
51 73 70 74 31 57 6d 4a 4f 4c 06 00 58 44 4a 2d 52 58 33 00 00 00 00 00 00 00 00 00 00 00 00 00
01 03 00 36 0b 02 c8 3d fc 11 43 60 a9 fe 43 60 01 00 00 00 07 00
```
Abweichung zur Doku: Die Doku nennt an 0x30 den Wert `30` und an 0x31 „p“. Im Mitschnitt steht an 0x30 `01`. Offsets 0x30–0x35 daher **gegen ein eigenes Capture prüfen**. ◐

**Typ 0x00 – Claim Stufe 1** (Länge 0x2c = 44) ✅
`0x24`: Zähler N (1–3) · `0x25`: Gerätetyp (`04` beim PC) · `0x26..0x2b`: MAC

**Typ 0x02 – Claim Stufe 2** (Länge 0x32 = 50) ✅
`0x24`: IPv4 · `0x28`: MAC · `0x2e`: **vorgeschlagene Gerätenr.** · `0x2f`: Zähler N · `0x30`: `04` · `0x31`: `01` (Auto-Assign-Flag laut Doku)
Beobachtete Kandidatenfolge des PCs: `0x11, 0x12, 0x29, 0x2a, 0x2b, 0x2c`, jeweils für N=1..6. Danach Keep-Alive mit **0x11**. Ein Typ-0x04- (Final-Claim) oder 0x08-Paket (Konflikt) kommt **nicht** vor. ✅

**Typ 0x0a, 0x04, 0x08** (Initial-Announce, Final-Claim, Konflikt): im Mitschnitt nicht enthalten; Aufbau siehe Deep-Symmetry-Doku. ◐

### 3.3 Anforderungen an deinen Client
1. UDP-Socket auf **50000** (Broadcast empfangen, `SO_REUSEADDR`/`SO_BROADCAST`).
2. Eigene Keep-Alives als Typ 0x06 senden (alle 1,5–2 s) mit **echter MAC und IP des genutzten Interfaces**.
3. Erst nach eigenem Keep-Alive beginnt der RX3, Status an **Port 50002** zu schicken (so beschreibt es die Doku; im Mitschnitt schickt der RX3 Status schon bei der ersten Begegnung mit dem PC-Keep-Alive, t=27,3). ◐

---

## 4. UDP 50002 – Status und Steuerung (Unicast + Broadcast)

### 4.1 Header (anders als 50000!) ✅
| Offset | Länge | Inhalt |
|---|---|---|
| 0x00 | 10 | Magic `Qspt1WmJOL` |
| 0x0a | 1 | **Pakettyp** |
| 0x0b | 20 | Gerätename (**beginnt bei 0x0b**, nicht 0x0c) |
| 0x1f | 1 | `01` |
| 0x20 | 1 | Subtyp |
| 0x21 | 1 | Gerätenummer des Absenders |
| 0x22 | 2 | „lenr“ = Restlänge **nach** diesem Feld (Gesamtlänge − 0x24) |
| 0x24 | … | Nutzdaten |

Beobachtete Typen (Anzahl · Richtung · Länge):

| Typ | Anz. | Von → Nach | Länge | Bedeutung |
|---|---|---|---|---|
| 0x0a | 215 | RX3 → PC (unicast) | 292 (0x124) | **CDJ-/Deck-Status** |
| 0x29 | 227 | PC → Broadcast | 56 | **Mixer-/Master-Status** des rekordbox-Hosts ◐ |
| 0x30 | 3 | RX3 → PC | 36 | ❓ Hello/„Ich bin da“, Subtyp 03 |
| 0x10 | 1 | RX3 → PC | 36 | ❓ Hello, Subtyp 00 |
| 0x11 | 1 | PC → RX3 | 296 | **Hostname** des PCs (UTF-16BE) |
| 0x46 | 3 | RX3 → PC | 40 | ❓ Anfrage, Subtyp 00 |
| 0x47 | 1 | PC → RX3 | 72 | ❓ Antwort auf 0x46 (enthält Magic `12 34 56 78`) |
| 0x16 | 5 | PC → RX3 | 48 | ❓ leer (nur Nullen), 3+2 Pakete im Burst |
| 0x31 | 2 | PC → RX3 | 44 | ❓ Subtyp 03, Nutzlast `06 00 …` |
| 0x05 | 2 | RX3 → PC | 48 | **Media-Slot-Anfrage** |
| 0x06 | 2 | PC → RX3 | 192 | **Media-Slot-Antwort** |

### 4.2 Typ 0x0a – Deck-Status (292 Byte) ✅ / ◐
Gelesen aus dem Mitschnitt (Leerlauf). Feldnamen/Offsets stimmen mit der öffentlichen Doku überein, soweit der Mitschnitt sie belegt.

| Off | Len | Feld | Leerlauf-Wert (✅) | Bedeutung/Doku |
|---|---|---|---|---|
| 0x20 | 1 | Subtyp | `05` | |
| 0x21 | 1 | Gerätenr. | 11 oder 12 | Deck-Nummer |
| 0x22 | 2 | lenr | `0100` (=256) | 292 − 36 |
| 0x24 | 1 | Gerätenr. (wiederholt) | = 0x21 | |
| 0x26 | 1 | ❓ Deck-Marker | Deck 12: `01`, Deck 11: `00` | variiert zwischen den Decks |
| 0x27 | 1 | Aktivitätsflag | `00` | 0 = idle |
| 0x28 | 1 | Quellgerät des Tracks | `00` | |
| 0x29 | 1 | Slot (1 CD, 2 SD, 3 USB, 4 rekordbox) | `00` | |
| 0x2a | 1 | Track-Typ | `00` | 1 = rekordbox-Track |
| 0x2c | 4 | rekordbox-Track-ID | `00000000` | |
| 0x68 | 1 | ❓ | Deck 11: `01`, Deck 12: `00` | variiert |
| 0x7b | 1 | Play-Zustand P1 | `00` | (Tabelle in Doku, hier nicht belegt) |
| 0x89 | 1 | Flags F | **`80`** | Doku: Bit-Maske Play/Master/Sync/On-Air/BPM; `0x80` = „BPM gültig“-Bit hier gesetzt ❓ |
| 0x8c | 4 | Pitch | `00100000` = **0x00100000** | 0x100000 = ±0 % (Nenn-Tempo) ◐ |
| 0x90 | 2 | Master-Tempo-Gültigkeit | `7fff` | |
| 0x92 | 2 | **BPM × 100** | `ffff` | **0xFFFF = „kein Track“** ✅ (=655,35 darf nicht als BPM gelten!) |
| 0x98 | 4 | Pitch 2 | `00000000` | (in der Doku fader-gekoppelter Pitch; im Leerlauf 0) |
| 0x8b | 1 | P2 (Sekundär-Play-Zustand) | `9e` | ❓ |
| 0xa0 | 4 | Beat-Zähler | `ffffffff` | 0xFFFFFFFF = kein Beat |
| 0xa4 | 2 | Takte bis Cue | `01ff` | ❓ |
| 0xa6 | 1 | Beat im Takt | `00` | (1–4, wenn Track läuft) |
| 0xc8 | 4 | Paketzähler | `00000000` | blieb im Mitschnitt 0 ✅ |
| 0xcc | 1 | Hardware-Klasse | **`1f`** | XDJ-RX3 → `0x1f` ✅ |
| 0xcd | 1 | Touch-Audio-Bits | `01` | |
| 0xf0 | 4 | ❓ | Deck 11: `12345678`, Deck 12: `00000000` | Magic-Marker nur beim Deck-11-Status |
| 0xf7, 0xf9–0xfd | 1 | ❓ | Deck 11: `01`, Deck 12: `00` | Deck-/Link-Flags |

Takt: Median **0,158 s** zwischen Status-Paketen (beide Decks gemischt), also ca. 6/s gesamt, 3/s pro Deck. ✅
**Alle anderen Offsets waren in 215 Paketen konstant**, d. h. Beat-, Pitch-, Play- und Track-Felder sind nur als Struktur belegt, nicht ihre Dynamik. Zum Verifizieren: neuen Mitschnitt mit laufendem Track aufnehmen.

### 4.3 Typ 0x29 – Mixer-/Master-Status vom PC (56 Byte) ◐
```
… 0a 29 'rekordbox'… | 0x1f=01 | 0x20=01 | 0x21=11 | 0x22=00 38 |
0x24: 11 00 00 c0 00 10 00 00 00 00 00 00 00 10 00 00 00 09 ff 00
```
Alle 227 Pakete **byte-identisch**. Bedeutung nach Doku: Flags (0x27 = `c0`), Pitch (`00100000` an 0x28 und 0x30), BPM (0x2e = `0000`), 0x35 = `09`, 0x36 = `ff` (Master-Handoff/„kein Master“ ❓). Für den Nachbau: **ignorieren** oder 1:1 als „Ich bin keine Taktquelle“ senden. ❓

### 4.4 Handshake-Pakete beim ersten Kontakt (t≈27,3–29)
Alle Offsets ab 0x24 (✅ Hex im Mitschnitt):

| Typ | Hex-Nutzdaten ab 0x1f |
|---|---|
| 0x30 (RX3→PC) | `01 03 0b 00 00` (Sub=03, Dev=0b, lenr=0) |
| 0x10 (RX3→PC) | `01 00 0b 00 00` |
| 0x46 (RX3→PC) | `01 00 0b 00 04 | 0b 04 00 00` (lenr=4; Nutzdaten = Dev `0b`, `04`, `00 00`) |
| 0x47 (PC→RX3) | `01 01 11 00 24 | 11 04 00 00 | 12 34 56 78 | 00 00 00 01 | 01 01 03 01 01 01 00…` |
| 0x16 (PC→RX3) | `01 01 11 00 00` + 24 × `00` |
| 0x31 (PC→RX3) | `01 03 11 00 08 | 06 00 00 00 00 00 00 00` |

Die genauen Semantiken von 0x30/0x10/0x46/0x47/0x16/0x31 sind **nicht dokumentiert** (weder hier noch auf der Deep-Symmetry-Seite, die ich dazu geprüft habe). ❓ Wenn dein Client nur *lesen* soll, brauchst du sie nicht: Der RX3 sendet Status auch ohne sie, sobald er ein Keep-Alive sieht. ◐

### 4.5 Typ 0x11 – Hostname ✅
`0x20=01, 0x21=11, 0x22=0x0104`, `0x24=11 01 00 00`, ab **0x28**: UTF-16BE, nullterminiert/gepolstert: `DESKTOP-J7J3LN0`.

### 4.6 Typ 0x05 / 0x06 – Media-Slot-Abfrage ✅
**0x05** (RX3→PC, Länge 48, 2× in 2 ms): ab 0x24 `a9 fe 43 60` (IP des Absenders) · `00 00 00 11` (Zielgerät 0x11) · `00 00 00 04` (Slot 4 = rekordbox). 
Achtung: In der Doku steht an dieser Stelle die Nr. des *Anfragenden*; im Mitschnitt ist es die des *Ziels* (der Absender hat 0x0b). ◐ Im Zweifel eigenen Mitschnitt gegenprüfen.

**0x06** (PC→RX3, Länge 192, 2×): ab 0x24 `00 00 00 11` · `00 00 00 04` (Slot) · ab 0x2c UTF-16BE-Name `rekordbox` (64 Byte) · … ab 0xac: `07 1f 00 00 01 01 00 00 00 14 …` (❓ vermutlich Datum/Zähler/Track-Anzahl-Felder; **nicht entschlüsselt**).

---

## 5. TCP – dbserver (Metadaten-Datenbank des PCs)

Hier ist die Rolle **umgekehrt zum Normalfall**: Der **RX3 ist Client**, der **PC ist Server**. ✅

### 5.1 Port-Erkennung ✅
1. RX3 → PC **TCP :12523**, sendet `00 00 00 0f "RemoteDBServer\0"` (4-Byte-Länge 15 + String).
2. PC antwortet **2 Byte Portnummer** BE: `f9 de` = **63966**; Verbindung wird sofort geschlossen.
3. (CDJ-Hardware antwortet laut Doku mit 1051; rekordbox nutzt einen dynamischen Port. **Immer erst 12523 abfragen!**)

### 5.2 Verbindung ✅
- RX3 verbindet sich von lokalem Port 37457 auf 63966.
- RX3 sendet Greeting `11 00 00 00 01` (Zahlenfeld 1), PC echot `11 00 00 00 01`.
- Danach Nachrichten im Format unten, **ohne** Framing auf TCP-Ebene (nur die Feldstruktur). Segmente können mehrere Nachrichten enthalten (z. B. 4001+4101…+4201 in einem Segment); **über TCP-Stream parsen, nicht pro Paket**.

### 5.3 Feldtypen ✅
| Tag | Inhalt |
|---|---|
| `0f` | 1-Byte-Zahl |
| `10` | 2-Byte-Zahl BE |
| `11` | 4-Byte-Zahl BE |
| `14` | Blob: 4-Byte-Länge + Daten |
| `26` | String: 4-Byte-Länge in **Zeichen** (inkl. Terminator) + UTF-16BE |

### 5.4 Nachrichtenformat ✅
```
11 87 23 49 ae          Magic (Zahlenfeld 0x872349ae)
11 TT TT TT TT          Transaction-ID
10 MM MM                Nachrichtentyp
0f NN                   Anzahl Argumente
14 00 00 00 0c  <12 B>  Argument-Tags (je Byte: 06 = Zahl 4 B, 02 = String, 03 = Blob, 00 = unbenutzt)
<NN Argumente als Felder>
```
Beispiel (Setup, RX3→PC):
```
11 87 23 49 ae | 11 ff ff ff fe | 10 00 00 | 0f 01 | 14 00 00 00 0c 06 00 00 00 00 00 00 00 00 00 00 00 | 11 00 00 00 0b
```

### 5.5 Transaction-IDs ✅
- Setup: `0xfffffffe`, Typ `0x0000`, Argument = eigene Gerätenr. (hier `0x0b`). Antwort `0x4000` mit Args `[0, 0x11]` → zweites Argument = **Gerätenr. des Servers (17)**.
- Danach: `0x7c09, 0x8009, 0x8409, 0x8809 …`, d. h. **+0x400 pro Request**, untere 10 Bit konstant `0x009`. Die Doku beschreibt „bei 1 beginnend, +1“. Der RX3 verhält sich also anders; rekordbox akzeptiert offenbar beides. ◐ Antworten wiederholen die TxID. Nach `0x4101`-Menüpunkten kommt `0x4201` mit **0 Argumenten**.

### 5.6 DMST-Argument (erstes Argument der meisten Requests) ✅
`0xDDMMSSTT` mit **D** = Gerätenr., **M** = Menü (1 Hauptmenü, 2 Untermenü, 7 = ❓ nur bei Root-Request beobachtet, 8 = Grafikdaten laut Doku), **S** = Slot (4 = rekordbox), **T** = Track-Typ (1 = rekordbox).
Beobachtet: `0x0b010401`, `0x0b070401`, `0x0b020401`.

### 5.7 Beobachtete Nachrichtentypen

| Typ | Richtung | Args | Bedeutung |
|---|---|---|---|
| 0x0000 | RX3→PC | `[dev]` | Setup/Intro |
| 0x4000 | PC→RX3 | `[echo_type_or_0, wert]` | Erfolg/„Menü verfügbar“. Bei Setup `[0, 0x11]`; bei Menü-Requests `[request_typ, Anzahl_Einträge]` (z. B. `[0x1000, 9]`, `[0x1002, 0x51f]` = **1311 Artists**) |
| 0x3007 | RX3→PC | `[DMST, 0]` | ❓ Vorbereitung vor Root-Menü (Antwort 0x4000 mit `[0x3007, 0]`) |
| 0x1000 | RX3→PC | `[DMST, 0, 0x05fdffff]` bzw. `0x0b010401, 0, 0x05fdffff` | **Root-Menü-Request** |
| 0x1002 | RX3→PC | `[DMST, 0]` | **Artist-Liste** (nach Doku-Nummerierung: „1004 = vollständige Trackliste“; 0x1002 = Artist-Liste im Mitschnitt) ❓ |
| 0x1102 | RX3→PC | `[DMST(menu=2), 0, artist_id]` | **Alben/Titel eines Artists** |
| 0x3000 | RX3→PC | `[DMST, 0, offset, 0, limit/total, 0]` | **Render-Menü**: holt die eigentlichen Zeilen (`4001`/`4101…`/`4201`) |
| 0x4001 | PC→RX3 | `[1, 0]` | Menü-Header |
| 0x4101 | PC→RX3 | 12 Args | **Menüpunkt** (s. 5.8) |
| 0x4201 | PC→RX3 | – | Menü-Footer |
| 0x0001 | RX3→PC | – | ❓ Abschluss/„Disconnect“ einer Abfrage (kam einmal, tx `0xa409`) |

Beispielablauf „Hauptmenü holen“ (✅ identisch bei t=31,7 und 43,4):
```
RX3→PC  1000  [0x0b010401, 0, 0x05fdffff]                tx=0x9409
PC→RX3  4000  [0x1000, 9]                                  (9 Einträge)
RX3→PC  3000  [0x0b010401, 0, 9, 0, 9, 0]                tx=0x9809
PC→RX3  4001 [1,0] + 9×4101 + 4201                       tx=0x9809
```
Menüeinträge des Hauptmenüs (Reihenfolge): ARTIST, ALBUM, TRACK, KEY, PLAYLIST, HISTORY, SEARCH, MATCHING, DATE ADDED. Beschriftungen sind in `U+FFFA … U+FFFB` eingeklammert (Platzhalter für lokalisierte Namen) → vor Anzeige entfernen. ✅

### 5.8 Menüpunkt 0x4101 (12 Argumente) ✅
Tags `06 06 06 02 06 02 06 06 06 06 06 06`.
| # | Typ | Inhalt |
|---|---|---|
| 1 | u32 | `0` (Parent/reserviert) |
| 2 | u32 | **ID** des Objekts (Artist-ID, Album-ID, Track-ID …; bei Hauptmenü die Menü-ID) |
| 3 | u32 | Länge des Labels in Bytes (Zeichen×2) |
| 4 | string | **Label** (z. B. Artist-Name) |
| 5 | u32 | Länge des 2. Strings (`2` = leer) |
| 6 | string | 2. String, hier leer |
| 7 | u32 | **Item-Typ** (siehe unten) |
| 8–12 | u32 | typabhängig, meist `0`; bei Alben (Typ 2) steht in Arg 9 wieder die ID |

Item-Typen im Mitschnitt: `0x81`…`0x95`, `0xaa` (Hauptmenü-Kategorien), `0x07` (Artist), `0x02` (Album). Öffentlicher Wert: `0x0002 Album`, `0x0004 Titel`, `0x0007 Artist`, `0x000f Key`, `0x0001 Ordner`. ◐

Artist-IDs sind **32-Bit-Hashes** (z. B. `0x56ce3817`), keine fortlaufenden Zahlen. ✅

### 5.9 Parsing-Fallen
- **TCP-Segmentierung:** Eine Antwort kann in einem Segment stehen (Hauptmenü: 1068 Byte) oder über mehrere gehen. Nicht pro Segment parsen, immer erst den Strom zusammensetzen (`djl_parse.reassemble`).
- Retransmissions/Duplikate: nach Sequenznummer deduplizieren.
- Blob mit Länge 0: nicht lesen.
- 0xFFFA/0xFFFB-Steuerzeichen in Strings entfernen.

---

## 6. NFS-Zugriff auf PC-Laufwerke (ONC-RPC über UDP)

Der RX3 mountet freigegebene Laufwerke des PCs (rekordbox stellt einen NFS-Server bereit), um Audio-/Analysedateien (ANLZ) zu lesen. ✅
Alle RPC-Aufrufe: UDP, AUTH_UNIX (Flavor 1, Länge 0x14, Hostname leer, uid=gid=0), Verifier AUTH_NONE.

| Dienst | Ort | Hinweis |
|---|---|---|
| Portmapper (prog 100000 v2) | PC **UDP 50111** (nicht 111!) | `GETPORT` (proc 3) |
| mountd (100005 v1) | PC UDP **36251** (Antwort `00008d9b`) | EXPORT (proc 5), MNT (proc 1) |
| nfsd (100003 v2) | PC UDP **2049** (Antwort `00000801`) | im Mitschnitt kein NFS-Dateiverkehr |

Ablauf (alle 200 ms wiederholt bis Erfolg): `GETPORT(mountd)` → `EXPORT`. Die ersten drei EXPORT-Antworten sind **leer** (t=28,25–28,65), erst ab t=28,85 gibt der PC die Exportliste: `/C/` und `/D/`, Berechtigung `169.254.226.215/255.255.0.0` (ASCII).
**Quirk:** Verzeichnisnamen sind **UTF-16LE** (`2f 00 43 00 2f 00` = `/C/`, Länge 6) – Windows-NFS-Server. Dann `MNT("/C/")` → Antwort mit Status 0 und 32-Byte-FileHandle (für `/C/` komplett 0, für `/D/` beginnend `00000001 00000001 00000001`). Die Bedeutung der Handles ❓.

---

## 7. Hintergrundverkehr (für Pro DJ Link irrelevant) ✅
| Protokoll | Quelle | Inhalt |
|---|---|---|
| ARP | PC | Anfragen nach 169.254.238.85 (24 Pakete) |
| mDNS (5353) | PC | leere Pakete |
| WS-Discovery (3702) | PC | `Resolve`-SOAP, 21 Pakete |
| SSDP (1900) | PC / Chromium | M-SEARCH (IGD, DIAL) |
| DHCP (67) | PC | 1 Discover, unbeantwortet |

---

## 8. Umsetzungsplan (Rust)

1. **Socket-Setup:** UDP 50000 + 50002 (reuse + broadcast), optional 50001 (Beats, hier nicht belegt), Interface mit Link-Local-IP wählen.
2. **Announce:** Keep-Alive (Typ 0x06, 54 Byte, Layout 3.2) alle ≤2 s mit eigener MAC/IP; Gerätenummer **1–4** (kein Konflikt mit 11/12/17).
3. **Listener:** 50000 → Geräteliste (Name, MAC, IP, Nr.); 50002 → Typ 0x0a dekodieren.
4. **Statusauswertung:** BPM nur gültig, wenn `bpm != 0xFFFF`; Pitch-Prozent = `(pitch_raw - 0x100000) / 0x100000 * 100`; Beat nur gültig, wenn `beat != 0xFFFFFFFF`; Track-ID nur gültig, wenn `slot != 0`. Beide Decks (11, 12) getrennt tracken.
5. **Metadaten (optional):** Port per `12523` ermitteln → Greeting → Setup (0x0000, TxID `0xfffffffe`) → Requests (`0x1000/0x3000`, …). **Achtung:** Hier ist der PC der Server. Ob der RX3 selbst dbserver *anbietet* (Standard bei CDJ: ja, Port 1051), ist im Mitschnitt **nicht** belegt, da keine Verbindung *zum* RX3 aufgebaut wird. ❓
6. **Tests:** `djl_parse.py` als Orakel; Ausgabe für diesen Mitschnitt: 215 Status-Pakete, 91 dbserver-Nachrichten, 0 Parserfehler.

## 9. Offene Punkte (nächster Mitschnitt)
- Track laden + abspielen (Beat-Pakete UDP 50001, Position, Play-Zustände, Master/Sync-Flags).
- Verbindung **zum** RX3 (dbserver auf dem Gerät, falls vorhanden) statt zum PC.
- Bedeutung von 0x10/0x30/0x46/0x47/0x16/0x31 und der Felder 0x26/0x68/0xf0–0xfd im Status.
- Keep-Alive-Offsets 0x30–0x35.
- Start-Sequenz des RX3 (Typ 0x0a/0x04 beim Einschalten), also Capture ab Gerätestart.
