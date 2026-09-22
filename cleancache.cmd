@echo off
setlocal EnableExtensions
cd /d "%~dp0"

echo ====================================
echo VLM_Screenshot_Action - Clean All Build Files
echo ====================================
echo.

call powershell -NoProfile -ExecutionPolicy Bypass -File "%~dp0scripts\cleanup-cache.ps1" -ForMigration

if not errorlevel 1 (
    echo.
    echo ====================================
    echo Clean completed successfully
    echo ====================================
    echo.
    echo Run clickstart.cmd to rebuild everything
    pause
    exit /b 0
)

echo.
echo ====================================
echo Clean completed with warnings
echo ====================================
pause
exit /b 1
