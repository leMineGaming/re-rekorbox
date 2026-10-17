@echo off
rem Records rekordbox <-> DJ player traffic (LINK EXPORT) for rekordbox-rs; asks for administrator rights.
powershell.exe -NoProfile -ExecutionPolicy Bypass -File "%~dp0link-capture.ps1"
