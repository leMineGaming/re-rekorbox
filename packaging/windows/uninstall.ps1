# Removes rekordbox-rs (program folder, shortcuts, uninstall entry). Settings, logs and own analyses in
# %APPDATA%\rekordbox-rs are kept unless -RemoveData is given.
#
#   powershell -ExecutionPolicy Bypass -File uninstall.ps1 [-RemoveData]
param([switch]$RemoveData)

$dir = $PSScriptRoot
if (Get-Process -Name 'rbx-ui' -ErrorAction SilentlyContinue) {
    Write-Host 'rekordbox-rs läuft noch - bitte zuerst beenden.' -ForegroundColor Yellow
    exit 1
}
Remove-Item -Recurse -Force (Join-Path ([Environment]::GetFolderPath('Programs')) 'rekordbox-rs') -ErrorAction SilentlyContinue
Remove-Item -Force (Join-Path ([Environment]::GetFolderPath('Desktop')) 'rekordbox-rs.lnk') -ErrorAction SilentlyContinue
Remove-Item -Force 'HKCU:\Software\Microsoft\Windows\CurrentVersion\Uninstall\rekordbox-rs' -ErrorAction SilentlyContinue
if ($RemoveData) {
    Remove-Item -Recurse -Force (Join-Path $env:APPDATA 'rekordbox-rs') -ErrorAction SilentlyContinue
}
Set-Location $env:TEMP
# only delete the folder if it really is a rekordbox-rs installation
if (Test-Path (Join-Path $dir 'rbx-ui.exe')) {
    Remove-Item -Recurse -Force $dir -ErrorAction SilentlyContinue
}
Write-Host 'rekordbox-rs wurde entfernt.'
if (-not $RemoveData) { Write-Host "Einstellungen und eigene Analysen bleiben in $env:APPDATA\rekordbox-rs." }
