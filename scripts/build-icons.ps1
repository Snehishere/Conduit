# Conduit Brand Icon Build / Verify (Windows)
#
# Every Conduit logo, icon and favicon in this repository is generated from
# assets/brand/conduit.mark.json. This script is a thin wrapper over the same
# two commands the npm scripts expose, matching the build-*.ps1 / lint-all.ps1
# convention.
#
#   .\scripts\build-icons.ps1            regenerate every asset + the manifest
#   .\scripts\build-icons.ps1 -Check     fail if anything is stale (no writes)
param(
    [switch]$Check
)

$ErrorActionPreference = "Stop"
$Repo = Resolve-Path (Join-Path $PSScriptRoot "..")
$Script = Join-Path $Repo "scripts\icons\build_icons.py"

if (-not (Get-Command python -ErrorAction SilentlyContinue)) {
    throw "python is required to build Conduit icons. Install Python 3.9+ and 'pip install Pillow'."
}

Write-Host "=== Conduit Icons ===" -ForegroundColor Cyan

if ($Check) {
    Write-Host "Verifying every asset is a fresh render of conduit.mark.json..." -ForegroundColor Yellow
    python $Script --check
    if ($LASTEXITCODE -ne 0) {
        throw "Icon assets are stale. Run '.\scripts\build-icons.ps1' (or 'npm run icons' in apps\desktop) and commit the result."
    }
    Write-Host "Icons are in sync." -ForegroundColor Green
} else {
    python $Script
    if ($LASTEXITCODE -ne 0) { throw "Icon generation failed." }
    Write-Host "Icons written. Review and commit assets/brand/icon-manifest.json with them." -ForegroundColor Green
}
