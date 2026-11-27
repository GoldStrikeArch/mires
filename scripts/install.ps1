<#
.SYNOPSIS
    Installs cargo-mires on Windows (PowerShell 5.1 and PowerShell 7+).

.DESCRIPTION
    Downloads the release zip archive and SHA256SUMS, strictly verifies the
    SHA-256 digest, installs cargo-mires.exe, and runs `cargo-mires.exe setup`.

    Usage:
      irm https://github.com/GoldStrikeArch/mires/releases/latest/download/install.ps1 | iex

    Environment variables:
      MIRES_VERSION      Release version to install (default: latest; accepts "0.1.0" or "v0.1.0")
      MIRES_INSTALL_DIR  Destination directory (default: $env:USERPROFILE\.cargo\bin)
      MIRES_SKIP_SETUP   Set to "1" to skip running `cargo-mires.exe setup`
      MIRES_RELEASE_URL  Override release download base URL (for mirrors or local testing)
#>

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

function Invoke-MiresInstall {
    if ($PSVersionTable.PSVersion.Major -lt 6) {
        [Net.ServicePointManager]::SecurityProtocol = `
            [Net.ServicePointManager]::SecurityProtocol -bor [Net.SecurityProtocolType]::Tls12
    }

    if (-not [Environment]::Is64BitOperatingSystem) {
        throw "Unsupported Windows architecture: cargo-mires requires 64-bit Windows (x86_64-pc-windows-msvc)."
    }

    $repoUrl = "https://github.com/GoldStrikeArch/mires"
    $apiLatestUrl = "https://api.github.com/repos/GoldStrikeArch/mires/releases/latest"
    $target = "x86_64-pc-windows-msvc"

    function Invoke-Download {
        param(
            [Parameter(Mandatory = $true)][string]$Uri,
            [Parameter(Mandatory = $true)][string]$OutFile
        )
        $prevProgress = $ProgressPreference
        $ProgressPreference = "SilentlyContinue"
        try {
            $params = @{
                Uri     = $Uri
                OutFile = $OutFile
                Headers = @{ "User-Agent" = "mires-installer" }
            }
            if ($PSVersionTable.PSVersion.Major -lt 6) {
                $params["UseBasicParsing"] = $true
            }
            Invoke-WebRequest @params
        } finally {
            $ProgressPreference = $prevProgress
        }
    }

    function Get-LatestTag {
        $release = Invoke-RestMethod `
            -Uri $apiLatestUrl `
            -Headers @{ "User-Agent" = "mires-installer" }
        if (-not $release -or -not $release.tag_name) {
            throw "Could not determine latest mires release tag from GitHub API. Set `$env:MIRES_VERSION explicitly."
        }
        return [string]$release.tag_name
    }

    function Get-ExpectedSha256 {
        param(
            [Parameter(Mandatory = $true)][string]$SumsPath,
            [Parameter(Mandatory = $true)][string]$AssetName
        )
        foreach ($rawLine in (Get-Content -Path $SumsPath -Encoding UTF8)) {
            $line = $rawLine.Trim()
            if (-not $line) { continue }
            if ($line -match '^([0-9a-fA-F]{64})\s+\*?(.+)$') {
                $digest = $Matches[1].ToLowerInvariant()
                $name = $Matches[2].Trim()
                if ($name.StartsWith("./")) { $name = $name.Substring(2) }
                if ($name -eq $AssetName) {
                    return $digest
                }
            }
        }
        return $null
    }

    function Get-VersionFromSums {
        param(
            [Parameter(Mandatory = $true)][string]$SumsPath,
            [Parameter(Mandatory = $true)][string]$TargetTriple
        )
        $pattern = '^cargo-mires-v(.+)-' + [Regex]::Escape($TargetTriple) + '\.zip$'
        foreach ($rawLine in (Get-Content -Path $SumsPath -Encoding UTF8)) {
            $line = $rawLine.Trim()
            if (-not $line) { continue }
            if ($line -match '^[0-9a-fA-F]{64}\s+\*?(.+)$') {
                $name = $Matches[1].Trim()
                if ($name.StartsWith("./")) { $name = $name.Substring(2) }
                if ($name -match $pattern) {
                    return $Matches[1]
                }
            }
        }
        return $null
    }

    $tempDir = Join-Path ([IO.Path]::GetTempPath()) ("mires-install-" + [guid]::NewGuid().ToString("N"))
    New-Item -ItemType Directory -Force -Path $tempDir | Out-Null
    $stagedExe = $null

    try {
        $requestedVersion = if ($env:MIRES_VERSION) { $env:MIRES_VERSION.Trim() } else { "latest" }
        $releaseOverride = if ($env:MIRES_RELEASE_URL) { $env:MIRES_RELEASE_URL.Trim() } else { "" }
        $sumsPath = Join-Path $tempDir "SHA256SUMS"

        if ($releaseOverride) {
            $baseUrl = $releaseOverride.TrimEnd("/")
            if (-not $requestedVersion -or $requestedVersion -eq "latest") {
                Write-Host "Downloading SHA256SUMS from $baseUrl..."
                Invoke-Download -Uri "$baseUrl/SHA256SUMS" -OutFile $sumsPath
                $version = Get-VersionFromSums -SumsPath $sumsPath -TargetTriple $target
                if (-not $version) {
                    throw "Could not infer version for '$target' from $baseUrl/SHA256SUMS. Set `$env:MIRES_VERSION explicitly."
                }
            } else {
                $version = $requestedVersion.TrimStart("v")
                $tagSuffix = "/v$version"
                if ($baseUrl.EndsWith($tagSuffix)) {
                    Write-Host "Downloading SHA256SUMS from $baseUrl..."
                    Invoke-Download -Uri "$baseUrl/SHA256SUMS" -OutFile $sumsPath
                } else {
                    try {
                        Invoke-Download -Uri "$baseUrl$tagSuffix/SHA256SUMS" -OutFile $sumsPath
                        $baseUrl = "$baseUrl$tagSuffix"
                    } catch {
                        Write-Host "Downloading SHA256SUMS from $baseUrl..."
                        Invoke-Download -Uri "$baseUrl/SHA256SUMS" -OutFile $sumsPath
                    }
                }
            }
        } else {
            if (-not $requestedVersion -or $requestedVersion -eq "latest") {
                $tag = Get-LatestTag
                $version = $tag.TrimStart("v")
            } else {
                $version = $requestedVersion.TrimStart("v")
                $tag = "v$version"
            }
            $baseUrl = "$repoUrl/releases/download/$tag"
            Write-Host "Downloading SHA256SUMS for cargo-mires v$version..."
            Invoke-Download -Uri "$baseUrl/SHA256SUMS" -OutFile $sumsPath
        }

        if (-not $version) {
            throw "Resolved release version is empty."
        }

        $archiveName = "cargo-mires-v$version-$target.zip"
        $archivePath = Join-Path $tempDir $archiveName

        Write-Host "Downloading $archiveName..."
        Invoke-Download -Uri "$baseUrl/$archiveName" -OutFile $archivePath

        Write-Host "Verifying SHA-256 checksum..."
        $expectedSha = Get-ExpectedSha256 -SumsPath $sumsPath -AssetName $archiveName
        if (-not $expectedSha) {
            throw "Checksum for '$archiveName' was not found in SHA256SUMS."
        }

        $actualSha = (Get-FileHash -Path $archivePath -Algorithm SHA256).Hash.ToLowerInvariant()
        if ($actualSha -ne $expectedSha) {
            throw "SHA-256 checksum mismatch for $archiveName`n  expected: $expectedSha`n  actual:   $actualSha"
        }

        $extractDir = Join-Path $tempDir "extract"
        Expand-Archive -Path $archivePath -DestinationPath $extractDir -Force

        $extractedExe = Join-Path $extractDir "cargo-mires.exe"
        if (-not (Test-Path -LiteralPath $extractedExe -PathType Leaf)) {
            throw "Archive '$archiveName' did not contain expected 'cargo-mires.exe' at archive root."
        }

        $installDir = if ($env:MIRES_INSTALL_DIR) {
            $env:MIRES_INSTALL_DIR.Trim()
        } else {
            Join-Path $env:USERPROFILE ".cargo\bin"
        }

        New-Item -ItemType Directory -Force -Path $installDir | Out-Null
        $destExe = Join-Path $installDir "cargo-mires.exe"
        $stagedExe = Join-Path $installDir (".cargo-mires-" + [guid]::NewGuid().ToString("N") + ".tmp")

        Copy-Item -LiteralPath $extractedExe -Destination $stagedExe -Force
        Move-Item -LiteralPath $stagedExe -Destination $destExe -Force
        $stagedExe = $null

        Write-Host "Installed cargo-mires v$version to $destExe"

        $normalizedInstallDir = $installDir.TrimEnd("\")
        $onPath = $false
        if ($env:PATH) {
            foreach ($entry in ($env:PATH -split ";")) {
                if ($entry.Trim().TrimEnd("\") -ieq $normalizedInstallDir) {
                    $onPath = $true
                    break
                }
            }
        }

        if (-not $onPath) {
            Write-Host ""
            Write-Host "Note: $installDir is not currently on your PATH."
            Write-Host "Add it for the current session:"
            Write-Host "  `$env:PATH = `"$installDir;`$env:PATH`""
            Write-Host "Or add it permanently for your user account:"
            Write-Host "  [Environment]::SetEnvironmentVariable('PATH', `"$installDir;`" + [Environment]::GetEnvironmentVariable('PATH', 'User'), 'User')"
        }

        if ($env:MIRES_SKIP_SETUP -eq "1") {
            Write-Host ""
            Write-Host "Skipping toolchain setup (MIRES_SKIP_SETUP=1)."
            Write-Host "Run '$destExe setup' when ready to download the compiler toolchain."
        } else {
            Write-Host ""
            Write-Host "Running 'cargo-mires.exe setup' to install the prebuilt compiler toolchain..."
            if ($releaseOverride) {
                & $destExe setup --version $version --url $baseUrl
            } else {
                & $destExe setup --version $version
            }
            if ($LASTEXITCODE -ne 0) {
                throw "cargo-mires.exe setup failed with exit code $LASTEXITCODE"
            }
        }
    } finally {
        if ($stagedExe -and (Test-Path -LiteralPath $stagedExe)) {
            Remove-Item -LiteralPath $stagedExe -Force -ErrorAction SilentlyContinue
        }
        Remove-Item -Recurse -Force $tempDir -ErrorAction SilentlyContinue
    }
}

Invoke-MiresInstall
