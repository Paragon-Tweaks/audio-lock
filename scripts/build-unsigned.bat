@echo off
setlocal
title Audio Lock - Build Unsigned (Local Test)
cd /d "%~dp0.."

echo ============================================================
echo   Audio Lock - Build Unsigned (Local Test)
echo ============================================================
echo.

powershell.exe -NoProfile -ExecutionPolicy Bypass -File "%~dp0build-unsigned.ps1" %*
set "EXIT_CODE=%ERRORLEVEL%"

echo.
if "%EXIT_CODE%"=="0" (
    echo DONE - Unsigned build succeeded.
) else (
    echo FAILED ^(exit code %EXIT_CODE%^)
)
echo.
pause
exit /b %EXIT_CODE%
