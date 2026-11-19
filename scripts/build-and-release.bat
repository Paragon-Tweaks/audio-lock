@echo off
setlocal
title Audio Lock - Build, Sign ^& Publish Release
cd /d "%~dp0.."

echo ============================================================
echo   Audio Lock - Build, Sign ^& Publish Release
echo ============================================================
echo   Reminder: Plug in your Sectigo USB code-signing token before continuing.
echo.

choice /C YN /M "Build, sign, tag, and PUBLISH this release to GitHub now"
if errorlevel 2 (
    echo.
    echo Cancelled.
    echo.
    pause
    exit /b 0
)

echo.
powershell.exe -NoProfile -ExecutionPolicy Bypass -File "%~dp0build-release.ps1" %*
set "EXIT_CODE=%ERRORLEVEL%"

echo.
if "%EXIT_CODE%"=="0" (
    echo DONE - Signed release published successfully.
) else (
    echo FAILED ^(exit code %EXIT_CODE%^)
)
echo.
pause
exit /b %EXIT_CODE%
