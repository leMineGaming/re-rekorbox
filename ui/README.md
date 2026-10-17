# rbx-ui – Oberfläche für das rekordbox-rs-MVP

Fenster-Programm (egui/eframe) für die Audio-Engine `rbx-engine`. **Eigenentwurf**, keine Nachbildung der
rekordbox-Oberfläche.

## Starten

```bash
cargo run --release -p rbx-ui
```

Optionen: `rbx-ui [datei1] [datei2] [--device NAME] [--silent] [--midi MAPPING.csv] [--djlink [ADAPTER]]
[--anlz DIR] [--music DIR] [--no-library] [--no-analyze] [--samples DIR]`. Mit `--silent` läuft die Engine ohne
Soundkarte, es ist also nichts zu hören. Wenn kein Audiogerät geöffnet werden kann, startet das Programm ebenfalls
stumm.

## Was es kann

| Bereich | Inhalt |
|---|---|
| Trackliste (unten) | rekordbox-Analysen, eigene Analysen und alle Audiodateien des Musik-Ordners; Suche, Spalten BPM/Key/Herkunft der Analyse; Doppelklick lädt in ein freies oder angehaltenes Deck, `> Deck 1` / `> Deck 2`; **„Fehlende analysieren“** analysiert alle Dateien ohne Analyse im Hintergrund (mit Fortschritt); Dateien und Ordner können ins Fenster gezogen werden |
| Wellenformen (oben) | Detail beider Decks mit Beatgrid (Taktanfang rot), Cue, Hot Cues und Loop. Quelle: `PWV5` aus den Analysedateien (rekordbox oder eigene), sonst aus dem Audio berechnet. Ziehen = Scratch/Scrub, Mausrad = Pitch Bend; Zoom oben rechts |
| Decks | Titel, BPM, Tempo %, Zeit/Restzeit, Tonart, Beat im Takt; Übersichtswellenform (Klick = Sprung); PLAY/PAUSE, CUE (halten = Vorhören), SYNC, MASTER, Quantize, Key Lock, Slip, Reverse, Hot Cues A–H (Rechtsklick löscht), Loop IN/OUT/RELOOP, Beat-Loops 1–16, ½/×2, Beat Jump, Memory Cues, Key Shift, Instant Double, Tempo-Fader (Doppelklick = 0) mit Bereich ±6/10/16/WIDE |
| Mixer | Trim, 3-Band-EQ, COLOR, Kanalfader mit Pegel, Kopfhörer-CUE je Kanal und Master, Master-Pegel, Sampler-Lautstärke, Crossfader mit Kurve |
| Effekte | Beat FX (14 Effekte, Beat-Raster, Zuweisung CH1/CH2/Master/Sampler, Depth, ON), Sound Color FX (6) |
| Sampler | 16 Pads (Klick spielt, Rechtsklick stoppt) |
| Einstellungen | Analyse beim Laden an/aus, Controller-Mapping verbinden (z. B. `XDJ-RX3.midi.csv`), Pro DJ Link beitreten und Decks dem Tempo eines Geräts folgen lassen |

Drehregler: nach oben/unten ziehen, Doppelklick setzt zurück. Unten stehen die letzten Meldungen (Laden, Analyse,
Controller, DJ Link).

## Tests

`cargo test -p rbx-ui`: steuert das Fenster ohne Bildschirm über AccessKit (egui_kittest), mit stummer Engine und
synthetischen Klick-Tracks in einem Temp-Ordner. Ablauf: Track in der Liste wählen → in Deck 1 laden →
automatische Analyse (128 BPM) → PLAY → Hot Cue A → zweiten Track laden → SYNC (Deck 2 auf 128 BPM) → PAUSE;
außerdem die Suche. Bildschirmfotos landen in `target/ui-shots/`.
`cargo test -p rbx-ui --test ui -- --ignored real_library` macht ein Bild mit der echten Bibliothek dieses
Rechners (nur lesend).

**Nicht geprüft:** wie es sich anfühlt und klingt; Darstellung bei sehr kleinen Fenstern; Bedienung mit echtem
Controller und DJ Link (siehe Engine-README).

## Grenzen

- Die Schriften enthalten nur Latin-1; Bedienelemente sind deshalb beschriftet statt mit Symbolen versehen.
- Keine Playlists (rekordbox.xml-Import ist in `rekordbox-formats` vorhanden, aber noch nicht angebunden).
- Tastaturkürzel fehlen.
