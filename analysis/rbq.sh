#!/usr/bin/env bash
# Query the persistent Ghidra project read-only, without re-analysis.
# Usage: analysis/rbq.sh <name> <command> [args...]   -> writes analysis/out/<name>.txt
set -euo pipefail
here="$(cd "$(dirname "$0")" && pwd -W)"
name="$1"; shift
mkdir -p "$here/out"
# Arguments go through a file so cmd.exe never sees regex metacharacters like | & < >.
argf="$here/out/$name.args"
{ echo "$here/out/$name.txt"; printf '%s\n' "$@"; } > "$argf"
rm -f "$here/out/$name.txt"
export GHIDRA_HEADLESS_MAXMEM=12G
export GHIDRA_HEADLESS_JAVA_OPTIONS="-XX:ParallelGCThreads=12 -XX:ConcGCThreads=4 -XX:CICompilerCount=6"
cmd //c "C:\\Tools\\ghidra_12.1.4_PUBLIC\\support\\analyzeHeadless.bat" "$here/ghidra" rekordbox \
  -process rekordbox.exe -noanalysis -readOnly \
  -scriptPath "$here/ghidra_scripts" \
  -postScript RbQuery.java "@$argf" > "$here/out/$name.log" 2>&1 || true
if [ -f "$here/out/$name.txt" ]; then cat "$here/out/$name.txt"; else echo "## no output; log tail:"; tail -20 "$here/out/$name.log"; fi
