@echo off
rem Link Export phase 1: rekordbox-rs joins the DJ Link network like rekordbox. rekordbox must be closed.
rem On the unit: choose LINK (source selection). Output also goes to the log file below.
rem Works in the installed package and in the source folder (falls back to target\release).
set "EXE=%~dp0link-announce.exe"
if not exist "%EXE%" set "EXE=%~dp0..\..\target\release\link-announce.exe"
if not exist "%EXE%" (
    echo link-announce.exe nicht gefunden. Im Quellordner zuerst bauen:
    echo   cargo build --release -p rbx-link
    pause
    exit /b 1
)
"%EXE%" --seconds 120 --log "%APPDATA%\rekordbox-rs\logs\link-anmeldung.txt" %*
echo.
echo Protokoll: %APPDATA%\rekordbox-rs\logs\link-anmeldung.txt
pause
