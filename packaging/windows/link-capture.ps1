# Records the network traffic between rekordbox (LINK EXPORT) and a DJ player such as the XDJ-RX3, so
# that rekordbox-rs can answer the player the same way (SPEC 5k, U33). Uses Windows' built-in pktmon
# (administrator rights needed; the script asks). Output: %APPDATA%\rekordbox-rs\capture\<time>\
#   capture.pcapng   the raw capture (contains all traffic of the computer during the recording)
#   linkdump.txt     decoded Link Export messages (send this one)
param([string]$Out)

$ErrorActionPreference = 'Stop'
$admin = ([Security.Principal.WindowsPrincipal][Security.Principal.WindowsIdentity]::GetCurrent()).IsInRole([Security.Principal.WindowsBuiltInRole]::Administrator)
if (-not $admin) {
    Start-Process powershell.exe -Verb RunAs -ArgumentList "-NoProfile -ExecutionPolicy Bypass -File `"$PSCommandPath`""
    exit
}
if (-not (Get-Command pktmon -ErrorAction SilentlyContinue)) {
    Write-Host 'pktmon fehlt (Windows 10 1809 oder neuer nötig).' -ForegroundColor Red
    Read-Host 'Enter zum Schließen'
    exit 1
}
if (-not $Out) {
    $stamp = Get-Date -Format 'yyyy-MM-dd-HH-mm-ss'
    $Out = Join-Path $env:APPDATA "rekordbox-rs\capture\$stamp"
}
New-Item -ItemType Directory -Force $Out | Out-Null
$etl = Join-Path $Out 'capture.etl'
$pcap = Join-Path $Out 'capture.pcapng'

Write-Host ''
Write-Host 'Mitschnitt rekordbox <-> DJ-Player (LINK EXPORT)' -ForegroundColor Cyan
Write-Host '1. rekordbox starten und in den EXPORT-Modus schalten.'
Write-Host '2. XDJ-RX3 per USB-Kabel (Buchse fuer den Computer) oder LAN verbinden.'
Write-Host '3. Erst JETZT Enter druecken, dann am Pult: Quelle LINK / rekordbox waehlen,'
Write-Host '   durch die Menues blaettern (Track, Artist, Album, Playlist, Suche), zwei Tracks laden,'
Write-Host '   je 20 s abspielen, einen Hot Cue anspringen, ein paar Sekunden warten.'
Write-Host '4. Danach hier wieder Enter druecken.'
Read-Host 'Enter = Aufnahme starten'

pktmon filter remove | Out-Null
pktmon start --capture --pkt-size 0 --file-name $etl | Out-Null
Write-Host 'Aufnahme laeuft ...' -ForegroundColor Green
Read-Host 'Enter = Aufnahme beenden'
pktmon stop | Out-Null
pktmon etl2pcap $etl --out $pcap | Out-Null
Remove-Item $etl -ErrorAction SilentlyContinue

$rbf = Join-Path $PSScriptRoot 'rbf.exe'
if (Test-Path $rbf) {
    & $rbf linkdump $pcap | Out-File -Encoding utf8 (Join-Path $Out 'linkdump.txt')
    Write-Host "Ausgewertet: $(Join-Path $Out 'linkdump.txt')"
}
Write-Host "Mitschnitt: $pcap"
Write-Host 'Bitte linkdump.txt schicken (capture.pcapng enthaelt den gesamten Netzwerkverkehr des Rechners).'
Start-Process explorer.exe $Out
Read-Host 'Enter zum Schließen'
