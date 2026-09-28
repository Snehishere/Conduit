<#
.SYNOPSIS
    Runs every linter and test suite in the Conduit workspace.

.DESCRIPTION
    Rewritten after remediation. The previous version ran three steps and
    covered only ONE of the three Rust crates:

        [1/3] cargo clippy   in apps/desktop/src-tauri   <- desktop only
        [2/3] flutter analyze in apps/mobile
        [3/3] npx eslint src/ in apps/desktop

    services/relay (163 test functions, ~24k lines of main.rs) and
    packages/protocol - the crate that signs every message - were not
    linted, tested, or formatted by anything. That is why a relay test
    suite that did not compile, and a protocol test suite failing 3 of 206,
    both survived for as long as they did.

    This version covers all three crates and uses the workspace root, so
    `-p <crate>` is the addressing mechanism rather than the current
    directory. Crate directory names and crate names are NOT the same
    (apps/desktop/src-tauri builds a crate called `conduit`).

.NOTES
    Exit code is non-zero if ANY step failed. Per-step results are
    accumulated rather than short-circuiting, so one run tells you
    everything that is broken.

    CI (.github/workflows/ci.yml) is the authority for merge gating; this
    script is the local equivalent and deliberately runs a slightly
    larger set.
#>

[CmdletBinding()]
param(
    # Skip the slower suites (cargo build --release, playwright e2e).
    [switch]$Fast,

    # Only run the Rust gates. Useful on a machine without Node or Flutter.
    [switch]$RustOnly
)

$ErrorActionPreference = 'Continue'
$ProgressPreference = 'SilentlyContinue'

# Resolve paths from the script location, not the caller's cwd. The old
# version assumed it was run from the repo root and its Push-Location
# calls failed from anywhere else.
$RepoRoot = (Resolve-Path (Join-Path $PSScriptRoot '..')).Path

$script:Results = [System.Collections.Generic.List[object]]::new()

function Invoke-Step {
    param(
        [Parameter(Mandatory)][string] $Name,
        [Parameter(Mandatory)][string] $WorkDir,
        [Parameter(Mandatory)][string[]] $Command,
        [switch] $Optional
    )

    Write-Host ''
    Write-Host ("=" * 78) -ForegroundColor DarkGray
    Write-Host "  $Name" -ForegroundColor Cyan
    Write-Host ("=" * 78) -ForegroundColor DarkGray
    Write-Host "  cwd: $WorkDir" -ForegroundColor DarkGray
    Write-Host "  \$ $($Command -join ' ')" -ForegroundColor DarkGray
    Write-Host ''

    Push-Location $WorkDir
    try {
        & $Command[0] @($Command[1..($Command.Count - 1)])
        $code = $LASTEXITCODE
    }
    catch {
        Write-Host "  EXCEPTION: $_" -ForegroundColor Red
        $code = 1
    }
    finally {
        Pop-Location
    }

    if ($null -eq $code) { $code = 0 }

    $ok = ($code -eq 0)
    $script:Results.Add([pscustomobject]@{
        Step     = $Name
        ExitCode = $code
        Passed   = $ok
        Optional = [bool]$Optional
    })

    if ($ok) {
        Write-Host "  PASS  $Name" -ForegroundColor Green
    }
    elseif ($Optional) {
        Write-Host "  SKIP  $Name (non-blocking, exit $code)" -ForegroundColor Yellow
    }
    else {
        Write-Host "  FAIL  $Name (exit $code)" -ForegroundColor Red
    }
}

# ---------------------------------------------------------------------
# Rust - all three crates, from the workspace root
# ---------------------------------------------------------------------

Invoke-Step 'cargo fmt --all --check (workspace)' $RepoRoot @('cargo', 'fmt', '--all', '--', '--check')

Invoke-Step 'cargo clippy -p conduit-protocol' $RepoRoot @('cargo', 'clippy', '-p', 'conduit-protocol', '--all-targets', '--', '-D', 'warnings')

Invoke-Step 'cargo clippy -p relay' $RepoRoot @('cargo', 'clippy', '-p', 'relay', '--all-targets', '--', '-D', 'warnings')

Invoke-Step 'cargo clippy -p conduit (desktop)' $RepoRoot @('cargo', 'clippy', '-p', 'conduit', '--all-targets', '--', '-D', 'warnings')

Invoke-Step 'cargo test -p conduit-protocol' $RepoRoot @('cargo', 'test', '-p', 'conduit-protocol', '--locked')

Invoke-Step 'cargo test -p relay' $RepoRoot @('cargo', 'test', '-p', 'relay', '--locked')

Invoke-Step 'cargo test -p conduit (desktop)' $RepoRoot @('cargo', 'test', '-p', 'conduit', '--locked')

if (-not $Fast) {
    Invoke-Step 'cargo build --workspace --release' $RepoRoot @('cargo', 'build', '--workspace', '--release', '--locked')
}

# ---------------------------------------------------------------------
# TypeScript / React
# ---------------------------------------------------------------------

if (-not $RustOnly) {
    $desktop = Join-Path $RepoRoot 'apps/desktop'

    # -----------------------------------------------------------------
    # Brand assets - platform independent, needs only Python + Pillow.
    # Every logo/icon/favicon in the repo is generated from
    # assets/brand/conduit.mark.json, so drift here means the artwork and
    # the code that renders it have diverged.
    # -----------------------------------------------------------------

    if (Get-Command python -ErrorAction SilentlyContinue) {
        Invoke-Step 'conduit icon drift' $RepoRoot @('python', 'scripts/icons/build_icons.py', '--check')
    }
    else {
        Write-Host ''
        Write-Host '  SKIP  Conduit icon drift - `python` is not on PATH.' -ForegroundColor Yellow
    }

    if (Test-Path (Join-Path $desktop 'node_modules')) {
        Invoke-Step 'tsc --noEmit' $desktop @('npx', 'tsc', '--noEmit')
        Invoke-Step 'eslint src/' $desktop @('npx', 'eslint', 'src/')
        Invoke-Step 'vitest' $desktop @('npm', 'test')
        Invoke-Step 'vite build' $desktop @('npx', 'vite', 'build')

        # Non-blocking: this suite runs the React shell in a plain browser.
        # Tauri IPC is unavailable, so it cannot exercise the backend or
        # the webview <-> local-server handshake. See docs/TESTING.md.
        Invoke-Step 'playwright e2e' $desktop @('npm', 'run', 'test:e2e') -Optional
    }
    else {
        Write-Host ''
        Write-Host '  SKIP  Node suites - apps/desktop/node_modules is absent. Run `npm ci` in apps/desktop.' -ForegroundColor Yellow
    }

    # -----------------------------------------------------------------
    # Flutter
    # -----------------------------------------------------------------

    $mobile = Join-Path $RepoRoot 'apps/mobile'
    $flutter = Get-Command flutter -ErrorAction SilentlyContinue

    if (-not $flutter -and (Test-Path 'C:\flutter\bin\flutter.bat')) {
        $env:PATH = 'C:\flutter\bin;' + $env:PATH
        $flutter = Get-Command flutter -ErrorAction SilentlyContinue
    }

    if ($flutter) {
        Invoke-Step 'dart format --set-exit-if-changed' $mobile @('dart', 'format', '--output=none', '--set-exit-if-changed', '.') -Optional
        Invoke-Step 'flutter analyze --fatal-infos' $mobile @('flutter', 'analyze', '--fatal-infos')
        Invoke-Step 'flutter test' $mobile @('flutter', 'test')
    }
    else {
        Write-Host ''
        Write-Host '  SKIP  Flutter suites - `flutter` is not on PATH.' -ForegroundColor Yellow
    }
}

# ---------------------------------------------------------------------
# Summary
# ---------------------------------------------------------------------

Write-Host ''
Write-Host ("=" * 78) -ForegroundColor DarkGray
Write-Host '  SUMMARY' -ForegroundColor Cyan
Write-Host ("=" * 78) -ForegroundColor DarkGray

$blocking = $script:Results | Where-Object { -not $_.Passed -and -not $_.Optional }
$skipped  = $script:Results | Where-Object { -not $_.Passed -and $_.Optional }

foreach ($r in $script:Results) {
    $mark = if ($r.Passed) { 'PASS' } elseif ($r.Optional) { 'SKIP' } else { 'FAIL' }
    $col  = if ($r.Passed) { 'Green' } elseif ($r.Optional) { 'Yellow' } else { 'Red' }
    $note = if ($r.Optional -and -not $r.Passed) { '  (non-blocking)' } else { '' }
    Write-Host ("  {0,-4} {1}{2}" -f $mark, $r.Step, $note) -ForegroundColor $col
}

Write-Host ''
if ($blocking.Count -eq 0) {
    Write-Host "  All $($script:Results.Count) blocking steps passed." -ForegroundColor Green
    if ($skipped.Count -gt 0) {
        Write-Host "  $($skipped.Count) non-blocking step(s) did not pass - see above." -ForegroundColor Yellow
    }
    exit 0
}
else {
    Write-Host "  $($blocking.Count) blocking step(s) FAILED:" -ForegroundColor Red
    foreach ($r in $blocking) { Write-Host "    - $($r.Step)" -ForegroundColor Red }
    Write-Host ''
    Write-Host '  CI is the merge gate: .github\workflows\ci.yml' -ForegroundColor DarkGray
    exit 1
}
