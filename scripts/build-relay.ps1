# Conduit Relay Build Script
param(
    [switch]$Release,
    [switch]$Docker
)

$ErrorActionPreference = "Stop"
$RelayDir = Join-Path $PSScriptRoot "..\services\relay"

Write-Host "=== Conduit Relay Build ===" -ForegroundColor Cyan

if ($Docker) {
    Write-Host "Building Docker image..." -ForegroundColor Yellow
    # Build context must be the repo root: the workspace Cargo.toml and the
    # packages/protocol crate live outside services/relay.
    $RepoRoot = Join-Path $PSScriptRoot ".."
    docker build -t conduit-relay -f (Join-Path $RelayDir "Dockerfile") $RepoRoot
    Write-Host "Docker image built: conduit-relay" -ForegroundColor Green
} else {
    Set-Location $RelayDir
    Write-Host "Building relay server..." -ForegroundColor Yellow
    if ($Release) {
        cargo build --release
    } else {
        cargo build
    }
    Write-Host "Build complete!" -ForegroundColor Green
}
