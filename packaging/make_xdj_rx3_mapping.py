"""Writes engine/mappings/XDJ-RX3.csv, the built-in controller mapping for the Pioneer DJ XDJ-RX3.

The file uses the mapping format of SPEC 5f (which the engine reads). Its content is the unit's MIDI
implementation (which message each control sends, which message lights which LED), taken from the
mapping that a local rekordbox installation provides for the unit; descriptive comments are not
copied. Run again when a newer rekordbox changes the unit's assignments:

    python packaging/make_xdj_rx3_mapping.py ["C:\\Program Files\\rekordbox\\rekordbox 7.2.19"]
"""
import csv
import glob
import io
import os
import sys

HERE = os.path.dirname(os.path.abspath(__file__))
OUT = os.path.join(HERE, '..', 'engine', 'mappings', 'XDJ-RX3.csv')


def find_source(arg):
    roots = [arg] if arg else sorted(glob.glob(r'C:\Program Files\rekordbox\rekordbox *'), reverse=True)
    for r in roots:
        p = os.path.join(r, 'MidiMappings', 'XDJ-RX3.midi.csv')
        if os.path.isfile(p):
            return p
    sys.exit('XDJ-RX3.midi.csv not found; pass the rekordbox folder')


def main():
    src = find_source(sys.argv[1] if len(sys.argv) > 1 else None)
    rows = list(csv.reader(io.open(src, encoding='utf-8-sig', newline='')))
    out = io.StringIO()
    w = csv.writer(out, lineterminator='\r\n')
    w.writerow(['@file', '1', 'XDJ-RX3'])
    w.writerow(['#name', 'function', 'type', 'input', 'deck1', 'deck2', 'deck3', 'deck4', 'output', 'deck1', 'deck2', 'deck3', 'deck4', 'option', 'comment'])
    w.writerow(['# rekordbox-rs built-in mapping: MIDI implementation of the XDJ-RX3 (see packaging/make_xdj_rx3_mapping.py)'] + [''] * 14)
    n = 0
    for r in rows[2:]:
        if not r or r[0].startswith('@'):
            continue
        r = (r + [''] * 15)[:15]
        if r[0].startswith('#'):
            w.writerow([r[0].split(',')[0]] + [''] * 14)  # section headings
            continue
        if not any(r[1:14]):
            continue
        r[14] = ''  # no descriptive comments
        w.writerow(r)
        n += 1
    os.makedirs(os.path.dirname(OUT), exist_ok=True)
    with io.open(OUT, 'w', encoding='utf-8', newline='') as f:
        f.write(out.getvalue())
    print(f'{n} entries from {src} -> {os.path.normpath(OUT)}')


if __name__ == '__main__':
    main()
