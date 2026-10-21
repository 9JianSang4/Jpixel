@echo off
cd /d "%~dp0"
echo Starting spike with F8 as the global hotkey...
echo If an error box appears, read it and tell the developer.
spike.exe --hotkey F8
pause
