@echo off
rem Link Export: exposes the rekordbox-rs library to the XDJ-RX3 (dbserver + NFS) so the unit shows
rem the PC as a source and can browse and load tracks. rekordbox must be closed.
rem On the unit: SOURCE -> choose the PC -> browse -> LOAD.
powershell.exe -NoProfile -ExecutionPolicy Bypass -File "%~dp0link-server.ps1" %*
