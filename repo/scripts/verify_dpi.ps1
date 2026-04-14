<#
.SYNOPSIS
  DPI & Window-Sizing Verification for Shoreline Property Operations Console.

.DESCRIPTION
  This script validates that the Tauri window configuration, WebView2 DPI
  awareness, and CSS layout settings are correct for high-DPI displays.
  It performs static analysis (config/code checks) and optional runtime
  checks when the application is running.

.NOTES
  Run from the repository root: .\scripts\verify_dpi.ps1
  Optional: start the app first with `pnpm tauri dev` for runtime checks.
#>

$ErrorActionPreference = "Stop"
Push-Location (Split-Path $PSScriptRoot -Parent)
$pass = 0; $fail = 0; $warn = 0

function Pass($msg) { Write-Host "  [PASS] $msg" -ForegroundColor Green; $script:pass++ }
function Fail($msg) { Write-Host "  [FAIL] $msg" -ForegroundColor Red;   $script:fail++ }
function Warn($msg) { Write-Host "  [WARN] $msg" -ForegroundColor Yellow; $script:warn++ }
function Header($msg) { Write-Host "`n== $msg ==" -ForegroundColor Cyan }

# ─── 1. Tauri config: window dimensions ─────────────────────────────────
Header "1. tauri.conf.json — Main window dimensions"

$conf = Get-Content "src-tauri\tauri.conf.json" -Raw | ConvertFrom-Json
$mainWin = $conf.app.windows[0]

if ($mainWin.width -ge 1600 -and $mainWin.height -ge 1000) {
    Pass "Main window default size: $($mainWin.width)x$($mainWin.height)"
} else {
    Fail "Main window default size $($mainWin.width)x$($mainWin.height) is below 1600x1000"
}

if ($mainWin.minWidth -ge 1280 -and $mainWin.minHeight -ge 720) {
    Pass "Main window minimum size: $($mainWin.minWidth)x$($mainWin.minHeight)"
} else {
    Fail "Main window min size $($mainWin.minWidth)x$($mainWin.minHeight) is below 1280x720"
}

if ($mainWin.resizable -eq $true) {
    Pass "Main window is resizable"
} else {
    Fail "Main window is NOT resizable — breaks DPI adaptation"
}

if ($mainWin.center -eq $true) {
    Pass "Main window centers on open"
} else {
    Warn "Main window does not auto-center"
}

# ─── 2. Workspace window dimensions (Rust code) ─────────────────────────
Header "2. Workspace windows — logical pixel sizes"

$windowsMod = Get-Content "src-tauri\src\windows\mod.rs" -Raw

$workspacePattern = 'fn default_size.*?\{([\s\S]*?)\}'
if ($windowsMod -match $workspacePattern) {
    $sizeBlock = $Matches[1]
    # Check for reasonable workspace sizes (>= 1100x720)
    $sizes = [regex]::Matches($sizeBlock, '\((\d+\.?\d*),\s*(\d+\.?\d*)\)')
    foreach ($m in $sizes) {
        $w = [double]$m.Groups[1].Value
        $h = [double]$m.Groups[2].Value
        if ($w -ge 1100 -and $h -ge 720) {
            Pass "Workspace size ${w}x${h} meets minimum"
        } else {
            Warn "Workspace size ${w}x${h} — verify readability at 150% DPI"
        }
    }
}

# Check that workspace windows enforce min_inner_size
if ($windowsMod -match 'min_inner_size\((\d+\.?\d*),\s*(\d+\.?\d*)\)') {
    $minW = $Matches[1]; $minH = $Matches[2]
    Pass "Workspace windows have min_inner_size constraint: ${minW}x${minH}"
} else {
    Fail "Workspace windows missing min_inner_size — may shrink below usable at high DPI"
}

# ─── 3. Vite build target ───────────────────────────────────────────────
Header "3. Vite config — build target"

$viteConf = Get-Content "vite.config.ts" -Raw
if ($viteConf -match 'target.*chrome\d+') {
    Pass "Vite build targets modern Chromium (WebView2 base)"
} else {
    Warn "Vite build target not set to chrome — verify WebView2 compatibility"
}

# ─── 4. CSS viewport / scaling ──────────────────────────────────────────
Header "4. HTML / CSS — viewport meta"

$indexHtml = Get-Content "index.html" -Raw
if ($indexHtml -match 'viewport.*width=device-width') {
    Pass "index.html has responsive viewport meta tag"
} else {
    Warn "index.html missing viewport meta — WebView2 may not scale correctly"
}

# ─── 5. Font stack check ────────────────────────────────────────────────
Header "5. Font stack — system fonts for DPI clarity"

$appTsx = Get-Content "src\App.tsx" -Raw
if ($appTsx -match 'Segoe UI') {
    Pass "Uses Segoe UI (Windows system font — renders crisp at all DPIs)"
} else {
    Warn "Not using Segoe UI — may render blurry on Windows high-DPI"
}

# ─── 6. CSP check (no external font/image loading) ─────────────────────
Header "6. CSP — no external resources that could block offline DPI"

$csp = $conf.app.security.csp
if ($csp -match "default-src 'self'") {
    Pass "CSP restricts to 'self' — no external font loading delays"
} else {
    Warn "CSP allows external sources — verify fonts load offline"
}

# ─── 7. Tray icon ───────────────────────────────────────────────────────
Header "7. Tray icon configuration"

$tray = $conf.app.trayIcon
if ($tray.iconPath) {
    Pass "Tray icon configured: $($tray.iconPath)"
    if (Test-Path "src-tauri\$($tray.iconPath)") {
        Pass "Tray icon file exists on disk"
    } else {
        Fail "Tray icon file NOT found at src-tauri\$($tray.iconPath)"
    }
} else {
    Fail "No tray icon configured"
}

# ─── 8. Bundle icons for MSI (multi-resolution) ─────────────────────────
Header "8. Bundle icons — multi-resolution for DPI"

$icons = $conf.bundle.icon
if ($icons.Count -ge 3) {
    Pass "Bundle has $($icons.Count) icon sizes configured"
    foreach ($icon in $icons) {
        $iconPath = "src-tauri\$icon"
        if (Test-Path $iconPath) {
            Pass "  Icon exists: $icon"
        } else {
            Fail "  Icon MISSING: $icon — run 'pnpm tauri icon <source.png>'"
        }
    }
} else {
    Fail "Only $($icons.Count) icon sizes — need 32x32, 128x128, 128x128@2x, .ico"
}

# ─── 9. Runtime checks (if app is running) ──────────────────────────────
Header "9. Runtime checks (requires running app)"

$proc = Get-Process -Name "shoreline" -ErrorAction SilentlyContinue
if ($proc) {
    Pass "Application process found (PID: $($proc.Id))"

    # Check DPI awareness of the process
    try {
        Add-Type -TypeDefinition @"
using System;
using System.Runtime.InteropServices;
public class DpiHelper {
    [DllImport("user32.dll")]
    public static extern IntPtr GetDpiForWindow(IntPtr hwnd);
    [DllImport("shcore.dll")]
    public static extern int GetProcessDpiAwareness(IntPtr hprocess, out int value);
}
"@ -ErrorAction SilentlyContinue

        $awareness = 0
        [DpiHelper]::GetProcessDpiAwareness([IntPtr]::Zero, [ref]$awareness) | Out-Null
        switch ($awareness) {
            0 { Warn "Process DPI Awareness: Unaware (may render blurry at >100%)" }
            1 { Pass "Process DPI Awareness: System-aware" }
            2 { Pass "Process DPI Awareness: Per-monitor aware (best)" }
        }
    } catch {
        Warn "Could not query DPI awareness (requires elevated or matching arch)"
    }
} else {
    Warn "App not running — skipping runtime DPI checks. Start with 'pnpm tauri dev' first."
}

# ─── 10. Current display DPI ────────────────────────────────────────────
Header "10. Current display DPI"
try {
    Add-Type -AssemblyName System.Windows.Forms -ErrorAction SilentlyContinue
    $screen = [System.Windows.Forms.Screen]::PrimaryScreen
    $bounds = $screen.Bounds
    $workArea = $screen.WorkingArea

    # Detect DPI scale from registry
    $dpiReg = Get-ItemProperty "HKCU:\Control Panel\Desktop\WindowMetrics" -Name "AppliedDPI" -ErrorAction SilentlyContinue
    if ($dpiReg) {
        $dpi = $dpiReg.AppliedDPI
        $scale = [math]::Round($dpi / 96 * 100)
        Pass "Current display DPI: $dpi (${scale}%)"
        if ($scale -gt 100) {
            Write-Host "    NOTE: At ${scale}% scaling, the 1600x1000 main window" -ForegroundColor Gray
            Write-Host "    occupies $([math]::Round(1600 * $scale / 100))x$([math]::Round(1000 * $scale / 100)) physical pixels" -ForegroundColor Gray
        }
    }
    Pass "Primary monitor: $($bounds.Width)x$($bounds.Height)"
    Pass "Working area: $($workArea.Width)x$($workArea.Height)"
} catch {
    Warn "Could not detect display DPI information"
}

# ─── Summary ────────────────────────────────────────────────────────────
Write-Host "`n────────────────────────────────────────────────────" -ForegroundColor Cyan
Write-Host "DPI Verification Summary" -ForegroundColor Cyan
Write-Host "  Passed: $pass" -ForegroundColor Green
Write-Host "  Failed: $fail" -ForegroundColor $(if ($fail -gt 0) { "Red" } else { "Green" })
Write-Host "  Warnings: $warn" -ForegroundColor $(if ($warn -gt 0) { "Yellow" } else { "Green" })

if ($fail -gt 0) {
    Write-Host "`n  RESULT: FAIL — address the issues above before acceptance." -ForegroundColor Red
    exit 1
} elseif ($warn -gt 0) {
    Write-Host "`n  RESULT: CONDITIONAL PASS — review warnings for edge cases." -ForegroundColor Yellow
    exit 0
} else {
    Write-Host "`n  RESULT: PASS" -ForegroundColor Green
    exit 0
}

Pop-Location
