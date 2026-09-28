# Conduit Desktop Build Script (Windows)
param(
    [switch]$Release,
    [switch]$Clean
)

$ErrorActionPreference = "Stop"
$DesktopDir = Join-Path $PSScriptRoot "..\apps\desktop"

Write-Host "=== Conduit Desktop Build ===" -ForegroundColor Cyan

if ($Clean) {
    Write-Host "Cleaning build artifacts..." -ForegroundColor Yellow
    Set-Location "$DesktopDir\src-tauri"
    cargo clean
}

Write-Host "Installing dependencies..." -ForegroundColor Yellow
Set-Location $DesktopDir
npm ci

Write-Host "Building frontend..." -ForegroundColor Yellow
npm run build

Write-Host "Building Tauri app..." -ForegroundColor Yellow
if ($Release) {
    npm run tauri -- build
} else {
    npm run tauri -- build --debug
}

Write-Host "Build complete!" -ForegroundColor Green
