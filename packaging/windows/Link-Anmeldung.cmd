@echo off
rem Link Export: rekordbox-rs joins the DJ Link network like rekordbox and offers the library for
rem browsing. rekordbox must be closed. On the unit: choose the PC in the source selection and browse.
rem Runs 15 minutes (close the window to end earlier). Output also goes to the log file below.
rem Works in the installed package and in the source folder (falls back to target\release).
set "EXE=%~dp0link-announce.exe"
if not exist "%EXE%" set "EXE=%~dp0..\..\target\release\link-announce.exe"
if not exist "%EXE%" (
    echo link-announce.exe nicht gefunden. Im Quellordner zuerst bauen:
    echo   cargo build --release -p rbx-link
    pause
    exit /b 1
)
echo Bibliothek wird geladen (kann etwa eine Minute dauern) ...
"%EXE%" --seconds 900 --db-delay 10 --log "%APPDATA%\rekordbox-rs\logs\link-anmeldung.txt" %*
echo.
echo Protokoll: %APPDATA%\rekordbox-rs\logs\link-anmeldung.txt
pause
