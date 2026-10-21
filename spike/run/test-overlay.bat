@echo off
cd /d "%~dp0"
echo Showing the native overlay on monitor 1 without a hotkey...
echo Drag a rectangle, Enter = copy, Esc = cancel.
spike.exe --test-overlay --monitor 1
pause
