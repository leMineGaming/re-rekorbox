# Phase 1 – Netzwerkbasis: Anmeldung wie rekordbox

Stand 2026-10-19. Crate `link/` (`rbx-link`), unabhängig von Engine und UI.

## Module

| Modul | Inhalt |
|---|---|
| `packets` | Pakete bauen und lesen (rein, ohne Sockets): Discovery, Claim, Keep-alive, Abmeldung; Antworten 0x11, 0x31, 0x47, 0x06, 0x16; Status 0x29 mit Master-Tempo; `parse_status_port` für die Anfragen des Pults; `describe` für `link-sniff` |
| `devices` | `Presence`: zeitgesteuerte Zustandsmaschine (Aufrufer gibt `now` vor) – Nummer beanspruchen, Konflikte, Gerätetabelle mit Timeout, Antworten auf Port 50002 |
| `net` | Adapterwahl (Pioneer/AlphaTheta-Adapter zuerst), drei UDP-Sockets, Thread `rbx-link`, Hot-Plug (Adapter alle 2 s geprüft), Ereignisse über Kanal |
| `rpc` | Portmapper (UDP 50111) und mountd (UDP 36251): GETPORT, Exportliste, MNT, UMNT – rein, byte-gleich zu rekordbox; nur für Geräte im Subnetz des Link-Adapters |
| `error` | `LinkError` (Port belegt → „läuft rekordbox?“), `PacketError` |

Werkzeuge:

- `cargo run -p rbx-link --bin link-sniff -- --pcap DATEI [--db]` – Mitschnitt dekodieren (UDP 50000–50002, mit `--db` die dbserver-Ströme); pktmon-Doppel innerhalb 3 ms werden einmal gezeigt.
- `cargo run -p rbx-link --bin link-sniff -- [--seconds N]` – live mithören (passiv, rekordbox muss geschlossen sein).
- `cargo run -p rbx-link --bin link-announce -- [--seconds N] [--adapter NAME|IP] [--tracks N] [--playlists N]` – anmelden wie rekordbox und protokollieren, was das Pult tut. Mit `RUST_LOG=rbx_link=trace` auch jedes gesendete Paket.

## Verhalten (aus Mitschnitt 1, mit `link-sniff` nachgemessen)

| Schritt | rekordbox 7.2.19 | rbx-link |
|---|---|---|
| Discovery 0x00 | 3×, 0,22 s Abstand | gleich |
| Claim 0x02 | 6 Runden über 17, 18, 41, 42, 43, 44 (Zähler 1–6), 0,1 s Abstand = 36 Pakete | gleich; behalten wird die erste freie Nummer (17) |
| Keep-alive 0x06 | alle 2,0 s; Peers = sich selbst + beide Decks (3) | gleich |
| Status 0x29 (Broadcast 50002) | alle 0,1 s, 56 Byte, Master-Tempo optional | gleich |
| 0x16 an jedes Pult | beim Erscheinen, nach 4,3 s, dann nach ~137 s | gleich |
| 0x10 → 0x11 (Geräteinfo, Hostname UTF-16) | sofort | gleich |
| 0x30 → 0x31, 0x46 → 0x47 | erst wenn das Pult in der Tabelle steht (das Pult fragt nach 5 s erneut) | gleich |
| 0x05 → 0x06 (Medien, Slot 4) | nur wenn die Anfrage an uns gerichtet ist | gleich |
| Abmeldung 0x08 | beim Beenden | gleich |

Alle Antworten sind gegen die Bytes aus Mitschnitt 1 getestet (byte-gleich bis auf den Hostnamen). Erste
Auswertung hatte die Zeiten um Faktor 10 zu klein angesetzt; korrigiert und per Test abgesichert.

Hinweis zu CAPTURE1.md: Zählungen dort („0x05 10×“) enthalten pktmon-Doppel; ohne Doppel kam 0x05 einmal.

## Live-Test (2026-10-19, rekordbox geschlossen, XDJ-RX3 per USB)

```
 0.02 adapter Ethernet 3 (Pioneer DJ Network Adapter, 169.254.226.215)
 1.36 device #11 XDJ-RX3 / #12 second deck
 4.18 Claimed(17)          -> 0x16 an das Pult
 4.29 0x30 -> 0x31, 0x10 -> 0x11
 4.79 0x46 -> 0x47
 8.48 zweites 0x16
      Keep-alive alle 2 s, Status beider Decks kommt an, beim Ende 0x08
```

Das Pult nimmt uns an und fragt die Geräteinfos ab wie bei rekordbox. **Keine Validierung sichtbar.**
Die Medienabfrage 0x05 kam ohne Bedienung am Pult nicht – im Mitschnitt kam sie, als am Pult LINK gewählt
wurde. Das ist der offene Punkt für den Test am Gerät.

## Nachtrag: ohne mountd keine Quelle (2026-10-19)

Am Pult erschien keine Quelle, und die Medienabfrage 0x05 kam nie. Grund laut Mitschnitt: 200 ms nach jedem
0x16 fragt das Pult den **Portmapper auf UDP 50111** (GETPORT mountd v1 → 36251) und die **mountd-Exportliste**.
Bei rekordbox war die Liste zuerst leer (Bibliothek noch nicht bereit), das Pult gab nach 1,8 s auf. Nach dem
zweiten 0x16 war sie gefüllt: `/C/`, `/D/` (UTF-16LE), Gruppe `169.254.226.215/255.255.0.0`. Dann folgten
GETPORT nfs v2 → 2049, MNT `/C/` (Handle 32 × 0) und MNT `/D/` (Handle 1, 1, 1), danach 0x05 und TCP 12523.

Dieser kleine Teil von Phase 5 ist deshalb vorgezogen (`rpc`). Dateien lesen (nfsd 2049) bleibt bei Phase 5.
Freigegeben werden die Laufwerke aus `LinkConfig::drives` (link-announce: `--drives`, Standard C).

Live danach:

```
 4.18 Claimed(17)
 4.38 rpc GETPORT mountd, EXPORT, GETPORT nfs, MNT /C/ ok
 4.40 request 0x05 -> Medienantwort 0x06
```

## Offen

- Live-Test am Pult: erscheint der PC jetzt in der Quellenwahl? (0x05/0x06 laufen; Browsen geht erst mit
  Phase 4 – das Pult fragt danach den dbserver auf TCP 12523, der noch fehlt.)
- `link::bridge` (Anbindung an Engine/UI) folgt mit Phase 2.
- Bibliothekszahlen (Tracks/Playlists in 0x06) kommen ab Phase 3 aus der echten Bibliothek.
