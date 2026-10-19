# Guided sniffing for further Link Export / Controller implementation (xdj-rx3-link-export-protokoll.md
# section 9 + 11): captures BOTH sides in parallel --
#   * USB (USBPcap)      -> RTL8152 Ethernet-over-USB framing, the MIDI SysEx, HID reports (section 4, 11.5)
#   * network (dumpcap or pktmon) -> Pro DJ Link / NFS / dbserver (sections 6-8)
#
#   powershell -ExecutionPolicy Bypass -File link-sniff.ps1 [-Szenario 1..5] [-UsbDevice \\.\USBPcap2]
#
# Output per run: %APPDATA%\rekordbox-rs\capture\<time>\capture.pcapng (network), capture-usb.pcapng (USB),
# markers.txt (timestamps), linkdump.txt + linksniff.txt (decoded network traffic).
#
# USBPcap: bundled with Wireshark (component "USBPcap") or from https://desowin.org/usbpcap/ (standalone).
# If the USB capture stays empty, the unit is on another root hub -- try -UsbDevice \\.\USBPcap2 etc.
param(
    [int]$Szenario = 0,
    [string]$Out = '',
    [string]$UsbDevice = '\\.\USBPcap1'
)

$ErrorActionPreference = 'Stop'

# ---- scenarios (protocol section 9) ------------------------------------------------------
$Szenarien = [ordered]@{
    1 = [ordered]@{ Titel = '1: Kabel einstecken / Handshake (Claim 0x00/0x02/0x04, USB-Enumeration)'; Schritte = @(
        'Vorbereitung: Pult aus (oder USB-Kabel ab). Aufnahme laeuft, dann Kabel einstecken bzw. Pult einschalten und NICHTS tun.',
        '30 s warten, bis sich der Link beruhigt hat (Keep-alives + Status laufen).'
        'Danach optional rekordbox im EXPORT-Modus starten und warten, bis die Bibliothek erscheint.' ) }
    2 = [ordered]@{ Titel = '2: Track laden, Play, Cue, Pitch, Loop (Struktur der 0x0A-Statuspakete)'; Schritte = @(
        'rekordbox im EXPORT-Modus starten; am Pult den PC als Quelle waehlen.',
        'Track A auf Deck 1 laden, Track B auf Deck 2 laden.',
        'Deck 1: PLAY, dann Pitch-Slider bewegen (mehrere Positionen), Cue setzen und anspringen.',
        'Loop setzen (z. B. 4 Takte), kurz laufen lassen, Loop verlassen, Pause.' ) }
    3 = [ordered]@{ Titel = '3: Playlists, Suche, grosse Bibliothek (Menuestruktur, Paging 0x3000)'; Schritte = @(
        'rekordbox im EXPORT-Modus; am Pult den PC als Quelle waehlen.',
        'Durch mehrere Playlists blaettern (auch verschachtelte Ordner).',
        'Eine Suche ausfuehren und in den Treffern blaettern.',
        'In einer grossen Track-Liste weit nach unten scrollen (mehrere Seiten laden).' ) }
    4 = [ordered]@{ Titel = '4: Beatgrid, Cues, farbige Wellenform (Formate 0x2004/0x2C04)'; Schritte = @(
        'rekordbox im EXPORT-Modus; am Pult den PC als Quelle waehlen.',
        'Tracks mit Beatgrid, mehreren Hot Cues und farbiger Wellenform laden.',
        'Beim Laden die Wellenform/Beatgrid-Anzeige am Pult beobachten.',
        'Mehrere solcher Tracks laden (unterschiedliche Tempi/Farben).' ) }
    5 = [ordered]@{ Titel = '5: Controller-Modus (HID/MIDI, Jog/Pads/Waveform, SysEx)'; Schritte = @(
        'Am Pult den Controller-/PC-Modus waehlen (nicht Link Export).',
        'rekordbox im Controller-/Performance-Modus starten und das Pult verbinden.',
        'Jogs drehen/beruehren, Pads druecken, Fader/Regler bewegen, Hot Cues setzen.',
        'Wellenform auf dem Pult-Display anzeigen lassen (sofern der Modus das vorsieht).' ) }
}

# ---- tool detection -----------------------------------------------------------------------
$dumpcap = @("$env:ProgramFiles\Wireshark\dumpcap.exe", "${env:ProgramFiles(x86)}\Wireshark\dumpcap.exe") | Where-Object { Test-Path $_ } | Select-Object -First 1
$usbpcap = @("$env:ProgramFiles\USBPcap\USBPcapCMD.exe", "${env:ProgramFiles(x86)}\USBPcap\USBPcapCMD.exe") | Where-Object { Test-Path $_ } | Select-Object -First 1
$admin = ([Security.Principal.WindowsPrincipal][Security.Principal.WindowsIdentity]::GetCurrent()).IsInRole([Security.Principal.WindowsBuiltInRole]::Administrator)
if (-not $dumpcap -and -not $admin) {
    Start-Process powershell.exe -Verb RunAs -ArgumentList "-NoProfile -ExecutionPolicy Bypass -File `"$PSCommandPath`" -Szenario $Szenario -UsbDevice `"$UsbDevice`""
    exit
}

# ---- scenario choice ----------------------------------------------------------------------
if ($Szenario -notin 1..5) {
    Write-Host ''
    Write-Host 'Welches Szenario soll aufgenommen werden?' -ForegroundColor Cyan
    foreach ($k in 1..5) { Write-Host "  $($Szenarien[$k].Titel)" }
    $in = Read-Host 'Nummer (1-5)'
    if ($in -notmatch '^[1-5]$') { Write-Host 'Abgebrochen.'; exit 0 }
    $Szenario = [int]$in
}
$s = $Szenarien[$Szenario]

# ---- output dir ---------------------------------------------------------------------------
if (-not $Out) { $Out = Join-Path $env:APPDATA ("rekordbox-rs\capture\" + (Get-Date -Format 'yyyy-MM-dd-HH-mm-ss')) }
New-Item -ItemType Directory -Force $Out | Out-Null
$pcap = Join-Path $Out 'capture.pcapng'
$usbp = Join-Path $Out 'capture-usb.pcapng'
$markers = Join-Path $Out 'markers.txt'

function Mark([string]$step) {
    $line = '{0}  {1}' -f (Get-Date).ToUniversalTime().ToString('yyyy-MM-dd HH:mm:ss.fff'), $step
    Add-Content -Encoding utf8 $markers $line
    Write-Host "[markiert] $step" -ForegroundColor DarkGray
}

function Step([string]$text) {
    Write-Host ''
    Write-Host $text -ForegroundColor Cyan
    Read-Host 'Enter, wenn erledigt' | Out-Null
}

Write-Host ''
Write-Host "Szenario: $($s.Titel)" -ForegroundColor Green
if (-not $usbpcap) { Write-Host 'USBPcap nicht gefunden: nur Netzwerk wird aufgenommen. Fuer USB-Level (4.2/11.5/HID) USBPcap installieren.' -ForegroundColor Yellow }

$adapter = Get-NetAdapter -ErrorAction SilentlyContinue | Where-Object { $_.InterfaceDescription -match 'Pioneer|AlphaTheta' } | Select-Object -First 1
if ($adapter) { Write-Host "Pult-Adapter: $($adapter.Name) ($($adapter.InterfaceDescription))" -ForegroundColor Green } else { Write-Host 'Kein Pioneer-Netzwerkadapter gefunden.' -ForegroundColor Yellow }

Write-Host ''
Write-Host 'Vorbereitung: rekordbox und rekordbox-rs geschlossen, XDJ-RX3 verbunden.' -ForegroundColor Cyan
Read-Host 'Enter = Aufnahme starten' | Out-Null

# ---- start captures -----------------------------------------------------------------------
$netProc = $null
$usbProc = $null
if ($usbpcap) {
    $usbProc = Start-Process -FilePath $usbpcap -ArgumentList @('-d', "`"$UsbDevice`"", '-o', "`"$usbp`"") -PassThru -WindowStyle Hidden
    Start-Sleep 1
}
if ($dumpcap) {
    $iface = if ($adapter) { $adapter.Name } else { '1' }
    $netProc = Start-Process -FilePath $dumpcap -ArgumentList @('-i', "`"$iface`"", '-w', "`"$pcap`"", '-q') -PassThru -WindowStyle Hidden
    Start-Sleep 2
    if ($netProc.HasExited) { throw "dumpcap konnte nicht auf '$iface' aufnehmen (Npcap installiert?)" }
} else {
    pktmon filter remove | Out-Null
    pktmon start --capture --pkt-size 0 --file-name (Join-Path $Out 'capture.etl') | Out-Null
}
Write-Host 'Aufnahme laeuft.' -ForegroundColor Green
Mark "Aufnahme gestartet (Szenario $Szenario)"

# ---- guided steps -------------------------------------------------------------------------
foreach ($schritt in $s.Schritte) { Step $schritt }
Mark "Szenario $Szenario abgeschlossen"

# ---- stop captures ------------------------------------------------------------------------
if ($usbProc) { Stop-Process -Id $usbProc.Id -ErrorAction SilentlyContinue; Start-Sleep 1 }
if ($netProc) {
    Stop-Process -Id $netProc.Id -ErrorAction SilentlyContinue
    Start-Sleep 1
} else {
    pktmon stop | Out-Null
    pktmon etl2pcap (Join-Path $Out 'capture.etl') --out $pcap | Out-Null
    Remove-Item (Join-Path $Out 'capture.etl') -ErrorAction SilentlyContinue
}
Mark 'Aufnahme beendet'

# ---- decode the network side --------------------------------------------------------------
$rbf = Join-Path $PSScriptRoot 'rbf.exe'
$sniff = Join-Path $PSScriptRoot 'link-sniff.exe'
if ((Test-Path $rbf) -and (Test-Path $pcap)) {
    & $rbf linkdump $pcap | Out-File -Encoding utf8 (Join-Path $Out 'linkdump.txt')
}
if ((Test-Path $sniff) -and (Test-Path $pcap)) {
    & $sniff --pcap $pcap --db | Out-File -Encoding utf8 (Join-Path $Out 'linksniff.txt')
}

Write-Host ''
Write-Host "Fertig: $Out" -ForegroundColor Green
Write-Host '  capture.pcapng      Netzwerk (Pro DJ Link / NFS / dbserver)'
if ($usbProc) { Write-Host '  capture-usb.pcapng  USB (RTL8152-Frames, SysEx, HID-Reports) - mit Wireshark/USBPcap auswerten' }
Write-Host '  markers.txt         Zeitpunkte der Schritte'
if (Test-Path (Join-Path $Out 'linkdump.txt')) { Write-Host '  linkdump.txt        dekodierter Netzwerkverkehr (rbf linkdump)' }
if (Test-Path (Join-Path $Out 'linksniff.txt')) { Write-Host '  linksniff.txt       DJ-Link + dbserver (link-sniff)' }
Write-Host ''
Write-Host 'Bitte schicken: capture.pcapng, capture-usb.pcapng, markers.txt (und linkdump.txt/linksniff.txt).' -ForegroundColor Green
Start-Process explorer.exe $Out
Read-Host 'Enter zum Schliessen' | Out-Null
