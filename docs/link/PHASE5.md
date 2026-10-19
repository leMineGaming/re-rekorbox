# Phase 5 – NFS-Dateiserver (Audiodateien ans Pult liefern)

Grundlage: `docs/link/PRO_DJ_LINK_SPEC.md` §7 (USB-HID-Mitschnitt, Link-Export). Das Pult liest die
Audiodatei **direkt per NFS v2** vom Host. Kein Audio über USB/Netz, keine Kryptografie – ONC-RPC über
UDP, `AUTH_UNIX`/`AUTH_NULL`, Klartext.

## Modul `link/src/nfs.rs`

`Fs` hält die freigegebenen Laufwerke (aus `LinkConfig::drives`) und eine Handle-Tabelle; `answer(&Fs,
&Call)` beantwortet eine NFS-Anfrage. Nur lesend:

| Proc | Nr. | Umsetzung |
|---|---|---|
| NULL | 0 | leere Antwort |
| GETATTR | 1 | `fattr` aus `std::fs::metadata` |
| LOOKUP | 4 | Pfadkomponente auflösen, neues Handle prägen, `fh + fattr` |
| READ | 6 | bis 8192 Byte ab Offset, `fattr + data` |
| READLINK | 5 | Fehler (kein Symlink) |
| STATFS | 17 | feste Werte |

**Eigenheiten (aus der Spec umgesetzt):**
- **Dateinamen/Pfade sind UTF-16LE** in den XDR-Strings (Windows-NFS-Quirk): `"/C/"` = `2f 00 43 00 2f 00`.
  XDR-Länge zählt Bytes, auf 4 aufgefüllt.
- **Filehandle** (32 Byte, undurchsichtig, vom Pult unverändert zurückgeschickt): Wurzelhandles decken sich
  mit `rpc::root_handle` (ein Handle aus mountd `MNT` wird hier aufgelöst). Für Dateien/Unterordner werden
  beim LOOKUP neue Handles vergeben (High-Bit gesetzt, in `table` gespeichert) – das Schema ist frei wählbar.
- **fattr** (17 Wörter, 68 Byte): Typ 2 Verzeichnis / 1 Datei, Größe und mtime aus der Datei, Rest fest.
- LOOKUP-Antwort `status(0) | fh[32] | fattr(68)`; READ-Antwort `status(0) | fattr(68) | len | data`.
- Große READ-Antworten (8192 Byte) werden vom OS-Socket als IP-Fragmente gesendet (Weg A, OS-NIC: ok).

## Einbindung

`net.rs` bindet zusätzlich **UDP 2049**; `serve_rpc` leitet Datagramme auf 2049 an `nfs::answer`, auf
50111/36251 weiter an `rpc::answer` (Portmapper/mountd). Es wird **ein** `Fs` je Laufwerksauswahl gehalten,
damit die Handle-Tabelle über Datagramme hinweg bestehen bleibt. Weiterhin nur das Link-Subnetz.

## Tests (`link/tests/nfs.rs`)

Gegen ein Temp-Verzeichnis: GETATTR Wurzel = Verzeichnis; LOOKUP `Music` → `Track.flac`; READ ab Offset 0
und ab Offset 8 (liefert die FLAC-Bytes); unbekannter Name → `NFSERR_NOENT`; NULL/STATFS.

## Offen / wichtig

- **Das Laden eines Tracks setzt voraus, dass das Pult den PC als Quelle anzeigt.** Dieser Schritt hängt
  weiterhin am USB-HID-Kanal (Gerät 3, 2B73:003D), der in beiden Mitschnitten **nicht dekodiert** ist
  (Spec §2/§9: [OFFEN]). Solange die Quelle nicht erscheint, fragt das Pult weder dbserver noch NFS ab.
  Der NFS-Server ist damit fertig, aber am Gerät erst nutzbar, wenn der Quell-Schalter gelöst ist.
- Der echte Dateipfad kommt aus der dbserver-Antwort `SONGDATA_TRACK2` (`C:/…/Track.flac`); Mount `/C/`,
  dann Komponenten. Das ist umgesetzt (dbserver liefert den Pfad, NFS löst ihn auf).
- Platzhalter-fattr-Zeiten/-Blocks sind nicht bitgleich zu rekordbox (nicht nötig; das Pult braucht Typ,
  Größe, gültige Struktur).
