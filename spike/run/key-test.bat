@echo off
cd /d "%~dp0"
echo Press F1, F8, Ctrl+Alt+J, and Fn+F1 during the next 20 seconds.
powershell -NoProfile -ExecutionPolicy Bypass -File "%~dp0key-test.ps1" -Seconds 20
echo.
echo Copy the lines above and send them to the developer.
pause
