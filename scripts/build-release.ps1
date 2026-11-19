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
    if ($content -match '(?ms)^\[workspace\.package\]\s*\r?\n(.*?)(?=^\[|\z)' -and $Matches[1] -match '(?m)^\s*version\s*=\s*"([^"]+)"') {
        return $Matches[1].Trim()
    }
    throw "Could not determine [workspace.package].version from $CargoTomlPath"
}

function Resolve-AssetUrl {
    param(
        [string]$ReleaseVersion,
        [string]$RepoParam,
        [string]$RepoEnv,
        [string]$ManifestUrlParam
    )

    $usageMessage = "Provide -GithubRepo <owner/repo>, set AUDIO_LOCK_GITHUB_REPO=<owner/repo>, or pass -ManifestUrl <https://.../manifest.json or loopback http://.../manifest.json>."

    $repoCandidate = $null
    $repoSource = $null
    if (-not [string]::IsNullOrWhiteSpace($RepoParam)) {
        $repoCandidate = $RepoParam
        $repoSource = "-GithubRepo"
    } elseif (-not [string]::IsNullOrWhiteSpace($RepoEnv)) {
        $repoCandidate = $RepoEnv
        $repoSource = "AUDIO_LOCK_GITHUB_REPO"
    }

    if ($null -ne $repoCandidate) {
        $repoSlug = $repoCandidate.Trim().Trim('/')
        if ($repoSlug -notmatch '^[A-Za-z0-9_.-]+/[A-Za-z0-9_.-]+$' -or $repoSlug.StartsWith('OWNER/', [System.StringComparison]::OrdinalIgnoreCase)) {
            throw "Invalid repository slug '$repoCandidate' from $repoSource. Expected '<owner>/<repo>'. $usageMessage"
        }
        return "https://github.com/$repoSlug/releases/download/v$ReleaseVersion/default-audio-lock-win64-update.zip"
    }

    if (-not [string]::IsNullOrWhiteSpace($ManifestUrlParam)) {
        $trimmedManifest = $ManifestUrlParam.Trim()
        $parsedUri = $null
        if (-not [System.Uri]::TryCreate($trimmedManifest, [System.UriKind]::Absolute, [ref]$parsedUri)) {
            throw "Invalid -ManifestUrl '$ManifestUrlParam'. Expected a valid absolute https:// or loopback http:// URL. $usageMessage"
        }

        $isHttps = $parsedUri.Scheme -eq 'https'
        $isLoopbackHttp = ($parsedUri.Scheme -eq 'http') -and $parsedUri.IsLoopback
        if (-not ($isHttps -or $isLoopbackHttp) -or [string]::IsNullOrWhiteSpace($parsedUri.Host)) {
            throw "Invalid -ManifestUrl '$ManifestUrlParam'. Only https:// or loopback http:// URLs are allowed. $usageMessage"
        }

        return ([System.Uri]::new($parsedUri, "default-audio-lock-win64-update.zip")).AbsoluteUri
    }

    throw "Cannot determine an absolute asset_url for manifest.json. $usageMessage"
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
    $CargoVersion = Get-CargoVersion -CargoTomlPath (Join-Path $RepoRoot "Cargo.toml")
    if ([string]::IsNullOrWhiteSpace($Version)) {
        $Version = $CargoVersion
    } else {
        $Version = $Version.Trim() -replace '^[vV]', ''
        if ($Version -ne $CargoVersion) {
            throw "Version mismatch: -Version '$Version' does not match Cargo.toml [workspace.package].version '$CargoVersion'."
        }
    }

    $AssetUrl = Resolve-AssetUrl `
        -ReleaseVersion $Version `
        -RepoParam $GithubRepo `
        -RepoEnv $prevGithubRepo `
        -ManifestUrlParam $ManifestUrl

    $hasGithubRepo = $PSBoundParameters.ContainsKey("GithubRepo") -and -not [string]::IsNullOrWhiteSpace($GithubRepo)
    $hasManifestUrl = $PSBoundParameters.ContainsKey("ManifestUrl") -and -not [string]::IsNullOrWhiteSpace($ManifestUrl)

    if ($hasGithubRepo) {
        $env:AUDIO_LOCK_GITHUB_REPO = $GithubRepo.Trim().Trim('/')
        if (-not $hasManifestUrl) {
            Remove-Item Env:AUDIO_LOCK_UPDATE_MANIFEST_URL -ErrorAction SilentlyContinue
        }
    }
    if ($hasManifestUrl) {
        $env:AUDIO_LOCK_UPDATE_MANIFEST_URL = $ManifestUrl.Trim()
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
