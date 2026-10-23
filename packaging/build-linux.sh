#!/usr/bin/env bash
# Builds the Linux package: dist/rekordbox-rs-<version>-linux-x86_64.tar.gz
#
#   packaging/build-linux.sh [--skip-tests]
#
# The release build is placed in its own target folder (target/dist) so that normal
# builds are not invalidated. The tests run first unless --skip-tests is given.
set -euo pipefail

root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$root"

skip_tests=0
if [ "${1:-}" = "--skip-tests" ]; then
    skip_tests=1
elif [ -n "${1:-}" ]; then
    echo "usage: packaging/build-linux.sh [--skip-tests]" >&2
    exit 2
fi

version="$(sed -n 's/^version = "\(.*\)"$/\1/p' ui/Cargo.toml | head -n 1)"
if [ -z "$version" ]; then
    echo "error: could not read the version from ui/Cargo.toml" >&2
    exit 1
fi

name="rekordbox-rs-$version-linux-x86_64"
dist="$root/dist"
stage="$dist/$name"
target_dir="$root/target/dist"
echo "== $name =="

if [ "$skip_tests" -eq 0 ]; then
    echo "-- Tests"
    cargo test --workspace --release --target-dir "$target_dir"
fi

echo "-- Release build"
cargo build --release --target-dir "$target_dir" \
    -p rbx-ui -p rbx-engine -p rekordbox-formats -p rbx-link --bins

rm -rf "$stage"
mkdir -p "$stage"
for bin in rbx-ui rbx-play rbx-analyze rbx-selftest rbf link-announce link-sniff; do
    install -m 755 "$target_dir/release/$bin" "$stage/$bin"
done

cp README.md "$stage/README.md"
cp ui/assets/icon.png "$stage/icon.png"

cat > "$stage/README-LINUX.txt" <<EOF
rekordbox-rs $version für Linux (x86_64)
========================================

Eigenständiger DJ-Player (zwei Decks, Mixer, Effekte, Sampler, eigene Track-Analyse),
Anbindung des Pioneer DJ XDJ-RX3 und Pro DJ Link.

Starten
-------
  ./rbx-ui          Oberfläche
  ./rbf --help      CLI für ANLZ-, XML-, MIDI-, Pad- und DJ-Link-Dateien
  ./rbx-selftest    automatischer Selbsttest mit Bericht
  ./link-announce --help, ./link-sniff --help

Das Programm wird direkt aus diesem Ordner gestartet; es muss nicht installiert werden.
Benötigt werden eine grafische Sitzung (X11 oder Wayland) und ALSA für die Audioausgabe.

Die Pakete sind nicht signiert. Beim ersten Start kann der Dateimanager nachfragen, ob
die Programme ausgeführt werden dürfen.
EOF

tar -C "$dist" -czf "$dist/$name.tar.gz" "$name"
sha256sum "$dist/$name.tar.gz" > "$dist/$name.tar.gz.sha256"

size="$(du -h "$dist/$name.tar.gz" | cut -f1)"
echo "-- fertig: $dist/$name.tar.gz ($size)"
