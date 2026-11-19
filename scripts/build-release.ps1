[CmdletBinding()]
param(
    [string]$Version,
    [string]$GithubRepo,
    [string]$ManifestUrl
)

$ErrorActionPreference = "Stop"
Set-StrictMode -Version Latest

$RepoRoot = Split-Path -Parent $PSScriptRoot

function Get-CargoVersion {
    param([string]$CargoTomlPath)
    $content = Get-Content -Path $CargoTomlPath -Raw
    if ($content -match '(?ms)^\[workspace\.package\].*?^\s*version\s*=\s*"([^"]+)"') {
        return $Matches[1]
    }
    if ($content -match '(?m)^\s*version\s*=\s*"([^"]+)"') {
        return $Matches[1]
    }
    throw "Could not determine version from $CargoTomlPath"
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

Push-Location $RepoRoot
$prevGithubRepo = $env:AUDIO_LOCK_GITHUB_REPO
$prevManifestUrl = $env:AUDIO_LOCK_UPDATE_MANIFEST_URL

try {
    if ([string]::IsNullOrWhiteSpace($Version)) {
        $Version = Get-CargoVersion -CargoTomlPath (Join-Path $RepoRoot "Cargo.toml")
    }
    $Version = $Version.Trim().TrimStart('v', 'V')

    if ($PSBoundParameters.ContainsKey("GithubRepo")) {
        $env:AUDIO_LOCK_GITHUB_REPO = $GithubRepo
    }
    if ($PSBoundParameters.ContainsKey("ManifestUrl")) {
        $env:AUDIO_LOCK_UPDATE_MANIFEST_URL = $ManifestUrl
    }

    Write-Host "Building release workspace for version $Version..."
    & cargo build --release --workspace
    if ($LASTEXITCODE -ne 0) {
        throw "cargo build failed with exit code $LASTEXITCODE"
    }

    $ReleaseDir = Join-Path $RepoRoot "target\release"
    $TrayExe = Join-Path $ReleaseDir "default-audio-lock.exe"
    if (-not (Test-Path -LiteralPath $TrayExe)) {
        throw "Expected release binary not found: $TrayExe"
    }

    $DistDir = Join-Path $RepoRoot "dist"
    New-Item -ItemType Directory -Path $DistDir -Force | Out-Null

    $UpdateZip = Join-Path $DistDir "default-audio-lock-win64-update.zip"
    Compress-Archive -Path $TrayExe -DestinationPath $UpdateZip -Force

    $Sha256 = (Get-FileHash -Path $UpdateZip -Algorithm SHA256).Hash.ToLowerInvariant()
    $PublishedAt = (Get-Date).ToUniversalTime().ToString("yyyy-MM-ddTHH:mm:ssZ")

    if (-not [string]::IsNullOrWhiteSpace($GithubRepo)) {
        $RepoSlug = $GithubRepo.Trim('/')
        $AssetUrl = "https://github.com/$RepoSlug/releases/download/v$Version/default-audio-lock-win64-update.zip"
    } elseif (-not [string]::IsNullOrWhiteSpace($ManifestUrl) -and $ManifestUrl -match '/manifest\.json$') {
        $AssetUrl = $ManifestUrl -replace '/manifest\.json$', '/default-audio-lock-win64-update.zip'
    } else {
        $AssetUrl = "default-audio-lock-win64-update.zip"
    }

    $ManifestObj = [ordered]@{
        version      = $Version
        published_at = $PublishedAt
        sha256       = $Sha256
        asset_url    = $AssetUrl
    }
    $ManifestJson = $ManifestObj | ConvertTo-Json -Depth 2
    $ManifestPath = Join-Path $DistDir "manifest.json"
    [System.IO.File]::WriteAllText($ManifestPath, $ManifestJson + "`n", [System.Text.UTF8Encoding]::new($false))

    Write-Host "Created update archive: $UpdateZip"
    Write-Host "Created manifest: $ManifestPath"

    $IsccPath = Find-Iscc
    if ($IsccPath) {
        $IssPath = Join-Path $RepoRoot "installer\default-audio-lock.iss"
        & $IsccPath "/DAppVersion=$Version" $IssPath
        if ($LASTEXITCODE -ne 0) {
            throw "iscc failed with exit code $LASTEXITCODE"
        }
        Write-Host "Created installer: $(Join-Path $DistDir "DefaultAudioLock-Setup-$Version.exe")"
    } else {
        Write-Host "Note: Inno Setup 6 (iscc.exe) was not found; skipped installer build." -ForegroundColor Yellow
    }
}
finally {
    if ($null -eq $prevGithubRepo) {
        Remove-Item Env:AUDIO_LOCK_GITHUB_REPO -ErrorAction SilentlyContinue
    } else {
        $env:AUDIO_LOCK_GITHUB_REPO = $prevGithubRepo
    }
    if ($null -eq $prevManifestUrl) {
        Remove-Item Env:AUDIO_LOCK_UPDATE_MANIFEST_URL -ErrorAction SilentlyContinue
    } else {
        $env:AUDIO_LOCK_UPDATE_MANIFEST_URL = $prevManifestUrl
    }
    Pop-Location
}
