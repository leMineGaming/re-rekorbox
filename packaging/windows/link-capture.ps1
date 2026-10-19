# Guided capture of rekordbox (LINK EXPORT) <-> XDJ-RX3 traffic on the unit's USB network adapter
# ("Pioneer DJ Network Adapter"). Uses Wireshark's dumpcap when installed (only that adapter), otherwise
# Windows' pktmon (all adapters; administrator rights, the script asks).
# Output: %APPDATA%\rekordbox-rs\capture\<time>\
#   capture.pcapng   raw capture        markers.txt   time of every scenario step
#   linkdump.txt     decoded Pro DJ Link / dbserver / NFS traffic (made by rbf.exe if it is next to this script)
param([string]$Out)

$ErrorActionPreference = 'Stop'
$dumpcap = @("$env:ProgramFiles\Wireshark\dumpcap.exe", "${env:ProgramFiles(x86)}\Wireshark\dumpcap.exe") | Where-Object { Test-Path $_ } | Select-Object -First 1
$admin = ([Security.Principal.WindowsPrincipal][Security.Principal.WindowsIdentity]::GetCurrent()).IsInRole([Security.Principal.WindowsBuiltInRole]::Administrator)
if (-not $dumpcap -and -not $admin) {
    # pktmon needs administrator rights
    Start-Process powershell.exe -Verb RunAs -ArgumentList "-NoProfile -ExecutionPolicy Bypass -File `"$PSCommandPath`""
    exit
}
if (-not $Out) { $Out = Join-Path $env:APPDATA ("rekordbox-rs\capture\" + (Get-Date -Format 'yyyy-MM-dd-HH-mm-ss')) }
New-Item -ItemType Directory -Force $Out | Out-Null
$pcap = Join-Path $Out 'capture.pcapng'
$markers = Join-Path $Out 'markers.txt'

$adapter = Get-NetAdapter -ErrorAction SilentlyContinue | Where-Object { $_.InterfaceDescription -match 'Pioneer|AlphaTheta' } | Select-Object -First 1
if ($adapter) {
    Write-Host "Pult-Adapter: $($adapter.Name) ($($adapter.InterfaceDescription)), Status $($adapter.Status)"
} else {
    Write-Host 'Kein Pioneer-Netzwerkadapter gefunden - ist das XDJ-RX3 per USB verbunden und eingeschaltet?' -ForegroundColor Yellow
}

function Mark([string]$step) {
    $line = "{0}  {1}" -f (Get-Date).ToUniversalTime().ToString('yyyy-MM-dd HH:mm:ss.fff'), $step
    Add-Content -Encoding utf8 $markers $line
    Write-Host "[markiert] $step" -ForegroundColor DarkGray
}

function Step([string]$text) {
    Write-Host ''
    Write-Host $text -ForegroundColor Cyan
    Read-Host 'Enter, wenn erledigt' | Out-Null
}

Write-Host ''
Write-Host 'Vorbereitung: rekordbox und rekordbox-rs geschlossen, XDJ-RX3 per USB verbunden und eingeschaltet.' -ForegroundColor Cyan
Read-Host 'Enter = Aufnahme starten' | Out-Null

$proc = $null
if ($dumpcap) {
    $iface = if ($adapter) { $adapter.Name } else { '1' }
    $proc = Start-Process -FilePath $dumpcap -ArgumentList @('-i', "`"$iface`"", '-w', "`"$pcap`"", '-q') -PassThru -WindowStyle Hidden
    Start-Sleep 2
    if ($proc.HasExited) { throw "dumpcap konnte nicht auf '$iface' aufnehmen (Npcap installiert?)" }
} else {
    pktmon filter remove | Out-Null
    pktmon start --capture --pkt-size 0 --file-name (Join-Path $Out 'capture.etl') | Out-Null
}
Write-Host 'Aufnahme laeuft.' -ForegroundColor Green
Mark 'Aufnahme gestartet'

Step '1. Verbindungsaufbau: rekordbox starten, EXPORT-Modus waehlen; am XDJ-RX3 die Quelle LINK / rekordbox waehlen, bis die Bibliothek erscheint.'
Mark '1 Verbindung aufgebaut'
Step '2. Browsen: am Pult durch Playlists, Ordner, Track-Liste und eine Sortierung blaettern, eine Suche ausprobieren.'
Mark '2 Browsen fertig'
Step '3. Track laden: einen Track auf Deck 1 laden, dann einen anderen auf Deck 2.'
Mark '3 Tracks geladen'
Write-Host ''
Write-Host '4. Playback: Enter druecken und sofort PLAY auf Deck 1 - das Skript wartet 30 s.' -ForegroundColor Cyan
Read-Host 'Enter = Play' | Out-Null
Mark '4 Play Deck 1'
Start-Sleep 30
Mark '4 30 s gespielt'
Step '5. Cue/Loop: einen Hot Cue setzen und anspringen, einen Loop setzen und verlassen, dann Pause.'
Mark '5 Cue und Loop fertig'
Step '6. Trennen: in rekordbox die Verbindung beenden bzw. rekordbox schliessen; am Pult eine andere Quelle waehlen.'
Mark '6 getrennt'

if ($proc) {
    Stop-Process -Id $proc.Id -ErrorAction SilentlyContinue
    Start-Sleep 1
} else {
    pktmon stop | Out-Null
    pktmon etl2pcap (Join-Path $Out 'capture.etl') --out $pcap | Out-Null
    Remove-Item (Join-Path $Out 'capture.etl') -ErrorAction SilentlyContinue
}
Mark 'Aufnahme beendet'

$rbf = Join-Path $PSScriptRoot 'rbf.exe'
if ((Test-Path $rbf) -and (Test-Path $pcap)) {
    & $rbf linkdump $pcap | Out-File -Encoding utf8 (Join-Path $Out 'linkdump.txt')
    Write-Host "Ausgewertet: $(Join-Path $Out 'linkdump.txt')"
}
Write-Host ''
Write-Host "Bitte schicken: capture.pcapng, markers.txt und linkdump.txt aus $Out" -ForegroundColor Green
if (-not $dumpcap) { Write-Host 'Hinweis: pktmon hat alle Adapter aufgenommen; capture.pcapng enthaelt daher auch anderen Netzwerkverkehr.' }
Start-Process explorer.exe $Out
Read-Host 'Enter zum Schliessen' | Out-Null
