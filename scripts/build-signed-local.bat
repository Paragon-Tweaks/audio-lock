@echo off
setlocal
title Audio Lock - Build ^& Sign Locally (No Publish)
cd /d "%~dp0.."

echo ============================================================
echo   Audio Lock - Build ^& Sign Locally (No Publish)
echo ============================================================
echo   Make sure your Sectigo USB code-signing token is plugged in.
echo.

powershell.exe -NoProfile -ExecutionPolicy Bypass -File "%~dp0build-release.ps1" -NoPublish %*
set "EXIT_CODE=%ERRORLEVEL%"

echo.
if "%EXIT_CODE%"=="0" (
    echo DONE - Signed local build succeeded.
) else (
    echo FAILED ^(exit code %EXIT_CODE%^)
)
echo.
pause
exit /b %EXIT_CODE%
