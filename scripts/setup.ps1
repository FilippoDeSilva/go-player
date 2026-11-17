#Requires -Version 5.1
<#
.SYNOPSIS
  Idempotent development environment setup for Go Player (Windows).

.DESCRIPTION
  Checks for and installs the prerequisites needed to build and run the
  app: Rust (MSVC), Node.js, pnpm, Visual Studio C++ Build Tools, then
  installs workspace dependencies.

  Safe to re-run; each step is skipped when the tool is already present.
  Some steps (winget installs) require elevation — run this script from an
  elevated PowerShell if a step fails with a permission error.

.EXAMPLE
  powershell -ExecutionPolicy Bypass -File scripts\setup.ps1
#>

$ErrorActionPreference = 'Stop'
$RepoRoot = Split-Path -Parent $PSScriptRoot

function Test-Command([string]$Name) {
    return [bool](Get-Command $Name -ErrorAction SilentlyContinue)
}

function Install-Winget([string]$Id, [string]$OverrideArgs) {
    Write-Host "  installing $Id ..."
    if ($OverrideArgs) {
        winget install --id $Id --override $OverrideArgs `
            --accept-source-agreements --accept-package-agreements --silent
    } else {
        winget install --id $Id `
            --accept-source-agreements --accept-package-agreements --silent
    }
    if ($LASTEXITCODE -ne 0) {
        Write-Warning "winget install of $Id exited with $LASTEXITCODE — continuing."
    }
}

Write-Host "== Go Player setup =="

# --- Rust (MSVC toolchain) -------------------------------------------------
if (-not (Test-Command 'rustc')) {
    Write-Host "[rust] not found — installing rustup..."
    Install-Winget 'Rustlang.Rustup'
    $env:Path = [System.Environment]::GetEnvironmentVariable('Path', 'User') + ';' +
                [System.Environment]::GetEnvironmentVariable('Path', 'Machine')
}
if (Test-Command 'rustup') {
    rustup default stable-msvc | Out-Null
    rustup target add x86_64-pc-windows-msvc | Out-Null
    Write-Host "[rust] $(rustc --version)"
} else {
    Write-Warning "rustup still not on PATH — open a new terminal and re-run this script."
}

# --- Node.js ---------------------------------------------------------------
if (-not (Test-Command 'node')) {
    Write-Host "[node] not found — installing Node.js LTS..."
    Install-Winget 'OpenJS.NodeJS.LTS'
    $env:Path = [System.Environment]::GetEnvironmentVariable('Path', 'User') + ';' +
                [System.Environment]::GetEnvironmentVariable('Path', 'Machine')
}
if (Test-Command 'node') {
    Write-Host "[node] $(node --version)"
}

# --- pnpm ------------------------------------------------------------------
if (-not (Test-Command 'pnpm')) {
    if (Test-Command 'corepack') {
        corepack enable | Out-Null
        corepack prepare pnpm@latest --activate | Out-Null
    } else {
        Write-Host "[pnpm] installing via npm..."
        npm install -g pnpm | Out-Null
    }
}
if (Test-Command 'pnpm') {
    Write-Host "[pnpm] $(pnpm --version)"
}

# --- Visual Studio C++ Build Tools (MSVC linker + Windows SDK) -------------
$vswhere = "${env:ProgramFiles(x86)}\Microsoft Visual Studio\Installer\vswhere.exe"
$hasVcTools = $false
if (Test-Path $vswhere) {
    $instPath = & $vswhere -latest -products '*' `
        -requires Microsoft.VisualStudio.Component.VC.Tools.x86.x64 `
        -property installationPath 2>$null
    $hasVcTools = [bool]$instPath
}
if (-not $hasVcTools) {
    Write-Host "[msvc] C++ workload not found — installing VS 2022 Build Tools..."
    Write-Host "       (large download, several GB — please be patient)"
    Install-Winget 'Microsoft.VisualStudio.2022.BuildTools' `
        '--wait --passive --norestart --add Microsoft.VisualStudio.Workload.VCTools --includeRecommended'
} else {
    Write-Host "[msvc] C++ build tools present"
}

# --- Workspace dependencies -------------------------------------------------
Push-Location $RepoRoot
try {
    Write-Host "[deps] pnpm install..."
    pnpm install
} finally {
    Pop-Location
}

Write-Host ""
Write-Host "Setup complete. If Rust was installed during this run, open a NEW"
Write-Host "terminal so PATH updates take effect, then run:"
Write-Host "    pnpm dev        # tauri dev (desktop app)"
Write-Host "    pnpm build      # production frontend build"
