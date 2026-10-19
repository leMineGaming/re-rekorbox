# Phase 4 (vorgezogen) – dbserver: Quelle am Pult und Browsen

Stand 2026-10-19. Vorgezogen, weil das Pult die Quelle erst anzeigt, wenn es auf TCP 12523 einen
Datenbank-Port bekommt (siehe PHASE1.md, Nachtrag „Quelle blinkt“).

## Module (rbx-link)

| Modul | Inhalt |
|---|---|
| `catalog` | Bibliothek wie das Pult sie sieht: Tracks mit Ids 1…n (nach Titel), Künstler und Tonarten mit Ids, Analysedaten (Vorschau-Wellenform, Beatgrid, ANLZ-Tags) |
| `dbserver` | `Session`: eine Anfrage → Antworten, rein. Menüs werden vorbereitet (Antwort: Anzahl) und seitenweise mit `CMD_LEFT_BUF` (Offset, Anzahl) geholt. `encode` schreibt wie rekordbox |
| `server` | Portabfrage TCP 12523 (`ff ff` solange keine Bibliothek/Nummer, sonst der Port), Datenbank-Port mit einem Thread je Verbindung; nur das Link-Subnetz |

`link-announce` lädt die Bibliothek (rekordbox-Analysen + eigene aus `%APPDATA%\rekordbox-rs\ANLZ`,
Musikordner `--music` oder `%USERPROFILE%\Music`) und bietet sie an; `Link-Anmeldung.cmd` läuft 15 min.

## Ports (für Firewall-Regeln)

Alle eingehend, nur aus dem Subnetz des Link-Adapters beantwortet:

| Protokoll | Port | Wofür |
|---|---|---|
| UDP | 50000, 50001, 50002 | DJ Link (Anmeldung, Beat, Status) |
| UDP | 50111 | Portmapper |
| UDP | 36251 | mountd |
| UDP | 2049 | NFS (Dateien laden, Phase 5) |
| TCP | 12523 | Portabfrage der Datenbank |
| TCP | 12524 | Datenbank (`server::DB_PORT`, `LinkConfig::db_port`, `link-announce --db-port`) |

rekordbox vergibt den Datenbank-Port bei jedem Start neu (im Mitschnitt 59930). Wir nehmen einen festen,
damit eine Port-Regel reicht. Die Firewall-Freigabe, die Windows beim ersten Start anlegt, gilt nur für das
Netzprofil, das gerade aktiv war. Unter „Privat“ wurde deshalb alles außer den Antworten auf eigene
Broadcasts geblockt (Test 2026-10-19).

## Protokoll, wie rekordbox antwortet (Mitschnitt 1)

- **Antwortformat:** Der Argumenttyp-Block ist nur so lang wie die Argumentzahl (rekordbox), nicht 12 Byte
  wie bei CDJs. Ein leerer Blob wird nicht übertragen.
- **Setup:** `CMD_INFO_PLAYER_ID [Pultnummer]` → `RET_DWORD [0, eigene Nummer]`.
- **Hauptmenü** (`0x1000`) mit 9 Einträgen. Labels stehen in U+FFFA…U+FFFB, die Id ist der Menü-Anfragetyp − 0x1000:

| Eintrag | Id | Typ | Anfrage |
|---|---|---|---|
| ARTIST | 0x02 | 0x81 | 0x1002 |
| ALBUM | 0x03 | 0x82 | 0x1003 |
| TRACK | 0x04 | 0x83 | 0x1004 |
| KEY | 0x0c | 0x8b | 0x100c |
| PLAYLIST | 0x05 | 0x84 | 0x1105 `[dmst, 0, Ordner, 1]` |
| HISTORY | 0x16 | 0x95 | 0x1012 (rekordbox: 0 Einträge) |
| SEARCH | 0x12 | 0x91 | 0x1300 `[dmst, 0, Länge, Text, 1]` |
| MATCHING | 0x1a | 0xaa | 0x3007 |
| DATE ADDED | 0x1b | 0x8c | 0x1708 (Jahre, Typ 0x2e) → 0x1808 (ALL + Monate) |

- **Seite holen:** `CMD_LEFT_BUF [dmst, Offset, Anzahl, 0, Gesamt, x]` → `LIST_START [1, Offset]`, die Einträge, `LIST_END`.
- **Eintrag finden:** `0x3100 [dmst, Id, 0, 0]` → `RET_DWORD [0x3100, Index]` (Position im vorbereiteten Menü).
- **Abbruch:** `0x0001` mit der Transaktions-Id einer laufenden Anfrage; ohne eigene Antwort.
- **Trackzeile** (Typ 4): `[0, Id, Länge, Titel, 2, "", 4, 0x01000000, Artwork, 1, 0x101, 0]`. Album: Typ 2, Artwork = Album-Id. „ALL“: Typ 0xa0, Id 0xffffffff.
- **SONGDATA_TRACK** (`0x2002`): 16 Einträge bei rekordbox. Gesehen wurden Titel (0), Dauer (3) und Tonart (6); wir senden 11 in der bekannten Reihenfolge.
- **SONGDATA_TRACK2** (`0x2102`, vor dem Laden): Dateityp (5 = FLAC), Dauer, BPM × 100, Kommentar, **Pfad** (`parent` = Dateigröße, `C:/…`), Flag 0x2f, Tonart.
- **GET_IMAGE** ohne Bild: `RET_IMAGE [0x2003, 0x32, 0, leer]`.

## Unsere Abweichungen (Bibliothek ohne diese Daten)

- **ALBUM:** leer. Unter ARTIST steht je Künstler ein „ALL“-Eintrag mit allen seinen Tracks.
- **PLAYLIST, HISTORY, MATCHING und DATE ADDED:** leer (noch keine Playlists, kein Hinzufügedatum).
- **Artwork:** keins.
- **Unbekannte Menüs:** 0 Einträge. Unbekannte Datenanfragen (0x2xxx): `RET_UNSUPPORTED`.

## Live (2026-10-19)

```
 4.18 Claimed(17)
 4.4x port query -> Port, Verbindung, PLAYER_ID, 0x3007, ROOT_MENU, LEFT_BUF (9 Einträge)
```

## Tests

`tests/dbserver.rs`:

- Setup, Hauptmenü und Hauptmenü-Seite sind byte-gleich zu rekordbox.
- Abgedeckt sind außerdem Tracks, Blättern, Künstler → ALL → Tracks, Suche, Metadaten, 0x3100, Abbruch, DROP und unbekannte Anfragen.
- Über Loopback laufen die geteilte Portabfrage, `ff ff` vor der Bereitschaft, Begrüßung und Sitzung sowie die Ablehnung fremder Subnetze.

## Offen

- **Test am Pult:** Erscheint die Quelle, gehen TRACK, ARTIST und SEARCH? Kommen Wellenform und Metadaten?
- **Laden** (SONGDATA_TRACK2 → NFS LOOKUP/READ) folgt mit Phase 5.
- **Bibliothek:** Playlists, Alben und Hinzufügedatum fehlen noch (Phase 3).
