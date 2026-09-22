@echo off
setlocal EnableExtensions
cd /d "%~dp0"

call powershell -NoProfile -ExecutionPolicy Bypass -File "%~dp0scripts\start.ps1"
if not errorlevel 1 exit /b 0

echo.
echo [VLM_Screenshot_Action] Startup failed. Review the error above.
pause
exit /b 1
