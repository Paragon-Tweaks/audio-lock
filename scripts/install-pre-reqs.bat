@echo off
setlocal EnableExtensions

title Audio Lock - Install Build & Release Prerequisites
cd /d "%~dp0.."

echo ============================================================
echo   Audio Lock - Install Build ^& Release Prerequisites
echo ============================================================
echo.

where winget >nul 2>&1
if errorlevel 1 (
    echo [ERROR] winget was not found on this machine.
    echo         Install "App Installer" from the Microsoft Store and re-run this script.
    echo.
    pause
    exit /b 1
)

set "FAILED=0"
set "INSTALLED_ANY=0"

:: 1. Rust toolchain (cargo / rustc)
where cargo >nul 2>&1
if errorlevel 1 (
    echo [MISSING] Rust toolchain ^(cargo^) - installing via winget...
    winget install --id Rustlang.Rustup -e --accept-package-agreements --accept-source-agreements
    if errorlevel 1 (
        echo [ERROR] Failed to install Rustlang.Rustup.
        set "FAILED=1"
    ) else (
        set "INSTALLED_ANY=1"
    )
) else (
    echo [OK] Rust toolchain ^(cargo^) is already installed.
)

:: 2. Inno Setup 6 (ISCC.exe)
set "ISCC_FOUND="
where ISCC.exe >nul 2>&1 && set "ISCC_FOUND=1"
if exist "C:\Program Files (x86)\Inno Setup 6\ISCC.exe" set "ISCC_FOUND=1"
if exist "C:\Program Files\Inno Setup 6\ISCC.exe" set "ISCC_FOUND=1"
if exist "%LOCALAPPDATA%\Programs\Inno Setup 6\ISCC.exe" set "ISCC_FOUND=1"

if not defined ISCC_FOUND (
    echo [MISSING] Inno Setup 6 ^(ISCC.exe^) - installing via winget...
    winget install --id JRSoftware.InnoSetup -e --accept-package-agreements --accept-source-agreements
    if errorlevel 1 (
        echo [ERROR] Failed to install JRSoftware.InnoSetup.
        set "FAILED=1"
    ) else (
        set "INSTALLED_ANY=1"
    )
) else (
    echo [OK] Inno Setup 6 ^(ISCC.exe^) is already installed.
)

:: 3. GitHub CLI (gh)
where gh >nul 2>&1
if errorlevel 1 (
    echo [MISSING] GitHub CLI ^(gh^) - installing via winget...
    winget install --id GitHub.cli -e --accept-package-agreements --accept-source-agreements
    if errorlevel 1 (
        echo [ERROR] Failed to install GitHub.cli.
        set "FAILED=1"
    ) else (
        set "INSTALLED_ANY=1"
    )
) else (
    echo [OK] GitHub CLI ^(gh^) is already installed.
)

:: 4. Windows SDK (signtool.exe)
set "SIGNTOOL_FOUND="
where signtool.exe >nul 2>&1 && set "SIGNTOOL_FOUND=1"
if not defined SIGNTOOL_FOUND (
    for /f "delims=" %%F in ('dir /b /s "C:\Program Files (x86)\Windows Kits\10\bin\signtool.exe" 2^>nul') do (
        echo %%F | findstr /i /l "\x64\signtool.exe" >nul && set "SIGNTOOL_FOUND=1"
    )
)

if not defined SIGNTOOL_FOUND (
    echo [MISSING] Windows SDK ^(signtool.exe^) - installing via winget...
    winget install --id Microsoft.WindowsSDK.10.0.26100 -e --accept-package-agreements --accept-source-agreements
    if errorlevel 1 (
        echo [ERROR] Failed to install Windows SDK ^(signtool.exe^).
        set "FAILED=1"
    ) else (
        set "INSTALLED_ANY=1"
    )
) else (
    echo [OK] Windows SDK ^(signtool.exe^) is already installed.
)

:: 5. Check GitHub CLI login status if gh is available
where gh >nul 2>&1
if not errorlevel 1 (
    gh auth status >nul 2>&1
    if errorlevel 1 (
        echo.
        echo [NOTE] GitHub CLI is installed but not signed in yet.
        echo        Run "gh auth login" before using build-and-release.bat.
    ) else (
        echo [OK] GitHub CLI is authenticated.
    )
)

echo.
if "%FAILED%"=="1" (
    echo FAILED: One or more prerequisites failed to install. Check output above.
) else (
    echo DONE: All build and release prerequisites are ready.
    if "%INSTALLED_ANY%"=="1" (
        echo       Open a fresh terminal or re-run your build .bat so updated PATH entries take effect.
    )
)
echo.
pause
exit /b %FAILED%
