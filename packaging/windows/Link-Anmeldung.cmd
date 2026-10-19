@echo off
rem Link Export phase 1: rekordbox-rs joins the DJ Link network like rekordbox. rekordbox must be closed.
rem On the unit: choose LINK (source selection). Output also goes to the log file below.
"%~dp0link-announce.exe" --seconds 120 --log "%APPDATA%\rekordbox-rs\logs\link-anmeldung.txt" %*
echo.
echo Protokoll: %APPDATA%\rekordbox-rs\logs\link-anmeldung.txt
pause
