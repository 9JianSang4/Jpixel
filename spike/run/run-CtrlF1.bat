@echo off
cd /d "%~dp0"
echo Starting spike with Ctrl+F1...
echo If this works but plain F1 does not, F1 is being intercepted by software.
spike.exe --hotkey Ctrl+F1
pause
