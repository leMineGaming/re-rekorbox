@echo off
rem Full self test; the report opens in the browser. Nothing is audible.
"%~dp0rbx-selftest.exe" --open --pause %*
