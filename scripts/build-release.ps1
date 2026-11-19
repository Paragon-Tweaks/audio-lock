[CmdletBinding()]
param(
    [string]$Version,
    [string]$GithubRepo = "Paragon-Tweaks/audio-lock",
    [string]$CertSha1,
    [string]$CertSubject,
    [string]$TimestampUrl = "http://timestamp.sectigo.com",
    [switch]$NoPublish,
    [switch]$Draft,
    [switch]$AllowDirty,
    [switch]$SkipTests,
    [switch]$Unsigned
)

$ErrorActionPreference = "Stop"
Set-StrictMode -Version Latest

$RepoRoot = Split-Path -Parent $PSScriptRoot
$SemverRegex = '^\d+\.\d+\.\d+(-[0-9A-Za-z.-]+)?$'
$FallbackTimestampUrl = "http://timestamp.digicert.com"

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

function Find-SignTool {
    $cmd = Get-Command "signtool.exe" -ErrorAction SilentlyContinue
    if ($cmd) {
        return $cmd.Source
    }

    $kitRoots = @(
        "${env:ProgramFiles(x86)}\Windows Kits\10\bin",
        "$env:ProgramFiles\Windows Kits\10\bin"
    )

    $candidates = @()
    foreach ($kitRoot in $kitRoots) {
        if (-not $kitRoot -or -not (Test-Path -LiteralPath $kitRoot)) {
            continue
        }
        $found = @(Get-ChildItem -Path (Join-Path $kitRoot "*\x64\signtool.exe") -File -ErrorAction SilentlyContinue)
        foreach ($item in $found) {
            $verFolder = Split-Path -Leaf (Split-Path -Parent $item.DirectoryName)
            $parsedVer = [version]"0.0.0.0"
            [void][version]::TryParse($verFolder, [ref]$parsedVer)
            $candidates += [pscustomobject]@{
                Path    = $item.FullName
                Version = $parsedVer
            }
        }
    }

    if ($candidates.Count -gt 0) {
        return ($candidates | Sort-Object -Property Version -Descending | Select-Object -First 1).Path
    }

    foreach ($kitRoot in $kitRoots) {
        if (-not $kitRoot) { continue }
        $flatCandidate = Join-Path $kitRoot "x64\signtool.exe"
        if (Test-Path -LiteralPath $flatCandidate) {
            return $flatCandidate
        }
    }

    return $null
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

function Invoke-NativeProbe {
    param(
        [string]$Executable,
        [string[]]$Arguments
    )
    $prevEap = $ErrorActionPreference
    $ErrorActionPreference = "Continue"
    try {
        $output = & $Executable @Arguments 2>&1
        $exitCode = $LASTEXITCODE
        return [pscustomobject]@{
            ExitCode = $exitCode
            Output   = ($output | Out-String).Trim()
        }
    } finally {
        $ErrorActionPreference = $prevEap
    }
}

function Invoke-SignAndVerify {
    param(
        [string]$SignToolPath,
        [string]$FilePath,
        [string]$Description,
        [string]$Sha1Thumbprint,
        [string]$SubjectName,
        [string]$PrimaryTimestampUrl,
        [string]$BackupTimestampUrl
    )

    $baseArgs = @(
        "sign",
        "/fd", "sha256",
        "/td", "sha256",
        "/d", $Description,
        "/du", "https://paragontweaks.net"
    )

    if (-not [string]::IsNullOrWhiteSpace($Sha1Thumbprint)) {
        $baseArgs += @("/sha1", $Sha1Thumbprint)
    } elseif (-not [string]::IsNullOrWhiteSpace($SubjectName)) {
        $baseArgs += @("/n", $SubjectName)
    } else {
        $baseArgs += "/a"
    }

    Write-Host "Signing '$FilePath' with timestamp server '$PrimaryTimestampUrl'..."
    $primaryArgs = $baseArgs + @("/tr", $PrimaryTimestampUrl, $FilePath)
    & $SignToolPath @primaryArgs
    $signExit = $LASTEXITCODE

    if ($signExit -ne 0 -and $PrimaryTimestampUrl -ne $BackupTimestampUrl) {
        Write-Host "Primary timestamp server failed (exit code $signExit); retrying once with '$BackupTimestampUrl'..." -ForegroundColor Yellow
        $fallbackArgs = $baseArgs + @("/tr", $BackupTimestampUrl, $FilePath)
        & $SignToolPath @fallbackArgs
        $signExit = $LASTEXITCODE
    }

    if ($signExit -ne 0) {
        throw "signtool.exe failed to sign '$FilePath' (exit code $signExit). Ensure your Sectigo hardware token is plugged in, unlocked, and the matching certificate is available."
    }

    Write-Host "Verifying Authenticode signature on '$FilePath'..."
    & $SignToolPath verify /pa /v $FilePath
    if ($LASTEXITCODE -ne 0) {
        throw "Authenticode signature verification failed for '$FilePath' (exit code $LASTEXITCODE)."
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
$StageDir = Join-Path $RepoRoot "dist\stage"

try {
    # 1. Validate semver matches Cargo.toml [workspace.package].version.
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

    $TagName = "v$Version"
    $repoSlug = Resolve-GithubRepoSlug -RepoParam $GithubRepo

    $normalizedSha1 = $null
    if (-not [string]::IsNullOrWhiteSpace($CertSha1)) {
        $normalizedSha1 = ($CertSha1 -replace '[\s:-]', '').Trim()
        if ($normalizedSha1 -notmatch '^[0-9a-fA-F]{40}$') {
            throw "Invalid -CertSha1 '$CertSha1'. Expected a 40-character hexadecimal SHA-1 thumbprint."
        }
    }

    $normalizedSubject = $null
    if (-not [string]::IsNullOrWhiteSpace($CertSubject)) {
        $normalizedSubject = $CertSubject.Trim()
    }

    # 2. Verify clean git working tree when publishing a signed release.
    if (-not $AllowDirty -and -not $NoPublish -and -not $Unsigned) {
        $gitCmd = Get-Command "git" -ErrorAction SilentlyContinue
        if (-not $gitCmd) {
            throw "git was not found on PATH. Install Git or pass -NoPublish."
        }
        $gitStatusProbe = Invoke-NativeProbe -Executable "git" -Arguments @("status", "--porcelain")
        if ($gitStatusProbe.ExitCode -ne 0) {
            throw "git status failed: $($gitStatusProbe.Output)"
        }
        if (-not [string]::IsNullOrWhiteSpace($gitStatusProbe.Output)) {
            throw "Git working tree is dirty. Commit or stash changes before publishing a release (or pass -AllowDirty or -NoPublish):`n$($gitStatusProbe.Output)"
        }
    }

    # 3. Locate signtool.exe (when signing) and ISCC.exe (Inno Setup 6).
    $SignToolPath = $null
    if (-not $Unsigned) {
        $SignToolPath = Find-SignTool
        if (-not $SignToolPath) {
            throw "signtool.exe was not found on PATH or in Windows Kits 10. Install the Windows 10/11 SDK (for example: winget install Microsoft.WindowsSDK.10.0.26100) or pass -Unsigned only for manual unsigned CI builds."
        }
        Write-Host "Using signtool.exe: $SignToolPath"
    } else {
        Write-Host "WARNING: Running in -Unsigned mode. Output binaries will NOT be Authenticode-signed." -ForegroundColor Yellow
    }

    $IsccPath = Find-Iscc
    if (-not $IsccPath) {
        if (-not $NoPublish) {
            throw "Inno Setup 6 (ISCC.exe) is required when publishing so releases always include the installer. Install it with: winget install JRSoftware.InnoSetup"
        }
        Write-Host "Note: Inno Setup 6 (ISCC.exe) was not found; skipping installer build (-NoPublish). Install with: winget install JRSoftware.InnoSetup" -ForegroundColor Yellow
    } else {
        Write-Host "Using ISCC.exe: $IsccPath"
    }

    # 4. Preflight GitHub CLI when publishing.
    if (-not $NoPublish) {
        $ghCmd = Get-Command "gh" -ErrorAction SilentlyContinue
        if (-not $ghCmd) {
            throw "GitHub CLI (gh) was not found on PATH. Install it with: winget install GitHub.cli"
        }

        $authProbe = Invoke-NativeProbe -Executable "gh" -Arguments @("auth", "status")
        if ($authProbe.ExitCode -ne 0) {
            throw "GitHub CLI is not authenticated. Run 'gh auth login' and retry.`n$($authProbe.Output)"
        }

        $existingReleaseProbe = Invoke-NativeProbe -Executable "gh" -Arguments @("release", "view", $TagName, "--repo", $repoSlug)
        if ($existingReleaseProbe.ExitCode -eq 0) {
            throw "GitHub Release '$TagName' already exists in $repoSlug. Bump Cargo.toml version or delete the existing release before publishing."
        }
    }

    # 5. Run workspace clippy and unit tests unless -SkipTests.
    if (-not $SkipTests) {
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
        Write-Host "Skipping clippy and unit tests (-SkipTests)." -ForegroundColor Yellow
    }

    # 6. Configure release build environment and build --release.
    $env:AUDIO_LOCK_GITHUB_REPO = $repoSlug
    Remove-Item Env:AUDIO_LOCK_UPDATE_MANIFEST_URL -ErrorAction SilentlyContinue
    if (-not $Unsigned) {
        $env:AUDIO_LOCK_REQUIRE_AUTHENTICODE = "1"
        if ($null -ne $normalizedSubject) {
            $env:AUDIO_LOCK_EXPECTED_PUBLISHER = $normalizedSubject
        } else {
            Remove-Item Env:AUDIO_LOCK_EXPECTED_PUBLISHER -ErrorAction SilentlyContinue
        }
    } else {
        Remove-Item Env:AUDIO_LOCK_REQUIRE_AUTHENTICODE -ErrorAction SilentlyContinue
        Remove-Item Env:AUDIO_LOCK_EXPECTED_PUBLISHER -ErrorAction SilentlyContinue
    }

    $ReleaseDir = Join-Path $RepoRoot "target\release"
    $TrayExe = Join-Path $ReleaseDir "default-audio-lock.exe"
    if (Test-FileLocked -Path $TrayExe) {
        throw "Cannot overwrite '$TrayExe' because it is locked by a running instance. Quit the running Audio Lock tray app first and retry."
    }

    Write-Host "Building release workspace for Audio Lock $TagName..."
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

    # 7. Stage binary into dist\stage\default-audio-lock.exe without modifying target\release.
    $DistDir = Join-Path $RepoRoot "dist"
    New-Item -ItemType Directory -Path $DistDir -Force | Out-Null

    if (Test-Path -LiteralPath $StageDir) {
        Remove-Item -LiteralPath $StageDir -Recurse -Force
    }
    foreach ($oldInstaller in @(Get-ChildItem -Path (Join-Path $DistDir "DefaultAudioLock-Setup-*.exe") -File -ErrorAction SilentlyContinue)) {
        Remove-Item -LiteralPath $oldInstaller.FullName -Force
    }

    New-Item -ItemType Directory -Path $StageDir -Force | Out-Null
    $StageExe = Join-Path $StageDir "default-audio-lock.exe"
    Copy-Item -LiteralPath $TrayExe -Destination $StageExe -Force

    # 8. Sign and verify staged binary when signing.
    if (-not $Unsigned) {
        Invoke-SignAndVerify `
            -SignToolPath $SignToolPath `
            -FilePath $StageExe `
            -Description "Audio Lock" `
            -Sha1Thumbprint $normalizedSha1 `
            -SubjectName $normalizedSubject `
            -PrimaryTimestampUrl $TimestampUrl `
            -BackupTimestampUrl $FallbackTimestampUrl
    }

    # 9. Package staged binary into update zip and generate manifest.json.
    $UpdateZip = Join-Path $DistDir "default-audio-lock-win64-update.zip"
    Compress-Archive -Path $StageExe -DestinationPath $UpdateZip -Force

    $Sha256 = (Get-FileHash -Path $UpdateZip -Algorithm SHA256).Hash.ToLowerInvariant()
    $PublishedAt = (Get-Date).ToUniversalTime().ToString("yyyy-MM-ddTHH:mm:ssZ")
    $AssetUrl = "https://github.com/$repoSlug/releases/download/$TagName/default-audio-lock-win64-update.zip"

    $ManifestObj = [ordered]@{
        version      = $Version
        published_at = $PublishedAt
        sha256       = $Sha256
        asset_url    = $AssetUrl
    }
    $ManifestJson = $ManifestObj | ConvertTo-Json -Depth 2
    $ManifestPath = Join-Path $DistDir "manifest.json"
    [System.IO.File]::WriteAllText($ManifestPath, $ManifestJson + "`n", [System.Text.UTF8Encoding]::new($false))

    # 10. Compile Inno Setup installer using the staged binary.
    $InstallerPath = Join-Path $DistDir "DefaultAudioLock-Setup-$Version.exe"
    if ($IsccPath) {
        $IssPath = Join-Path $RepoRoot "installer\default-audio-lock.iss"
        Write-Host "Compiling Inno Setup installer..."
        & $IsccPath "/DMyAppVersion=$Version" "/DAppExePath=$StageExe" $IssPath
        if ($LASTEXITCODE -ne 0) {
            throw "ISCC.exe failed with exit code $LASTEXITCODE"
        }
        if (-not (Test-Path -LiteralPath $InstallerPath)) {
            throw "Expected installer not found after ISCC build: $InstallerPath"
        }

        # 11. Sign and verify the compiled installer when signing.
        if (-not $Unsigned) {
            Invoke-SignAndVerify `
                -SignToolPath $SignToolPath `
                -FilePath $InstallerPath `
                -Description "Audio Lock Setup" `
                -Sha1Thumbprint $normalizedSha1 `
                -SubjectName $normalizedSubject `
                -PrimaryTimestampUrl $TimestampUrl `
                -BackupTimestampUrl $FallbackTimestampUrl
        }
    }

    # 12. Clean up temporary dist\stage\ folder.
    if (Test-Path -LiteralPath $StageDir) {
        Remove-Item -LiteralPath $StageDir -Recurse -Force
    }

    Write-Host ""
    Write-Host "Release assets ready in ${DistDir}:" -ForegroundColor Green
    foreach ($asset in @($InstallerPath, $UpdateZip, $ManifestPath)) {
        if (Test-Path -LiteralPath $asset) {
            $item = Get-Item -LiteralPath $asset
            $mb = [Math]::Round($item.Length / 1MB, 2)
            $hash = (Get-FileHash -LiteralPath $asset -Algorithm SHA256).Hash.ToLowerInvariant()
            Write-Host ("  - {0} ({1:N2} MB / {2:N0} bytes) sha256:{3}" -f $item.Name, $mb, $item.Length, $hash)
        }
    }

    # 13. Create/push git tag and publish GitHub Release unless -NoPublish.
    if (-not $NoPublish) {
        $headProbe = Invoke-NativeProbe -Executable "git" -Arguments @("rev-parse", "HEAD")
        if ($headProbe.ExitCode -ne 0) {
            throw "Could not resolve HEAD commit: $($headProbe.Output)"
        }
        $headCommit = $headProbe.Output.Trim()

        $tagProbe = Invoke-NativeProbe -Executable "git" -Arguments @("rev-parse", "--verify", "refs/tags/$TagName")
        if ($tagProbe.ExitCode -ne 0) {
            Write-Host "Creating annotated git tag $TagName..."
            & git tag -a $TagName -m "Audio Lock $TagName"
            if ($LASTEXITCODE -ne 0) {
                throw "git tag failed with exit code $LASTEXITCODE"
            }
        } else {
            $tagCommitProbe = Invoke-NativeProbe -Executable "git" -Arguments @("rev-parse", "$TagName^{commit}")
            if ($tagCommitProbe.ExitCode -ne 0 -or $tagCommitProbe.Output.Trim() -ne $headCommit) {
                throw "Local git tag '$TagName' already exists and points to '$($tagCommitProbe.Output.Trim())' instead of HEAD '$headCommit'."
            }
            Write-Host "Reusing existing local git tag $TagName at HEAD."
        }

        Write-Host "Pushing tag $TagName to origin..."
        & git push origin $TagName
        if ($LASTEXITCODE -ne 0) {
            throw "git push origin $TagName failed with exit code $LASTEXITCODE"
        }

        $ghArgs = @(
            "release", "create", $TagName,
            "--repo", $repoSlug,
            "--title", "Audio Lock $TagName",
            "--generate-notes"
        )
        if ($Draft) {
            $ghArgs += "--draft"
        }
        $ghArgs += @(
            $InstallerPath,
            $UpdateZip,
            $ManifestPath
        )

        $releaseKind = if ($Draft) { "draft GitHub Release" } else { "GitHub Release" }
        Write-Host "Publishing $releaseKind $TagName to $repoSlug..."
        & gh @ghArgs
        if ($LASTEXITCODE -ne 0) {
            throw "gh release create failed with exit code $LASTEXITCODE"
        }

        Write-Host "Published $releaseKind $TagName successfully." -ForegroundColor Green
    } else {
        Write-Host "Skipped git tag push and GitHub Release creation (-NoPublish)." -ForegroundColor Yellow
    }
}
finally {
    if (Test-Path -LiteralPath $StageDir) {
        Remove-Item -LiteralPath $StageDir -Recurse -Force -ErrorAction SilentlyContinue
    }
    Restore-EnvVar -Name "AUDIO_LOCK_GITHUB_REPO" -PreviousValue $prevGithubRepo
    Restore-EnvVar -Name "AUDIO_LOCK_UPDATE_MANIFEST_URL" -PreviousValue $prevManifestUrl
    Restore-EnvVar -Name "AUDIO_LOCK_REQUIRE_AUTHENTICODE" -PreviousValue $prevRequireAuthenticode
    Restore-EnvVar -Name "AUDIO_LOCK_EXPECTED_PUBLISHER" -PreviousValue $prevExpectedPublisher
    Pop-Location
}
