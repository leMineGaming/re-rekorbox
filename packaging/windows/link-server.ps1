# Link Export server: exposes the rekordbox-rs library to the XDJ-RX3 so the unit shows the PC as
# a source and can browse and LOAD tracks (dbserver + NFS audio). rekordbox must be closed.
#
#   powershell -ExecutionPolicy Bypass -File link-server.ps1 [-Seconds 900] [-Music D:\Musik]
#               [-Drives CD] [-Adapter NAME|IP] [-NoFirewall]
#
# What it does:
#   1. checks that the unit's USB network adapter ("Pioneer DJ Network Adapter") is present
#   2. adds the Windows Firewall rules for the Link Export ports (once; asks for admin once)
#   3. runs link-announce.exe, which joins Pro DJ Link like rekordbox and serves the library
# Output also goes to %APPDATA%\rekordbox-rs\logs\link-server.txt.
param(
    [int]$Seconds = 900,
    [string]$Music = '',
    [string]$Drives = '',
    [string]$Adapter = '',
    [switch]$NoFirewall,
    # internal: run only the firewall part (used for the elevation round-trip)
    [switch]$AddFirewallOnly
)

$ErrorActionPreference = 'Stop'

# ---- firewall -----------------------------------------------------------------------------
$group = 'rekordbox-rs Link Export'

function Test-FirewallRules {
    $null -ne (Get-NetFirewallRule -Group $group -ErrorAction SilentlyContinue)
}

function Add-FirewallRules {
    Get-NetFirewallRule -Group $group -ErrorAction SilentlyContinue | Remove-NetFirewallRule
    New-NetFirewallRule -DisplayName "$group (UDP: DJ Link, Portmapper, NFS)" -Group $group -Direction Inbound -Action Allow -Protocol UDP -LocalPort 50000,50001,50002,50111,36251,2049 | Out-Null
    New-NetFirewallRule -DisplayName "$group (TCP: dbserver, Port-Abfrage)" -Group $group -Direction Inbound -Action Allow -Protocol TCP -LocalPort 12523,12524 | Out-Null
    Write-Host 'Firewall-Regeln angelegt (eingehend: UDP 50000-50002/50111/36251/2049, TCP 12523/12524).' -ForegroundColor Green
}

if ($AddFirewallOnly) {
    Add-FirewallRules
    exit 0
}

$admin = ([Security.Principal.WindowsPrincipal][Security.Principal.WindowsIdentity]::GetCurrent()).IsInRole([Security.Principal.WindowsBuiltInRole]::Administrator)
if (-not $NoFirewall -and -not (Test-FirewallRules)) {
    if (-not $admin) {
        Write-Host 'Firewall-Regeln fehlen - einmalige Freigabe (Administrator) wird angefordert ...' -ForegroundColor Yellow
        Start-Process powershell.exe -Verb RunAs -Wait -ArgumentList "-NoProfile -ExecutionPolicy Bypass -File `"$PSCommandPath`" -AddFirewallOnly"
    } else {
        Add-FirewallRules
    }
}

# ---- locate the binary --------------------------------------------------------------------
$exe = Join-Path $PSScriptRoot 'link-announce.exe'
if (-not (Test-Path $exe)) { $exe = Join-Path (Join-Path $PSScriptRoot '..\..') 'target\release\link-announce.exe' }
if (-not (Test-Path $exe)) {
    Write-Host 'link-announce.exe nicht gefunden. Im Quellordner zuerst bauen:' -ForegroundColor Red
    Write-Host '  cargo build --release -p rbx-link'
    Read-Host 'Enter zum Schliessen' | Out-Null
    exit 1
}

# ---- pre-flight checks --------------------------------------------------------------------
$adp = Get-NetAdapter -ErrorAction SilentlyContinue | Where-Object { $_.InterfaceDescription -match 'Pioneer|AlphaTheta' } | Select-Object -First 1
if (-not $adp) {
    Write-Host 'Kein "Pioneer DJ Network Adapter" gefunden.' -ForegroundColor Yellow
    Write-Host '  - XDJ-RX3 per USB angeschlossen und eingeschaltet? (Buchse "PC" auf der Rueckseite)'
    Write-Host '  - XDJ-RX3-Treiber von Pioneer DJ installiert?'
} else {
    Write-Host "Pult-Adapter: $($adp.Name) ($($adp.InterfaceDescription)), Status $($adp.Status)" -ForegroundColor Green
}

if (Get-Process rekordbox -ErrorAction SilentlyContinue) {
    Write-Host '' -ForegroundColor Yellow
    Write-Host 'ACHTUNG: rekordbox laeuft. Die DJ-Link-Ports sind dann belegt und das Pult zeigt rekordbox, nicht rekordbox-rs.' -ForegroundColor Yellow
    Write-Host 'rekordbox schliessen, dann dieses Skript erneut starten.' -ForegroundColor Yellow
    Read-Host 'Enter zum Abbrechen (oder weiter auf eigenes Risiko)' | Out-Null
}

# ---- music folder and drives --------------------------------------------------------------
if (-not $Music) { $Music = Join-Path $env:USERPROFILE 'Music' }
$musicLetter = ''
$q = Split-Path -Qualifier $Music
if ($q -match '^([A-Za-z]):$') { $musicLetter = $matches[1].ToUpper() }
$drives = if ($Drives) { ($Drives -replace '[^A-Za-z]', '').ToUpper() } else { 'C' + $musicLetter }
$drives = -join ($drives.ToCharArray() | Sort-Object -Unique)

# ---- arguments ----------------------------------------------------------------------------
$linkArgs = @('--seconds', $Seconds, '--db-delay', '10', '--drives', $drives, '--music', $Music)
if ($Adapter) { $linkArgs += @('--adapter', $Adapter) }
$logDir = Join-Path $env:APPDATA 'rekordbox-rs\logs'
New-Item -ItemType Directory -Force $logDir | Out-Null
$log = Join-Path $logDir 'link-server.txt'
$linkArgs += @('--log', $log)

Write-Host ''
Write-Host "Bibliothek wird geladen und fuer $Seconds s angeboten (Ordner: $Music, Laufwerke: $drives)." -ForegroundColor Cyan
Write-Host 'Am XDJ-RX3: SOURCE druecken und den PC als Quelle waehlen, dann browsen und laden.' -ForegroundColor Cyan
Write-Host 'Beenden: dieses Fenster schliessen.' -ForegroundColor DarkGray
Write-Host ''

& $exe @linkArgs
$code = $LASTEXITCODE
Write-Host ''
Write-Host "Protokoll: $log"
if ($code -ne 0) { Write-Host "link-announce beendet mit Code $code." -ForegroundColor Yellow }
Read-Host 'Enter zum Schliessen' | Out-Null
