@echo off
rem Guided capture rekordbox <-> XDJ-RX3 (LINK EXPORT) for rekordbox-rs; see docs/link/PHASE0.md
powershell.exe -NoProfile -ExecutionPolicy Bypass -File "%~dp0link-capture.ps1"
