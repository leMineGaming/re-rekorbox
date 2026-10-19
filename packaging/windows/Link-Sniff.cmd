@echo off
rem Guided sniffing (USB + network) for further Link Export / Controller implementation.
rem Captures USBPcap (RTL8152 framing, SysEx, HID) and the network side (DJ Link/NFS/dbserver)
rem for the scenarios of xdj-rx3-link-export-protokoll.md section 9. Pick a scenario in the menu.
powershell.exe -NoProfile -ExecutionPolicy Bypass -File "%~dp0link-sniff.ps1"
