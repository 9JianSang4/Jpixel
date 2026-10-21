@echo off
cd /d "%~dp0"
echo Starting spike with Ctrl+Alt+J as the global hotkey...
echo If an error box appears, read it and tell the developer.
spike.exe --hotkey Ctrl+Alt+J
pause
