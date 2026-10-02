# Conduit Vendored OpenSSL Fetch (Windows)
#
# The desktop crate (apps/desktop/src-tauri, package `conduit`) depends on
# rusqlite with the `bundled-sqlcipher` feature. That compiles SQLCipher from
# source but still links against a system crypto library, and on Windows the
# repo supplies one from a gitignored directory:
#
#     <repo>/.tools/openssl-win64/{include,lib/static}
#
# `.tools/` is in .gitignore, so a fresh clone does NOT have it and a first
# `cargo build -p conduit` fails with a link error from libsqlite3-sys that does
# not name the missing directory. This script is that missing bootstrap step.
#
#     .\scripts\fetch-openssl.ps1            fetch, verify, extract
#     .\scripts\fetch-openssl.ps1 -Force     replace an existing extraction
#     .\scripts\fetch-openssl.ps1 -Check     verify what is already there, exit
#                                            non-zero if it is missing or wrong
#
# The version and the SHA-256 below are pinned here, in a tracked file, rather
# than in the untracked .tools/ directory. The hash is the digest GitHub
# publishes for the release asset, so it is verifiable independently of this
# script.
#
# Linux and macOS need none of this: they link SQLCipher against the system
# libssl-dev / brew openssl, and .cargo/config.toml no longer forces these
# variables on them.
#
# REDISTRIBUTION WARNING - READ BEFORE PACKAGING A RELEASE.
# The upstream README.txt shipped inside the archive marks `include/`,
# `lib/import/` and `lib/static/` as [non-redistributable], and
# .cargo/config.toml points SQLCipher at `lib/static/`, which is *statically*
# linked into the shipped binary. Those labels describe the archive's contents,
# not OpenSSL's licence terms, and nothing here settles whether a released
# Conduit may redistribute them. This script deliberately does not commit the
# libraries and deliberately does not answer that question; see the tracked
# item in docs/REMAINING_WORK.md (W5.2) before cutting a release.

[CmdletBinding()]
param(
    # Where to extract. Must be <repo>/.tools/openssl-win64 for .cargo/config.toml
    # to find it, which is the default. Overridable so this script can be tested
    # without touching a working copy.
    [string]$Destination,

    # Re-fetch and overwrite an existing extraction.
    [switch]$Force,

    # Verify only. Never downloads, never writes.
    [switch]$Check
)

$ErrorActionPreference = "Stop"

# ---- the pin -----------------------------------------------------------------
$OpenSslVersion = "3.5.8"
$AssetName      = "openssl-$OpenSslVersion-Windows-x64.zip"
$ArchiveUrl     = "https://github.com/TaurusTLS-Developers/OpenSSL-Distribution/releases/download/v$OpenSslVersion/$AssetName"
$ArchiveSha256  = "734b32904d11e57cdbd21b163617f5ab88bc9d094170ae1ace72c088223237e2"
$ArchiveBytes   = 26862061

# Files the desktop build actually consumes. A zip that verifies against the
# SHA-256 but lacks these is not a usable OpenSSL, and saying so here beats
# failing later inside a C compiler's output.
$RequiredPaths = @(
    "version.txt",
    "include\openssl\opensslv.h",
    "lib\static\libcrypto.lib",
    "lib\static\libssl.lib"
)

$Repo = Resolve-Path (Join-Path $PSScriptRoot "..")
if (-not $Destination) {
    $Destination = Join-Path $Repo ".tools\openssl-win64"
}

Write-Host "=== Conduit Vendored OpenSSL $OpenSslVersion ===" -ForegroundColor Cyan

function Test-OpensslTree([string]$Root) {
    $missing = @()
    foreach ($rel in $RequiredPaths) {
        if (-not (Test-Path -LiteralPath (Join-Path $Root $rel))) { $missing += $rel }
    }
    return $missing
}

function Get-TreeVersion([string]$Root) {
    $file = Join-Path $Root "version.txt"
    if (-not (Test-Path -LiteralPath $file)) { return $null }
    return ((Get-Content -LiteralPath $file -Raw) -replace '\s', "")
}

function Assert-Tree([string]$Root) {
    $missing = Test-OpensslTree $Root
    if ($missing.Count -gt 0) {
        throw ("$Root is not a usable OpenSSL $OpenSslVersion tree. Missing: " + ($missing -join ", "))
    }
    $found = Get-TreeVersion $Root
    if ($found -ne $OpenSslVersion) {
        throw "$Root reports OpenSSL $found, but this script is pinned to $OpenSslVersion."
    }
    Write-Host "Found OpenSSL $found in $Root" -ForegroundColor Green
    return $found
}

# ---- -Check ------------------------------------------------------------------
if ($Check) {
    if (-not (Test-Path -LiteralPath $Destination)) {
        throw "No vendored OpenSSL at $Destination. Run .\scripts\fetch-openssl.ps1."
    }
    Assert-Tree $Destination | Out-Null
    Write-Host "Vendored OpenSSL is present and matches the pin." -ForegroundColor Green
    return
}

# ---- already fetched? --------------------------------------------------------
if (Test-Path -LiteralPath $Destination) {
    if ($Force) {
        Write-Host "Replacing $Destination (-Force)." -ForegroundColor Yellow
        Remove-Item -LiteralPath $Destination -Recurse -Force
    } else {
        Write-Host "$Destination already exists." -ForegroundColor Yellow
        Assert-Tree $Destination | Out-Null
        Write-Host "Nothing to do. Pass -Force to re-fetch." -ForegroundColor Green
        return
    }
}

# ---- download ----------------------------------------------------------------
Write-Host "Downloading $AssetName ($([math]::Round($ArchiveBytes / 1MB, 1)) MB)..." -ForegroundColor Yellow

# Windows PowerShell 5.1 defaults to SSL3/TLS1.0 in some hosts, which GitHub
# rejects. PowerShell 7 (pwsh) already defaults to TLS 1.2+.
try {
    [Net.ServicePointManager]::SecurityProtocol = [Net.SecurityProtocolType]::Tls12
} catch {
    Write-Host "Could not pin TLS 1.2; continuing with the platform default." -ForegroundColor Yellow
}

$staging = Join-Path ([IO.Path]::GetTempPath()) ("conduit-openssl-" + [guid]::NewGuid().ToString("N"))
New-Item -ItemType Directory -Path $staging | Out-Null
$archive = Join-Path $staging $AssetName

try {
    $ProgressPreference = "SilentlyContinue"   # Invoke-WebRequest is unusably slow otherwise
    Invoke-WebRequest -Uri $ArchiveUrl -OutFile $archive -UseBasicParsing

    $actualBytes = (Get-Item -LiteralPath $archive).Length
    if ($actualBytes -ne $ArchiveBytes) {
        throw "Downloaded $actualBytes bytes, expected $ArchiveBytes. The asset has probably been replaced upstream."
    }

    Write-Host "Verifying SHA-256..." -ForegroundColor Yellow
    $actualHash = (Get-FileHash -LiteralPath $archive -Algorithm SHA256).Hash.ToLowerInvariant()
    if ($actualHash -ne $ArchiveSha256) {
        throw "SHA-256 mismatch.`n  expected $ArchiveSha256`n  actual   $actualHash`nRefusing to extract an archive that does not match the pin."
    }
    Write-Host "SHA-256 matches the pin." -ForegroundColor Green

    # The archive may or may not wrap its contents in a single top-level
    # directory. Descend only if it does and version.txt is not at the root.
    $unpack = Join-Path $staging "unpack"
    Expand-Archive -LiteralPath $archive -DestinationPath $unpack -Force
    $root = $unpack
    if (-not (Test-Path -LiteralPath (Join-Path $root "version.txt"))) {
        $dirs = @(Get-ChildItem -LiteralPath $unpack -Directory)
        $files = @(Get-ChildItem -LiteralPath $unpack -File)
        if ($dirs.Count -eq 1 -and $files.Count -eq 0) { $root = $dirs[0].FullName }
    }
    Assert-Tree $root | Out-Null

    # Record where this tree came from, so the untracked directory is at least
    # self-describing for the next person who wonders whether it is current.
    $provenance = @(
        "Fetched by scripts/fetch-openssl.ps1"
        "source:  $ArchiveUrl"
        "sha256:  $ArchiveSha256"
        "bytes:   $ArchiveBytes"
        "fetched: $([DateTime]::UtcNow.ToString('u'))"
    ) -join [Environment]::NewLine
    Set-Content -LiteralPath (Join-Path $root "FETCHED-BY-CONDUIT.txt") -Value $provenance -Encoding UTF8

    $parent = Split-Path -Parent $Destination
    if (-not (Test-Path -LiteralPath $parent)) { New-Item -ItemType Directory -Path $parent -Force | Out-Null }
    Move-Item -LiteralPath $root -Destination $Destination

    Write-Host "Extracted OpenSSL $OpenSslVersion to $Destination" -ForegroundColor Green
    Write-Host ""
    Write-Host "REDISTRIBUTION WARNING: include/ and lib/static/ are labelled" -ForegroundColor Yellow
    Write-Host "[non-redistributable] upstream and are statically linked into the" -ForegroundColor Yellow
    Write-Host "shipped binary. Settle W5.2 before packaging a release." -ForegroundColor Yellow
    Write-Host ""
    Write-Host "Now run: cargo build -p conduit" -ForegroundColor Cyan
} finally {
    if (Test-Path -LiteralPath $staging) { Remove-Item -LiteralPath $staging -Recurse -Force -ErrorAction SilentlyContinue }
}
