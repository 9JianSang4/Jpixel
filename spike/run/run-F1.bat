@echo off
cd /d "%~dp0"
echo Starting spike with F1 as the only global hotkey...
echo IMPORTANT: exit Jpixel from the system tray first, otherwise F1 is already taken.
spike.exe --hotkey F1
pause
