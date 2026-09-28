# Conduit Mobile Build Script
param(
    [ValidateSet("android", "ios", "all")]
    [string]$Target = "all",
    [switch]$Release
)

$ErrorActionPreference = "Stop"
$MobileDir = Join-Path $PSScriptRoot "..\apps\mobile"

Write-Host "=== Conduit Mobile Build ===" -ForegroundColor Cyan
Set-Location $MobileDir

Write-Host "Getting dependencies..." -ForegroundColor Yellow
flutter pub get

Write-Host "Running analysis..." -ForegroundColor Yellow
flutter analyze

if ($Target -eq "android" -or $Target -eq "all") {
    Write-Host "Building Android..." -ForegroundColor Yellow
    if ($Release) {
        flutter build apk --release
    } else {
        flutter build apk --debug
    }
}

if ($Target -eq "ios" -or $Target -eq "all") {
    Write-Host "Building iOS..." -ForegroundColor Yellow
    if ($Release) {
        flutter build ios --release --no-codesign
    } else {
        flutter build ios --debug --no-codesign
    }
}

Write-Host "Build complete!" -ForegroundColor Green
