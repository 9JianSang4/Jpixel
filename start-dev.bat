@echo off
cd /d "%~dp0"
echo Starting Jpixel in development mode...
echo This starts Vite (localhost:1420) and the Tauri app together.
echo Keep this window open; close it to stop the dev server.
call npm run tauri dev
pause
