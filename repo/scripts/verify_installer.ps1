<#
.SYNOPSIS
  MSI Installer Verification for Shoreline Property Operations Console.

.DESCRIPTION
  Validates the MSI installer package produced by `pnpm tauri build`.
  Checks: MSI file presence, Wix metadata, file size, install/uninstall
  simulation, registry entries, WebView2 dependency, and icon embedding.

.PARAMETER MsiPath
  Path to the .msi file. If omitted, searches the default Tauri output
  directory (src-tauri/target/release/bundle/msi/).

.PARAMETER SkipInstall
  Skip the actual install/uninstall test (useful for CI without admin).

.EXAMPLE
  .\scripts\verify_installer.ps1
  .\scripts\verify_installer.ps1 -MsiPath "C:\builds\shoreline.msi"
  .\scripts\verify_installer.ps1 -SkipInstall
#>

param(
    [string]$MsiPath,
    [switch]$SkipInstall
)

$ErrorActionPreference = "Stop"
Push-Location (Split-Path $PSScriptRoot -Parent)
$pass = 0; $fail = 0; $warn = 0

function Pass($msg) { Write-Host "  [PASS] $msg" -ForegroundColor Green; $script:pass++ }
function Fail($msg) { Write-Host "  [FAIL] $msg" -ForegroundColor Red;   $script:fail++ }
function Warn($msg) { Write-Host "  [WARN] $msg" -ForegroundColor Yellow; $script:warn++ }
function Header($msg) { Write-Host "`n== $msg ==" -ForegroundColor Cyan }

# ─── 1. Locate MSI ──────────────────────────────────────────────────────
Header "1. MSI file location"

if (-not $MsiPath) {
    $msiDir = "src-tauri\target\release\bundle\msi"
    $found = Get-ChildItem -Path $msiDir -Filter "*.msi" -ErrorAction SilentlyContinue |
             Sort-Object LastWriteTime -Descending | Select-Object -First 1
    if ($found) {
        $MsiPath = $found.FullName
        Pass "Found MSI: $MsiPath"
    } else {
        Fail "No MSI found in $msiDir — run 'pnpm tauri build' first"
        Write-Host "`nAborting — no MSI to verify." -ForegroundColor Red
        Pop-Location; exit 1
    }
} else {
    if (Test-Path $MsiPath) {
        Pass "Using specified MSI: $MsiPath"
    } else {
        Fail "Specified MSI not found: $MsiPath"
        Pop-Location; exit 1
    }
}

# ─── 2. File size sanity ────────────────────────────────────────────────
Header "2. MSI file size"

$msiFile = Get-Item $MsiPath
$sizeMB = [math]::Round($msiFile.Length / 1MB, 2)
if ($sizeMB -ge 3 -and $sizeMB -le 200) {
    Pass "MSI size: ${sizeMB} MB (within expected range)"
} elseif ($sizeMB -lt 3) {
    Fail "MSI size: ${sizeMB} MB — suspiciously small, may be incomplete"
} else {
    Warn "MSI size: ${sizeMB} MB — larger than expected, check embedded resources"
}

# ─── 3. MSI metadata via Windows Installer COM ──────────────────────────
Header "3. MSI metadata"

try {
    $installer = New-Object -ComObject WindowsInstaller.Installer
    $db = $installer.OpenDatabase($MsiPath, 0)  # 0 = read-only

    # Product name
    $view = $db.OpenView("SELECT `Value` FROM `Property` WHERE `Property` = 'ProductName'")
    $view.Execute()
    $record = $view.Fetch()
    if ($record) {
        $productName = $record.StringData(1)
        if ($productName -like "*Shoreline*") {
            Pass "Product name: $productName"
        } else {
            Fail "Product name '$productName' does not contain 'Shoreline'"
        }
    } else {
        Fail "ProductName property not found in MSI"
    }

    # Version
    $view2 = $db.OpenView("SELECT `Value` FROM `Property` WHERE `Property` = 'ProductVersion'")
    $view2.Execute()
    $record2 = $view2.Fetch()
    if ($record2) {
        $version = $record2.StringData(1)
        Pass "Product version: $version"
        # Cross-check with tauri.conf.json
        $conf = Get-Content "src-tauri\tauri.conf.json" -Raw | ConvertFrom-Json
        if ($version -eq $conf.version) {
            Pass "Version matches tauri.conf.json ($($conf.version))"
        } else {
            Warn "Version mismatch: MSI=$version, config=$($conf.version)"
        }
    }

    # Manufacturer
    $view3 = $db.OpenView("SELECT `Value` FROM `Property` WHERE `Property` = 'Manufacturer'")
    $view3.Execute()
    $record3 = $view3.Fetch()
    if ($record3) {
        Pass "Manufacturer: $($record3.StringData(1))"
    } else {
        Warn "No Manufacturer property — consider adding for Add/Remove Programs"
    }

    # Language
    $view4 = $db.OpenView("SELECT `Value` FROM `Property` WHERE `Property` = 'ProductLanguage'")
    $view4.Execute()
    $record4 = $view4.Fetch()
    if ($record4) {
        $langId = $record4.StringData(1)
        if ($langId -eq "1033") {
            Pass "Language: en-US (1033)"
        } else {
            Warn "Language ID is $langId (expected 1033 / en-US)"
        }
    }

    [System.Runtime.Interopservices.Marshal]::ReleaseComObject($installer) | Out-Null
} catch {
    Warn "Could not read MSI metadata via COM: $_"
    Warn "  (This is expected in non-interactive or headless environments)"
}

# ─── 4. Tauri build configuration ───────────────────────────────────────
Header "4. Tauri bundle configuration"

$conf = Get-Content "src-tauri\tauri.conf.json" -Raw | ConvertFrom-Json

if ($conf.bundle.active -eq $true) {
    Pass "Bundle is active"
} else {
    Fail "Bundle is NOT active in tauri.conf.json"
}

if ($conf.bundle.targets -contains "msi") {
    Pass "MSI is a build target"
} else {
    Fail "MSI not listed in bundle targets: $($conf.bundle.targets -join ', ')"
}

if ($conf.bundle.category) {
    Pass "Category: $($conf.bundle.category)"
} else {
    Warn "No category set — may show as 'Unknown' in Windows apps list"
}

# Check WiX language config
if ($conf.bundle.windows.wix.language -eq "en-US") {
    Pass "WiX language: en-US"
} else {
    Warn "WiX language: $($conf.bundle.windows.wix.language)"
}

# ─── 5. Icon files ──────────────────────────────────────────────────────
Header "5. Icon files for installer"

$requiredIcons = @("icons/icon.ico", "icons/32x32.png", "icons/128x128.png", "icons/128x128@2x.png")
foreach ($icon in $requiredIcons) {
    $iconPath = "src-tauri\$icon"
    if (Test-Path $iconPath) {
        $iconSize = [math]::Round((Get-Item $iconPath).Length / 1KB, 1)
        Pass "Icon present: $icon (${iconSize} KB)"
    } else {
        Fail "Icon MISSING: $icon — run 'pnpm tauri icon <source.png>'"
    }
}

# ─── 6. WebView2 dependency ─────────────────────────────────────────────
Header "6. WebView2 Runtime"

$wv2Key = "HKLM:\SOFTWARE\WOW6432Node\Microsoft\EdgeUpdate\Clients\{F3017226-FE2A-4295-8BDF-00C3A9A7E4C5}"
$wv2Key2 = "HKLM:\SOFTWARE\Microsoft\EdgeUpdate\Clients\{F3017226-FE2A-4295-8BDF-00C3A9A7E4C5}"

$wv2Installed = $false
foreach ($key in @($wv2Key, $wv2Key2)) {
    $reg = Get-ItemProperty $key -ErrorAction SilentlyContinue
    if ($reg -and $reg.pv) {
        Pass "WebView2 Runtime installed: version $($reg.pv)"
        $wv2Installed = $true
        break
    }
}
if (-not $wv2Installed) {
    Fail "WebView2 Runtime NOT found — required for Tauri 2.x apps"
    Write-Host "    Install from: https://developer.microsoft.com/en-us/microsoft-edge/webview2/" -ForegroundColor Gray
}

# ─── 7. Install/Uninstall test ──────────────────────────────────────────
Header "7. Install / Uninstall test"

if ($SkipInstall) {
    Warn "Skipped (-SkipInstall flag). Run without flag for full test."
} else {
    $isAdmin = ([Security.Principal.WindowsPrincipal] [Security.Principal.WindowsIdentity]::GetCurrent()).IsInRole([Security.Principal.WindowsBuiltInRole]::Administrator)
    if (-not $isAdmin) {
        Warn "Not running as Administrator — install test requires elevation"
        Warn "Re-run as admin or use -SkipInstall for config-only checks"
    } else {
        Write-Host "    Installing MSI (silent)..." -ForegroundColor Gray
        try {
            $installLog = "$env:TEMP\shoreline_install.log"
            $proc = Start-Process "msiexec.exe" -ArgumentList "/i `"$MsiPath`" /qn /l*v `"$installLog`"" -Wait -PassThru
            if ($proc.ExitCode -eq 0) {
                Pass "MSI installed successfully (exit code 0)"

                # Check for installed executable
                $installDir = "$env:ProgramFiles\Shoreline Property Operations Console"
                if (Test-Path $installDir) {
                    Pass "Install directory created: $installDir"
                    $exe = Get-ChildItem $installDir -Filter "*.exe" -Recurse | Select-Object -First 1
                    if ($exe) {
                        Pass "Executable found: $($exe.Name)"
                    } else {
                        Fail "No .exe found in install directory"
                    }
                } else {
                    Warn "Install directory not at expected path"
                }

                # Check Add/Remove Programs registry
                $uninstallKeys = Get-ChildItem "HKLM:\SOFTWARE\Microsoft\Windows\CurrentVersion\Uninstall" |
                    Where-Object { (Get-ItemProperty $_.PsPath).DisplayName -like "*Shoreline*" }
                if ($uninstallKeys) {
                    Pass "Add/Remove Programs entry found"
                } else {
                    Warn "No Add/Remove Programs entry found"
                }

                # Uninstall
                Write-Host "    Uninstalling MSI (silent)..." -ForegroundColor Gray
                $uninstallProc = Start-Process "msiexec.exe" -ArgumentList "/x `"$MsiPath`" /qn" -Wait -PassThru
                if ($uninstallProc.ExitCode -eq 0) {
                    Pass "MSI uninstalled successfully (exit code 0)"
                    if (-not (Test-Path $installDir)) {
                        Pass "Install directory cleaned up after uninstall"
                    } else {
                        Warn "Install directory remains after uninstall (may have user data)"
                    }
                } else {
                    Fail "Uninstall failed (exit code: $($uninstallProc.ExitCode))"
                }
            } else {
                Fail "MSI install failed (exit code: $($proc.ExitCode))"
                Write-Host "    Check log: $installLog" -ForegroundColor Gray
            }
        } catch {
            Fail "Install test threw exception: $_"
        }
    }
}

# ─── 8. AppData directory convention ────────────────────────────────────
Header "8. AppData directory convention"

$appDataPath = "$env:APPDATA\Shoreline"
$altPath = "$env:APPDATA\com.shoreline.propops"
if (Test-Path $appDataPath) {
    Pass "AppData directory exists: $appDataPath"
} elseif (Test-Path $altPath) {
    Pass "AppData directory exists: $altPath"
} else {
    Warn "No AppData directory found (created on first launch)"
    Write-Host "    Expected at: $appDataPath or $altPath" -ForegroundColor Gray
}

# ─── Summary ────────────────────────────────────────────────────────────
Write-Host "`n────────────────────────────────────────────────────" -ForegroundColor Cyan
Write-Host "Installer Verification Summary" -ForegroundColor Cyan
Write-Host "  Passed:   $pass" -ForegroundColor Green
Write-Host "  Failed:   $fail" -ForegroundColor $(if ($fail -gt 0) { "Red" } else { "Green" })
Write-Host "  Warnings: $warn" -ForegroundColor $(if ($warn -gt 0) { "Yellow" } else { "Green" })

if ($fail -gt 0) {
    Write-Host "`n  RESULT: FAIL — address the issues above." -ForegroundColor Red
    exit 1
} elseif ($warn -gt 0) {
    Write-Host "`n  RESULT: CONDITIONAL PASS — review warnings." -ForegroundColor Yellow
    exit 0
} else {
    Write-Host "`n  RESULT: PASS" -ForegroundColor Green
    exit 0
}

Pop-Location
