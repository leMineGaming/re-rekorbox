# Mitschnitt 1: rekordbox 7.2.19 ↔ XDJ-RX3 (Link Export über USB) – Befunde

Mitschnitt vom 2026-10-19 (pktmon, 160 s, Szenarien laut `markers.txt`: Verbindung, Browsen, zwei Tracks laden,
30 s Play, Cue/Loop, Trennen). PC = 169.254.226.215 („Pioneer DJ Network Adapter“), Pult = 169.254.67.96.
Ausgewertet nur der Verkehr zwischen diesen beiden Adressen. Lücken: pktmon hat einen Teil großer TCP-Segmente
nicht aufgezeichnet (84 Stellen im dbserver-Antwortstrom); der Auswerter setzt nach Lücken an der nächsten
Nachricht wieder an (`decode_stream_resync`).

## Ergebnis Stopp-Regel

**Keine Validierung sichtbar.** Alle Pakete sind Klartext-Abfragen mit festen oder beschreibenden Inhalten;
nirgends ein Austausch mit Zufalls- oder Schlüsseldaten. Die Bibliothek wird ohne Vorbedingung abgefragt, sobald
rekordbox auf die Media-Query antwortet.

## 1. Anmeldung (UDP 50000)

rekordbox: Nummer **17 (0x11)**, Geräteart **4**, Name `rekordbox`; Claim-Pakete 0x00/0x02, dann Keep-alive 0x06
(296 in 160 s). Das Pult: Keep-alive 0x06 als **#11** (Geräteart 7, Name `XDJ-RX3`). Die Regel „Nummer 1–4“
(prolink F45) gilt für Player, nicht für rekordbox – rekordbox wird mit 17 durchsucht.

## 2. Pakete auf UDP 50002 (Typ: Bedeutung, beobachtet)

| Richtung | Typ | Inhalt (ab Byte 0x0A) |
|---|---|---|
| Pult → PC | **0x05** Media-Query (10×) | `…00 01 00 0b 00 0c` + IP des Pults (a9 fe 43 60) + `00 00 00 11` (Dr = 17) + `00 00 00 04` (Slot 4 = rekordbox-Sammlung) |
| PC → Pult | **0x06** Media-Response (8×, 192 Byte) | Name `rekordbox` (UTF-16BE), …, Track-Anzahl **0x071f** (1823), Tr = 1, Playlist-Anzahl 0x14 (20) |
| Pult → PC | 0x10 (5×) | leere Anfrage |
| PC → Pult | 0x11 (4×, 296 Byte) | Rechnername UTF-16 („DESKTOP-…“) und Nullen |
| Pult → PC | 0x30 (15×) | leere Anfrage |
| PC → Pult | 0x31 (8×) | `00 08 06 00 00 …` |
| Pult → PC | 0x46 (10×) | `00 04 0b 04 00 00` |
| PC → Pult | 0x47 (4×, 72 Byte) | `00 24 11 04 00 00 12 34 56 78 00 00 00 01 01 01 03 01 01 01 …` (feste Werte) |
| PC → Pult | 0x16 (28×, 48 Byte) | Kopf + Nullen |
| PC → Broadcast | **0x29** (5820×, 56 Byte) | Status von rekordbox: `11 00 00 c0 00 10 00 00 …`, mit Tempo (z. B. `34 bc` = 135,00) und `09 ff 00` |
| Pult → PC | 0x0A CDJ-Status (7055×), 0x1a (20×) | Status beider Decks; 0x1a `00 04 0b 01 00 00` |

## 3. Datenbank (TCP)

Port-Abfrage auf 12523 (4×), dann zwei dbserver-Verbindungen zu Port 59930 (vom Pult aus Port 46383/46385,
Kontext `CMD_INFO_PLAYER_ID` mit 0x0b bzw. 0x0c = Deck 1 und 2, Antwort `[0, 0x11]`).

**Wurzelmenü** (`CMD_ROOT_MENU`, 9 Einträge; Name zwischen U+FFFA/U+FFFB, Länge in Byte inkl. NUL, Text 2 leer):

| ID | Name | Typ |
|---|---|---|
| 0x02 | ARTIST | 0x81 |
| 0x03 | ALBUM | 0x82 |
| 0x04 | TRACK | 0x83 |
| 0x0c | KEY | 0x8b |
| 0x05 | PLAYLIST | 0x84 |
| 0x16 | HISTORY | 0x95 |
| 0x12 | SEARCH | 0x91 |
| 0x1a | MATCHING | 0xaa |
| 0x1b | DATE ADDED | 0x8c |

Weitere Anfragen des Pults (Anzahl): `SONGDATA_TRACK` 78, `LEFT_BUF` ~190, `ALBUM_ARTIST` 55,
`GET_SPECIFIED_ATOM_INFO` 59, `GET_IMAGE` 51, `TRACK_ALBUM` 17, `TRACK_PLAYLIST` 17, `SONGDATA_TRACK2` 9,
`SEARCH` 4, `GET_WAVEDATA`, `GET_VBR_INFO`, `GET_USB_CUE2`, `GET_QUANTIZE_DATA` je 4, dazu 0x0001 (Cancel, 39),
0x3100 (`CMD_GET_OFFSET_CONTENTID`, 16), 0x2d04, 0x2103, 0x1708, 0x1808, 0x2705, 0x3007, 0x3d03, 0x1012.
`ALBUM_ARTIST` liefert ggf. zuerst `\uFFFAALL\uFFFB` (ID 0xffffffff, Typ 0xa0), dann Alben (Typ 2, Artwork-ID = Album-ID).

**Dateipfad**: `CMD_SONGDATA_TRACK2` (0x2102) → 7 Einträge: Typ 4 (ID 5), Typ 0x0b Dauer (166 s), Typ 0x0d Tempo
(0x34bc), Typ 0x23 (ID = Track-ID), **Typ 0 mit Text 1 = voller Pfad `C:/Users/…/….flac` und Eltern-ID =
Dateigröße** (0x14016dd), Typ 0x2f (1), Typ 0x0f (0x7fffffff).

## 4. Audiodatei (ONC RPC über UDP)

| Schritt | Beobachtung |
|---|---|
| Portmapper | **UDP 50111** (nicht 111): `GETPORT` mount v1 → 36251, nfs v2 → 2049 |
| mountd | `EXPORT` (proc 5) wird regelmäßig gefragt, Antwort **leere Liste**; `MNT "/C/"` und `MNT "/D/"` – **Pfade in UTF-16LE** – liefern Dateihandles (C: 32 Nullbyte; D: `00000001 00000001 00000001 0…`); am Ende `UMNT` |
| nfsd 2049 | `LOOKUP` je Pfadteil, **Namen UTF-16LE** (`Users`, `domi`, `Music`, …); Handles fortlaufend (`00000002…`, `00000003…`); `READ` je **8192 Byte** (11 165 Aufrufe), Attribute: Datei-Modus 0x8124, Größe = Dateigröße |

## 5. Folgerungen für die Umsetzung

- Anmeldung wie rekordbox (Nummer 17, Art 4) ist richtig; was fehlte: Antworten auf **0x05, 0x10, 0x30, 0x46**,
  periodisches **0x29** (und 0x16), Portmapper auf **50111**, mountd, nfsd.
- Wurzelmenü und Metadaten wie oben; der Pfad kommt aus `SONGDATA_TRACK2`.
- NFS mit UTF-16LE-Namen und Laufwerks-Exporten `/C/`, `/D/` …; Dateihandles selbst vergeben.
