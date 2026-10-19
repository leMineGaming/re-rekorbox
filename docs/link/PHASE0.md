# XDJ-RX3 über Link Export – Phase 0: Bestandsaufnahme und Machbarkeit

Stand 2026-10-19. Grundlage: Auftrag „Implementierung des DJ Links“, Codebase `rekordbox-rs`, Messungen am
XDJ-RX3 vom 2026-10-17, öffentliche Quellen (djl-analysis, dysentery, beat-link) und das Rust-Projekt `prolink`
(usr-ein/prolink, GPL-3.0, nur als Dokumentation gelesen).

## 1. Ergebnis in Kürze

| Prüfpunkt | Ergebnis |
|---|---|
| Netzwerkadapter beim Verbinden per USB | **ja**: „Pioneer DJ Network Adapter“ (USB `VID_2B73&PID_0007`), Windows-Name „Ethernet 3“, Link-local 169.254.226.215/16. Er kommt mit dem Pioneer-Treiber; dazu meldet das Gerät Audio, MIDI und eine Hersteller-HID-Schnittstelle (`VID_2B73&PID_003D`). |
| Pult im DJ-Link-Netz sichtbar, Status-Pakete | **ja** (eigene Diagnose `rbx-selftest --link-diag`): Gerät **#11 und #12 „XDJ-RX3“** (Geräteart 7, 169.254.67.96), Keep-alives und CDJ-Status (Typ 0x0A) für beide Decks kommen an. |
| Bibliothek am Pult | **noch nicht**: rekordbox-rs wurde als Quelle „SOFTWARE CONTROL“ angezeigt, das Pult hat aber nie gefragt (keine Port-Abfrage, kein dbserver, kein NFS). |
| Validierung sichtbar? | **nicht belegt.** Die wahrscheinliche Ursache ist bekannt und technisch (siehe 3): wir haben uns wie rekordbox mit **Gerätenummer 17** angemeldet; ein Player fragt nur Geräte mit Nummer **1–4** ab. |

**Stopp-Regel**: nicht ausgelöst. Es gibt keinen Hinweis auf eine Challenge/Response zwischen Player und
Bibliotheksquelle im Netz. rekordbox enthält zwar eine „Geräte-Authentifizierung“ (Dialoge „Authenticate this
device?“, `InnerLinkAPI::receiveAuthStatus`); die Texte beziehen sich auf rekordbox-Funktionen (Cloud, Abo,
Hardware-Unlock). Ob sie den Link-Verkehr betrifft, zeigt erst Phase 1 (Test mit Nummer 1–4) oder der Mitschnitt
(Abschnitt 6). **Wird dabei eine Validierung sichtbar, halte ich an und lege es dir vor.** Kopierschutz,
Lizenz- und Freischaltmechanismen werden in keinem Fall analysiert oder nachgebaut.

## 2. Bestandsaufnahme der Codebase

Workspace (Rust, keine Async-Runtime – Threads + `std::sync::mpsc`):

| Crate | Inhalt (für Link relevant) |
|---|---|
| `rekordbox-formats` (Wurzel) | `djlink` (Paket-Codecs: Keep-alive/Claim 50000, Beat 50001, CDJ- und Mixer-Status 50002, Gerätetabelle), `djlink_participant` (Zustandsautomat Nummernvergabe/Keep-alive, ohne I/O), `djlink_net` (UDP-Sockets, Empfangs-Thread, Beat senden), `netif` (Adapterliste, Auswahl – bevorzugt jetzt den Pioneer-Adapter), `dbserver` (dbserver-Felder und -Nachrichten, Port-Abfrage, Begrüßung), `capture` (pcap/pcapng, TCP-Reassemblierung, Bericht), `anlz` (ANLZ lesen/schreiben: PPTH, PVBR/PVB2, PQTZ, PWAV, PWV2–PWV7, PWVC, PCOB/PCO2, PSSI …), `library` (Bibliothek aus Analyseordnern: `LibTrack` mit Pfad, Titel/Artist, Beatgrid, Cues, Key, Analyseordner), `devicesql` (export.pdb-Strukturen, nur teilweise) |
| `rbx-engine` | Audio-Engine (Mixer, Decks), eigene Analyse (`analyze` → ANLZ-Dateien), `session` (Loader, Bibliothek, DJ Link starten, Log), `linkexport` (Port-Abfrage 12523 + dbserver-Server: Menüs, Metadaten, Beatgrid, Wellenformen, ANLZ-Tags; Client für Tests), `rbx-selftest --link-diag` |
| `rbx-ui` | egui-Oberfläche; Befehle an die Engine über `Command`-Kanal, Zustand über `MixerState`; Einstellungen in `settings.txt` |

Analyse-Daten, die das Pult braucht, liegen bereits im rekordbox-Format vor (eigene Analysen und rekordbox' eigene,
`%APPDATA%\rekordbox-rs\ANLZ` bzw. rekordbox' `USBANLZ`). Phase 3 ist damit zum großen Teil vorhanden.

**Abweichungen von den Qualitätsvorgaben**: Bisheriger Code nutzt eigene Fehlertypen und ein eigenes Log statt
`thiserror`/`tracing`, keine Async-Runtime. Neuer Link-Code bekommt `thiserror` + `tracing`; bestehender Code wird
nur angepasst, wo die Link-Module ihn brauchen.

## 3. Was ein Player verlangt, bevor er eine Quelle durchsucht

Aus `prolink` (Rust, hat diese Server-Seite gegen echte CDJs entwickelt; Befunde F28/F37/F45/F46) und djl-analysis,
in der nötigen Reihenfolge:

1. **Ankündigen** auf UDP 50000 mit Gerätenummer **1–4** (F45: außerhalb davon wird die Ankündigung angenommen,
   aber nie abgefragt – genau unser Befund mit Nummer 17).
2. **Status senden** auf UDP 50002, alle 200 ms per Unicast an jeden Player, mit **Slot-Belegung** (USB/SD).
   Nur darüber erfährt der Player, dass es Medien gibt.
3. **Media-Queries beantworten** (Typ 0x05 → 0x06) mit Track- und Playlist-Zahl.
4. **NFS bereitstellen**, und zwar vor dem dbserver: **Portmapper auf UDP 111** ist Pflicht (F46: ohne ihn fragt
   der Player jede Sekunde `GETPORT` und kommt nie zum dbserver), dazu mountd und nfsd (NFSv2, nur lesend).
   Achtung F28: der Player gibt NFS-Dateihandles nicht unverändert zurück – nur die ersten 12 Byte sind verlässlich.
5. **Port-Abfrage** auf TCP 12523 beantworten.
6. **dbserver** auf dem genannten Port; unbekannte Anfragen nie mit Fehler beantworten. Mehrere Slots über **eine**
   Verbindung, unterschieden pro Nachricht am Slot-Byte (F37).
7. Optional: Einstellungs-Abfrage (LOAD SETTINGS).

Das ist der Weg **„virtueller Player mit eingestecktem Stick“**. Ein Player liest Analysedaten und Audiodateien
dann von unserem „Stick“ – die Bibliothek kommt aus rekordbox-rs, die Wiedergabe macht das Pult. Für die
Wellenform am Pult muss der Stick wie ein rekordbox-Medium aussehen (Track-Art „rekordbox“, `PIONEER/USBANLZ/…`
mit unseren ANLZ-Dateien, ggf. eine erzeugte `export.pdb`); ein Ordner ohne rekordbox-Daten wird nur über die
Ordnerstruktur durchsucht.

**Offen und nur am Gerät klärbar**: ob das XDJ-RX3 (Nummern 11/12, Geräteart 7) die Medien eines anderen Players
unter LINK anbietet. Für CDJs ist das belegt, für All-in-Ones nicht.

## 4. Abweichung vom Auftrag – Entscheidung nötig

Der Auftrag sagt „Präsenz so ankündigen, wie es rekordbox im Link-Export-Modus tut“. rekordbox meldet sich mit
Nummer 17 (Geräteart 4). Genau so wurden wir als „SOFTWARE CONTROL“ angezeigt und nie abgefragt; die belegte
Alternative ist die Ankündigung als **Player mit Nummer 1–4** (virtueller CDJ). Vorschlag: Phase 1 baut den
virtuellen Player; ob zusätzlich die rekordbox-Ankündigung nützt, klärt der Mitschnitt.

## 5. Modulstruktur (Vorschlag)

Neues Workspace-Crate **`rbx-link`** (MIT/Apache, nativ, Threads statt Async-Runtime wie im Rest), das die
vorhandenen Codecs aus `rekordbox-formats` nutzt statt sie zu verschieben:

| Modul | Aufgabe |
|---|---|
| `link::net` | Adapterwahl (Pioneer-Adapter zuerst), Sockets 50000/50001/50002, 111, 2049, 12523, dbserver-Port; Hot-Plug |
| `link::packets` | reine Codecs ohne I/O: vorhandenes `djlink` + neu Media-Query/-Response, Load-Track, Fader-Start, eigener CDJ-Status |
| `link::devices` | Nummernvergabe 1–4, Keep-alive, Gerätetabelle, Timeouts (auf `djlink_participant` aufbauend) |
| `link::rpc` | ONC RPC: Portmapper, mountd, NFSv2 (nur lesend) über eine virtuelle Dateiansicht |
| `link::medium` | „Stick“ aus der rekordbox-rs-Bibliothek: Ordner/Playlists, Track-IDs, `PIONEER/USBANLZ`-Pfade, ggf. `export.pdb` |
| `link::dbserver` | vorhandener Server aus `rbx-engine::linkexport`, umgezogen und an das Medium angepasst |
| `link::bridge` (in `rbx-engine`) | Status der Pult-Decks → UI (Mirror-Ansicht), Befehle aus der UI → Load-Track/Fader-Start |
| `bin/link-sniff` | Pro-DJ-Link-Verkehr live oder aus `.pcapng` dekodiert ausgeben (baut auf `capture` auf) |

## 6. Was du liefern bzw. testen kannst

1. **Optional, Gegenprobe zu unserer Diagnose**: Beat Link Trigger (beat-link) starten, rekordbox geschlossen –
   erscheinen „XDJ-RX3“ #11/#12 und laufen die Statusanzeigen beim Abspielen vom USB-Stick mit? (Unsere eigene
   Diagnose hat beides bereits gezeigt.)
2. **Mitschnitt rekordbox ↔ Pult** mit `Link-Mitschnitt.cmd` (Szenarien: Verbindungsaufbau, Browsen, Track laden,
   30 s spielen, Cue/Loop setzen, Trennen). Er zeigt, wie rekordbox sich anmeldet, ob es eine Validierung gibt, wie
   das Pult Audio abruft. Wireshark ist auf diesem Rechner nicht installiert; das Skript nimmt Wiresharks `dumpcap`,
   falls vorhanden, sonst Windows' `pktmon`.
3. Nach Phase 1: ein Test am Pult, ob unser virtueller Player unter LINK erscheint.
