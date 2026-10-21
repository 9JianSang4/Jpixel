@echo off
cd /d "%~dp0"
echo Starting spike with Alt+X as the global hotkey...
echo Alt+X = capture, Enter = copy, Esc = cancel.
spike.exe --hotkey Alt+X
pause
