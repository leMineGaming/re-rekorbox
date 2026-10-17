# Installs rekordbox-rs for the current user (no administrator rights needed):
# program folder %LOCALAPPDATA%\Programs\rekordbox-rs, start menu and desktop shortcuts, entry under
# "Apps & Features" for uninstalling. Settings, logs and own analyses stay in %APPDATA%\rekordbox-rs.
#
#   powershell -ExecutionPolicy Bypass -File install.ps1 [-Target DIR] [-NoShortcuts] [-NoDesktop]
param(
    [string]$Target = (Join-Path $env:LOCALAPPDATA 'Programs\rekordbox-rs'),
    [switch]$NoShortcuts,
    [switch]$NoDesktop
)

$ErrorActionPreference = 'Stop'
$src = $PSScriptRoot
if (Get-Process -Name 'rbx-ui' -ErrorAction SilentlyContinue) {
    Write-Host 'rekordbox-rs läuft noch - bitte zuerst beenden.' -ForegroundColor Yellow
    exit 1
}
New-Item -ItemType Directory -Force $Target | Out-Null
$files = Get-ChildItem $src -File | Where-Object { $_.Extension -in '.exe', '.ps1', '.cmd', '.txt', '.ico' -and $_.Name -ne 'Installieren.cmd' }
foreach ($f in $files) { Copy-Item $f.FullName $Target -Force }
$version = (Get-Item (Join-Path $Target 'rbx-ui.exe')).VersionInfo.FileVersion
Write-Host "rekordbox-rs $version installiert in $Target"

if (-not $NoShortcuts) {
    $shell = New-Object -ComObject WScript.Shell
    function New-Link($path, $exe, $arguments, $desc) {
        $l = $shell.CreateShortcut($path)
        $l.TargetPath = $exe
        $l.Arguments = $arguments
        $l.WorkingDirectory = $Target
        $l.IconLocation = (Join-Path $Target 'icon.ico')
        $l.Description = $desc
        $l.Save()
    }
    $menu = Join-Path ([Environment]::GetFolderPath('Programs')) 'rekordbox-rs'
    New-Item -ItemType Directory -Force $menu | Out-Null
    New-Link (Join-Path $menu 'rekordbox-rs.lnk') (Join-Path $Target 'rbx-ui.exe') '' 'DJ-Player'
    New-Link (Join-Path $menu 'Selbsttest.lnk') (Join-Path $Target 'Selbsttest.cmd') '' 'Automatischer Selbsttest mit Bericht'
    New-Link (Join-Path $menu 'Deinstallieren.lnk') 'powershell.exe' "-ExecutionPolicy Bypass -File `"$(Join-Path $Target 'uninstall.ps1')`"" 'rekordbox-rs entfernen'
    if (-not $NoDesktop) {
        New-Link (Join-Path ([Environment]::GetFolderPath('Desktop')) 'rekordbox-rs.lnk') (Join-Path $Target 'rbx-ui.exe') '' 'DJ-Player'
    }
    $key = 'HKCU:\Software\Microsoft\Windows\CurrentVersion\Uninstall\rekordbox-rs'
    New-Item -Force $key | Out-Null
    Set-ItemProperty $key DisplayName 'rekordbox-rs'
    Set-ItemProperty $key DisplayVersion $version
    Set-ItemProperty $key Publisher 'rekordbox-rs'
    Set-ItemProperty $key InstallLocation $Target
    Set-ItemProperty $key DisplayIcon (Join-Path $Target 'rbx-ui.exe')
    Set-ItemProperty $key UninstallString "powershell.exe -ExecutionPolicy Bypass -File `"$(Join-Path $Target 'uninstall.ps1')`""
    Set-ItemProperty $key NoModify 1 -Type DWord
    Set-ItemProperty $key NoRepair 1 -Type DWord
    Write-Host 'Verknüpfungen im Startmenü (rekordbox-rs) und auf dem Desktop angelegt.'
}
