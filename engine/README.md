# rbx-engine – Audio-Engine und DJ-Controller-Anbindung

Zwei-Deck-Audio-Engine für das rekordbox-rs-MVP mit vollständiger Anbindung des Pioneer DJ XDJ-RX3.
**Eigenentwurf**: Das Klang- und Bedienverhalten ist nicht aus dem rekordbox-Code abgeleitet (dort wurde
keine Audio-Engine analysiert). Aus der Spezifikation stammen nur die MIDI-Schicht (Mapping-CSV,
Werteverarbeitung je Steuerelement, SPEC §5f/§5f-2) und die Funktionsnamen des Mappings.

## Umfang

| Modul | Inhalt |
|---|---|
| `pcm` | Dekodieren in den Speicher (FLAC, MP3, WAV, AAC/M4A, Ogg Vorbis über `symphonia`), Hermite-Interpolation, Testsignale, WAV-Writer |
| `deck` | Play/Pause, CUE, 8 Hot Cues, Loops (In/Out/Reloop, Beat-Loop, ½/×2), Beat Jump, Tempo ±6/10/16/100 %, Jog (Scratch/Bend/Feinsuche), Needle, Quantize, **Reverse, Slip, Master Tempo (Key Lock), Key Shift, Keyboard-Modus, Vinyl Brake/Start, Memory Cues (Call/Delete), Auto Cue, Suchlauf** |
| `dsp` | Biquads (RBJ), **3-Band-Isolator-EQ** (Kill … +6 dB, Linkwitz-Riley 300 Hz / 4 kHz), Delay, Hall, Rauschen, granularer Pitch-Shifter, **Sound Color FX**: FILTER, SPACE, DUB ECHO, SWEEP, NOISE, CRUSH |
| `fx` | **Beat FX** (14): DELAY, ECHO, PINGPONG, SPIRAL, HELIX, REVERB, FLANGER, PHASER, FILTER, TRANS, ROLL, SLIP ROLL, PITCH, VINYL BRAKE; Beat-Raster 1/16 … 16, Adjust Time, Tap/Auto-BPM, Zuweisung CH1/CH2/Master/Sampler; **Pad FX** (2 × 16 Presets) |
| `sampler` | 16 One-Shot-Slots auf eigenem Bus; aus einem Ordner oder 16 eingebaute synthetische Klänge |
| `mixer` | Kanalzug Trim → EQ → Color FX → Pad FX → Beat FX → Fader, Crossfader mit Kurve, Master mit Soft-Clipping, Sync mit Master-Deck, Instant Double, **Kopfhörer-Cue-Bus** (Pre-Fader, Master Cue, Cue Link) |
| `browser` | Ordner-Browser: Browse/Öffnen/Zurück, Tag-Liste, „Related“ (gleicher Interpret), Track ◀▶ |
| `output` | Echtzeit-Ausgabe (`cpal`/WASAPI); Geräte mit ≥ 4 Kanälen bekommen den Kopfhörer-Cue auf Kanal 3/4 |
| `control` | Textbefehle und zeitgesteuerte Skripte |
| `controller` | MIDI → Engine-Befehl über das Mapping, LED-/Anzeige-Ausgabe |

## Ausprobieren

```bash
cargo run --release -p rbx-engine --bin rbx-play -- devices
cargo run --release -p rbx-engine --bin rbx-play -- play "C:\Pfad\track1.flac" "C:\Pfad\track2.mp3"
cargo run --release -p rbx-engine --bin rbx-play -- render mix.wav 30 click:128 click:124 --script "0:play 1; 0:bpm 2 124; 8:play 2; 8:sync 2; 16:xf 1"
```

Wichtige Textbefehle im `play`-Modus (vollständige Liste in `src/control.rs`): `play 1`, `cue 1`, `hot 1 3`,
`loop 1 4`, `jump 1 -4`, `tempo 1 0.25`, `sync 2`, `eq 1 low 0`, `cfx filter` + `color 1 0.2`,
`fx echo` + `fx on` + `fxdepth 0.6`, `padfx 1 1 5`, `reverse 1 on`, `slip 1 on`, `mt 1 on`, `key 1 -2`,
`brake 1 1.5`, `mem 1` / `call 1 next`, `sample 3`, `hp 1 on`, `xfcurve 0.8`, `load 2 <datei>`, `browse`,
`state`, `quit`.

## Pioneer DJ XDJ-RX3

```bash
cargo run --release -p rbx-engine --bin rbx-play -- midi-coverage "C:\Program Files\rekordbox\rekordbox 7.2.19\MidiMappings\XDJ-RX3.midi.csv"
cargo run --release -p rbx-engine --bin rbx-play -- play --midi "C:\Program Files\rekordbox\rekordbox 7.2.19\MidiMappings\XDJ-RX3.midi.csv" --device XDJ-RX3
```

Optionen: `--midi-in`/`--midi-out NAME` (sonst Suche nach dem Gerätenamen aus dem Mapping), `--browse DIR`
(Browser-Wurzel, Standard: Musik-Ordner), `--samples DIR` (Sampler-Slots, Rest eingebaute Klänge),
`--device NAME` (Audio-Ausgang; bei einem 4-Kanal-Interface Master auf 1/2 und Kopfhörer auf 3/4).
Ohne Gerät lassen sich Nachrichten mit `midi 90 00 7F` eintippen.

**Abdeckung: alle 220 Funktionen** des mitgelieferten Mappings: 211 mit Aktion, 9 bewusst ohne
(`JogPitchBend` = gleiche Nachricht wie `JogScratch`; `JogTouch+Shift`; Geräteparameter `JogRatency`,
`MidiOutInterval`; `DeckState`; `InputSelect` und `AuxInputSelect.*` werden im Gerät geschaltet).
LEDs/Anzeigen: Transport, Cue, Hot-Cue-, Pad-FX-, Sampler-, Key-Shift- und Keyboard-Pads, Loop, Sync,
Quantize, Vinyl, Reverse, Slip (inkl. Blinken), Master Tempo, Master, Pad-Modus-Tasten, Color-FX-Tasten,
Beat FX an/Tap/Auto, Kopfhörer-Cue, Ladeanzeige, Kanalpegel.

**Eigene Festlegungen** (das Mapping gibt sie nicht vor; änderbar in `ControllerConfig` bzw. `fx::PAD_FX`):
Beat-Loop-Pads ¼ … 32 Beats, Beat-Jump-Paare 1/2/4/8 Beats, Key-Shift-Pads ±1 … ±4 (Shift: ±5/6/7/12),
Keyboard-Pads 0 … +7 (Shift −8 … −1), Pad-FX-Presets, Gain ±12 dB, Vinyl Brake bis 3 s, Suchlauf ×4,
Sampler-Pads Deck 1 = Slots 1–8 / Deck 2 = Slots 9–16, „Link Cue“ = immer nur ein Kanal im Kopfhörer,
„Related Tracks“ = gleicher Interpret im selben Ordner.

**Am echten Gerät noch nicht geprüft**: Tempo-Fader-Richtung (`tempo_invert`), Pad-LED-Farben (es wird nur
an/aus gesendet), Jog-Auflösung (`jog_ticks_per_rev`), Kanalbelegung des Kopfhörerausgangs.

## Tests

`cargo test -p rbx-engine`: 55 Tests mit synthetischem Audio und synthetischen MIDI-Daten (kein Audio- oder
MIDI-Gerät nötig): Decoder, Deck, Loops, Sync, Jog, EQ-Frequenzgang, Color FX, alle 14 Beat FX
(Zeitpunkt, Gate-Anteil, Tonhöhe, Bremse), Tap-Tempo, Reverse, Slip, Key Lock, Key Shift, Bremse,
Memory Cues, Auto Cue, Sampler, Cue-Bus, Crossfader-Kurve, Browser und die Controller-Zuordnung samt LEDs.

**Nicht geprüft: wie es klingt** – dafür muss jemand zuhören.

## Grenzen

- Ganze Dateien im Speicher; kein Streaming.
- Key Lock / Key Shift über einen einfachen granularen Pitch-Shifter (hörbare Artefakte bei großen Werten).
- Effekte sind eigene, einfache Umsetzungen, keine Nachbildung des Pioneer-Klangs.
- Beatgrid nur manuell (`bpm`) oder vom Click-Track; ANLZ-Beatgrids kommen mit der Bibliothek (MVP-Schritt 1).
- Ohne 4-Kanal-Ausgabegerät ist der Kopfhörer-Cue nicht hörbar.
