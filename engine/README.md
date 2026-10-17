# rbx-engine – Audio-Engine (MVP-Schritt 2)

Zwei-Deck-Audio-Engine für das rekordbox-rs-MVP. **Eigenentwurf**: Das Verhalten ist nicht aus dem
rekordbox-Code abgeleitet (dort wurde keine Audio-Engine analysiert); wo es übliches CDJ-Verhalten
nachahmt, steht das am jeweiligen Code.

## Umfang

| Modul | Inhalt |
|---|---|
| `pcm` | Dekodieren ganzer Dateien in den Speicher (FLAC, MP3, WAV, AAC/M4A, Ogg Vorbis über `symphonia`), Stereo-`f32`, Hermite-Interpolation, Testsignale (Sinus, Click-Track), WAV-Writer |
| `deck` | Play/Pause, CUE (gespielt → zurück und Stopp; pausiert → Cue setzen, Vorhören solange gedrückt), 8 Hot Cues (setzen/anspringen/Vorhören/löschen), Loop In/Out/Reloop, Beat-Loop, Loop halbieren/verdoppeln, Tempo-Fader mit Bereichen ±6/10/16/100 %, Jog (Scratch bei Berührung, Pitch-Bend beim Spielen, Feinsuche pausiert), Needle-Search, Quantisierung auf das Beatgrid, Declick-Rampe |
| `mixer` | 2 Decks, Kanalfader, Trim, Crossfader (konstante Leistung), Master mit Soft-Clipping, **Sync** (Tempo + Beat-Phase), Offline-Rendern |
| `output` | Echtzeit-Ausgabe über das Standard-Audiogerät (`cpal`, unter Windows WASAPI); Steuerung per `Command`-Kanal, Zustand per Snapshot |
| `control` | Textbefehle und zeitgesteuerte Skripte (`0:play 1; 4:sync 2; …`) |

## Ausprobieren

```bash
cargo run --release -p rbx-engine --bin rbx-play -- devices
cargo run --release -p rbx-engine --bin rbx-play -- tone 2
cargo run --release -p rbx-engine --bin rbx-play -- play "C:\Pfad\track1.flac" "C:\Pfad\track2.mp3"
cargo run --release -p rbx-engine --bin rbx-play -- render mix.wav 30 click:128 click:124 --script "0:play 1; 0:bpm 2 124; 8:play 2; 8:sync 2; 16:xf 1"
```

Im `play`-Modus Befehle zeilenweise eingeben: `play 1`, `cue 1` / `cueup 1`, `hot 1 3`, `loop 1 4`,
`reloop 1`, `tempo 1 0.25`, `range 1 16`, `bpm 1 128 0.12` (Beatgrid setzen), `sync 2`, `xf -1`,
`vol 2 0.8`, `seek 1 0.5`, `load 2 <datei>`, `state`, `quit` (vollständige Liste in `src/control.rs`).
Quellen statt Dateien: `tone` (440 Hz) und `click:BPM` (Click-Track mit Beatgrid).

## Tests

`cargo test -p rbx-engine`: 13 Tests mit synthetischem Audio (kein Audiogerät nötig): WAV-Round-Trip
durch den Decoder, Interpolation, Geschwindigkeit bei Tempo und anderer Abtastrate, Pegel/Declick/Track-Ende,
CUE, Hot Cues, Quantisierung, Loops, Sync (Tempo und Phase, auch nach 5 s), Bereichserweiterung, Jog,
Crossfader/Clipping, Befehle und Skripte.

Lokal geprüft (2026-10-17): echte FLAC (127 s) in 0,15 s dekodiert (Release-Build), Skript-Render nach
WAV, Audio-Ausgang geöffnet (Standardgerät, 48 kHz). **Nicht geprüft: wie es klingt** – dafür muss jemand zuhören.

## Grenzen (bewusst fürs MVP)

- Ganze Datei im Speicher (≈ 10 MB pro Minute Stereo-`f32`); kein Streaming.
- Kein Key-Lock (Master Tempo): Tempoänderung verändert die Tonhöhe.
- Kein EQ/Filter, keine Effekte, kein Kopfhörer-Vorhören (Cue-Bus).
- Beatgrid nur manuell (`bpm`) oder vom Click-Track; Anbindung an ANLZ-Beatgrids kommt mit der Bibliothek.
- Beim Laden eines neuen Tracks wird der alte Puffer im Audio-Thread freigegeben (kurzer Aussetzer möglich).
