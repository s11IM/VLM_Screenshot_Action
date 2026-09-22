@echo off
setlocal EnableExtensions
cd /d "%~dp0"

call powershell -NoProfile -ExecutionPolicy Bypass -File "%~dp0launch.ps1" -AppPath "%~dp0VLM_Screenshot_Action.exe"
if not errorlevel 1 exit /b 0

echo.
echo [VLM_Screenshot_Action] Startup failed. Review the error above.
pause
exit /b 1
