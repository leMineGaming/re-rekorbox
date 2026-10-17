@echo off
rem Link diagnosis with the DJ unit: rekordbox must be closed. Choose LINK on the unit, browse, load a track.
"%~dp0rbx-selftest.exe" --link-diag 240 --open --pause %*
