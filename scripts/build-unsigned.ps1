[CmdletBinding()]
param(
    [string]$Version,
    [string]$GithubRepo = "Paragon-Tweaks/audio-lock",
    [string]$ManifestUrl,
    [switch]$SkipChecks,
    [switch]$Package
)

$ErrorActionPreference = "Stop"
Set-StrictMode -Version Latest

$RepoRoot = Split-Path -Parent $PSScriptRoot
$SemverRegex = '^\d+\.\d+\.\d+(-[0-9A-Za-z.-]+)?$'

function Get-CargoVersion {
    param([string]$CargoTomlPath)
    $content = Get-Content -Path $CargoTomlPath -Raw
    if ($content -match '(?ms)^\[workspace\.package\]\s*\r?\n(.*?)(?=^\[|\z)' -and $Matches[1] -match '(?m)^\s*version\s*=\s*"([^"]+)"') {
        return $Matches[1].Trim()
    }
    throw "Could not determine [workspace.package].version from $CargoTomlPath"
}

function Resolve-GithubRepoSlug {
    param([string]$RepoParam)
    $repoSlug = $RepoParam.Trim().Trim('/')
    if ($repoSlug -notmatch '^[A-Za-z0-9_.-]+/[A-Za-z0-9_.-]+$' -or $repoSlug.StartsWith('OWNER/', [System.StringComparison]::OrdinalIgnoreCase)) {
        throw "Invalid -GithubRepo '$RepoParam'. Expected '<owner>/<repo>' (for example, 'Paragon-Tweaks/audio-lock')."
    }
    return $repoSlug
}

function Resolve-ManifestUri {
    param([string]$ManifestUrlParam)
    $trimmedManifest = $ManifestUrlParam.Trim()
    $parsedUri = $null
    if (-not [System.Uri]::TryCreate($trimmedManifest, [System.UriKind]::Absolute, [ref]$parsedUri)) {
        throw "Invalid -ManifestUrl '$ManifestUrlParam'. Expected a valid absolute https:// or loopback http:// URL."
    }

    $isHttps = $parsedUri.Scheme -eq 'https'
    $isLoopbackHttp = ($parsedUri.Scheme -eq 'http') -and $parsedUri.IsLoopback
    if (-not ($isHttps -or $isLoopbackHttp) -or [string]::IsNullOrWhiteSpace($parsedUri.Host)) {
        throw "Invalid -ManifestUrl '$ManifestUrlParam'. Only https:// or loopback http:// URLs are allowed."
    }

    return $parsedUri
}

function Resolve-AssetUrl {
    param(
        [string]$ReleaseVersion,
        [string]$RepoSlug,
        [string]$ManifestUrlParam
    )

    if (-not [string]::IsNullOrWhiteSpace($ManifestUrlParam)) {
        $parsedUri = Resolve-ManifestUri -ManifestUrlParam $ManifestUrlParam
        return ([System.Uri]::new($parsedUri, "default-audio-lock-win64-update.zip")).AbsoluteUri
    }

    return "https://github.com/$RepoSlug/releases/download/v$ReleaseVersion/default-audio-lock-win64-update.zip"
}

function Find-Iscc {
    $cmd = Get-Command "iscc.exe" -ErrorAction SilentlyContinue
    if ($cmd) {
        return $cmd.Source
    }

    $candidates = @(
        "${env:ProgramFiles(x86)}\Inno Setup 6\ISCC.exe",
        "$env:ProgramFiles\Inno Setup 6\ISCC.exe",
        "$env:LOCALAPPDATA\Programs\Inno Setup 6\ISCC.exe"
    )
    foreach ($candidate in $candidates) {
        if ($candidate -and (Test-Path -LiteralPath $candidate)) {
            return $candidate
        }
    }
    return $null
}

function Test-FileLocked {
    param([string]$Path)
    if (-not (Test-Path -LiteralPath $Path)) {
        return $false
    }
    $stream = $null
    try {
        $stream = [System.IO.File]::Open(
            $Path,
            [System.IO.FileMode]::Open,
            [System.IO.FileAccess]::ReadWrite,
            [System.IO.FileShare]::None
        )
        return $false
    } catch [System.IO.IOException] {
        return $true
    } catch [System.UnauthorizedAccessException] {
        return $true
    } finally {
        if ($null -ne $stream) {
            $stream.Dispose()
        }
    }
}

function Assert-BinaryUnlocked {
    param([string]$BinaryPath)
    if (Test-FileLocked -Path $BinaryPath) {
        throw "Cannot overwrite '$BinaryPath' because it is currently locked by a running process. Quit the running Audio Lock tray app first (right-click the tray icon -> Quit, or run 'Stop-Process -Name default-audio-lock') and retry."
    }
}

function Restore-EnvVar {
    param(
        [string]$Name,
        [AllowNull()][string]$PreviousValue
    )
    if ($null -eq $PreviousValue) {
        Remove-Item "Env:$Name" -ErrorAction SilentlyContinue
    } else {
        Set-Item "Env:$Name" -Value $PreviousValue
    }
}

Push-Location $RepoRoot
$prevGithubRepo = $env:AUDIO_LOCK_GITHUB_REPO
$prevManifestUrl = $env:AUDIO_LOCK_UPDATE_MANIFEST_URL
$prevRequireAuthenticode = $env:AUDIO_LOCK_REQUIRE_AUTHENTICODE
$prevExpectedPublisher = $env:AUDIO_LOCK_EXPECTED_PUBLISHER

try {
    $CargoVersion = Get-CargoVersion -CargoTomlPath (Join-Path $RepoRoot "Cargo.toml")
    if ($CargoVersion -notmatch $SemverRegex) {
        throw "Cargo.toml [workspace.package].version '$CargoVersion' does not match strict semver pattern '$SemverRegex'."
    }

    if ([string]::IsNullOrWhiteSpace($Version)) {
        $Version = $CargoVersion
    } else {
        $Version = $Version.Trim() -replace '^[vV]', ''
        if ($Version -notmatch $SemverRegex) {
            throw "-Version '$Version' does not match strict semver pattern '$SemverRegex'."
        }
        if ($Version -ne $CargoVersion) {
            throw "Version mismatch: -Version '$Version' does not match Cargo.toml [workspace.package].version '$CargoVersion'."
        }
    }

    $repoSlug = Resolve-GithubRepoSlug -RepoParam $GithubRepo
    $hasManifestUrl = -not [string]::IsNullOrWhiteSpace($ManifestUrl)
    if ($hasManifestUrl) {
        [void](Resolve-ManifestUri -ManifestUrlParam $ManifestUrl)
    }

    # Always keep local unsigned test builds free of Authenticode signature requirements.
    Remove-Item Env:AUDIO_LOCK_REQUIRE_AUTHENTICODE -ErrorAction SilentlyContinue
    Remove-Item Env:AUDIO_LOCK_EXPECTED_PUBLISHER -ErrorAction SilentlyContinue
    $env:AUDIO_LOCK_GITHUB_REPO = $repoSlug
    if ($hasManifestUrl) {
        $env:AUDIO_LOCK_UPDATE_MANIFEST_URL = $ManifestUrl.Trim()
    } else {
        Remove-Item Env:AUDIO_LOCK_UPDATE_MANIFEST_URL -ErrorAction SilentlyContinue
    }

    if (-not $SkipChecks) {
        Write-Host "Running workspace clippy..."
        & cargo clippy --workspace --all-targets -- -D warnings
        if ($LASTEXITCODE -ne 0) {
            throw "cargo clippy failed with exit code $LASTEXITCODE"
        }

        Write-Host "Running workspace tests..."
        & cargo test --workspace
        if ($LASTEXITCODE -ne 0) {
            throw "cargo test failed with exit code $LASTEXITCODE"
        }
    } else {
        Write-Host "Skipping clippy and unit tests (-SkipChecks)." -ForegroundColor Yellow
    }

    $ReleaseDir = Join-Path $RepoRoot "target\release"
    $TrayExe = Join-Path $ReleaseDir "default-audio-lock.exe"
    Assert-BinaryUnlocked -BinaryPath $TrayExe

    Write-Host "Building unsigned release binary for Audio Lock v$Version..."
    & cargo build --release --workspace
    if ($LASTEXITCODE -ne 0) {
        if (Test-FileLocked -Path $TrayExe) {
            throw "cargo build failed because '$TrayExe' is locked by a running instance. Quit the running Audio Lock tray app first and retry."
        }
        throw "cargo build failed with exit code $LASTEXITCODE"
    }

    if (-not (Test-Path -LiteralPath $TrayExe)) {
        throw "Expected release binary not found: $TrayExe"
    }

    $exeInfo = Get-Item -LiteralPath $TrayExe
    $sizeBytes = $exeInfo.Length
    $sizeMb = [Math]::Round($sizeBytes / 1MB, 2)
    Write-Host ("Built unsigned binary: {0} ({1:N2} MB / {2:N0} bytes)" -f $exeInfo.FullName, $sizeMb, $sizeBytes) -ForegroundColor Green

    if ($Package) {
        $UnsignedDistDir = Join-Path $RepoRoot "dist\unsigned"
        New-Item -ItemType Directory -Path $UnsignedDistDir -Force | Out-Null

        $AssetUrl = Resolve-AssetUrl `
            -ReleaseVersion $Version `
            -RepoSlug $repoSlug `
            -ManifestUrlParam $ManifestUrl

        $UpdateZip = Join-Path $UnsignedDistDir "default-audio-lock-win64-update.zip"
        Compress-Archive -Path $TrayExe -DestinationPath $UpdateZip -Force

        $Sha256 = (Get-FileHash -Path $UpdateZip -Algorithm SHA256).Hash.ToLowerInvariant()
        $PublishedAt = (Get-Date).ToUniversalTime().ToString("yyyy-MM-ddTHH:mm:ssZ")

        $ManifestObj = [ordered]@{
            version      = $Version
            published_at = $PublishedAt
            sha256       = $Sha256
            asset_url    = $AssetUrl
        }
        $ManifestJson = $ManifestObj | ConvertTo-Json -Depth 2
        $ManifestPath = Join-Path $UnsignedDistDir "manifest.json"
        [System.IO.File]::WriteAllText($ManifestPath, $ManifestJson + "`n", [System.Text.UTF8Encoding]::new($false))

        Write-Host "Created unsigned update archive: $UpdateZip"
        Write-Host "Created unsigned manifest: $ManifestPath (sha256: $Sha256)"

        $IsccPath = Find-Iscc
        if ($IsccPath) {
            $IssPath = Join-Path $RepoRoot "installer\default-audio-lock.iss"
            & $IsccPath "/O$UnsignedDistDir" "/DMyAppVersion=$Version" "/DAppExePath=$TrayExe" $IssPath
            if ($LASTEXITCODE -ne 0) {
                throw "ISCC.exe failed with exit code $LASTEXITCODE"
            }
            $InstallerPath = Join-Path $UnsignedDistDir "DefaultAudioLock-Setup-$Version.exe"
            Write-Host "Created unsigned installer: $InstallerPath" -ForegroundColor Green
        } else {
            Write-Host "Note: Inno Setup 6 (ISCC.exe) not found; skipped unsigned installer build. Install with: winget install JRSoftware.InnoSetup" -ForegroundColor Yellow
        }
    }
}
finally {
    Restore-EnvVar -Name "AUDIO_LOCK_GITHUB_REPO" -PreviousValue $prevGithubRepo
    Restore-EnvVar -Name "AUDIO_LOCK_UPDATE_MANIFEST_URL" -PreviousValue $prevManifestUrl
    Restore-EnvVar -Name "AUDIO_LOCK_REQUIRE_AUTHENTICODE" -PreviousValue $prevRequireAuthenticode
    Restore-EnvVar -Name "AUDIO_LOCK_EXPECTED_PUBLISHER" -PreviousValue $prevExpectedPublisher
    Pop-Location
}
