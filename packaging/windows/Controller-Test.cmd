@echo off
rem Controller test for the XDJ-RX3: LED light, then 90 s of input (move every control), report in the browser.
rem rekordbox and rbx-ui must be closed so that the MIDI ports are free.
"%~dp0rbx-selftest.exe" --quick --no-network --leds --midi-listen 90 --open --pause %*
