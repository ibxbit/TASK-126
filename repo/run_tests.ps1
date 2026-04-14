# Shoreline Property Operations Console — full test runner (Windows).
# Runs the Rust backend suite plus frontend type-check and unit tests.
#
# Usage:   .\run_tests.ps1
# Exit:    non-zero on any failure; zero on all-green.

$ErrorActionPreference = "Stop"

Push-Location $PSScriptRoot
try {
    Write-Host "── Rust backend tests ──────────────────────────────────────────" -ForegroundColor Cyan
    cargo test --manifest-path src-tauri/Cargo.toml --all-features
    if ($LASTEXITCODE -ne 0) { throw "cargo test failed ($LASTEXITCODE)" }

    Write-Host ""
    Write-Host "── Frontend type-check ─────────────────────────────────────────" -ForegroundColor Cyan
    if (Get-Command pnpm -ErrorAction SilentlyContinue) {
        pnpm typecheck
        if ($LASTEXITCODE -ne 0) { throw "typecheck failed ($LASTEXITCODE)" }
    } elseif (Get-Command npm -ErrorAction SilentlyContinue) {
        npx --yes tsc --noEmit
        if ($LASTEXITCODE -ne 0) { throw "tsc failed ($LASTEXITCODE)" }
    } else {
        Write-Host "[skip] no pnpm/npm found on PATH"
    }

    Write-Host ""
    Write-Host "── Frontend unit tests ─────────────────────────────────────────" -ForegroundColor Cyan
    if (Test-Path "package.json") {
        if (Get-Command pnpm -ErrorAction SilentlyContinue) {
            pnpm test --run
            if ($LASTEXITCODE -ne 0) { throw "pnpm test failed ($LASTEXITCODE)" }
        } elseif (Get-Command npm -ErrorAction SilentlyContinue) {
            npm test -- --run
            if ($LASTEXITCODE -ne 0) { throw "npm test failed ($LASTEXITCODE)" }
        }
    } else {
        Write-Host "[skip] no package.json"
    }

    Write-Host ""
    Write-Host "All tests passed." -ForegroundColor Green
}
finally {
    Pop-Location
}
