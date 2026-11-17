#Requires -Version 5.1
<#
.SYNOPSIS
  Provision libmpv binaries for go-media-mpv (Windows x86_64).

.DESCRIPTION
  Downloads the official mpv CI build (LGPL variant — safe to link from
  this app), extracts libmpv-2.dll + headers into
  native/mpv/x86_64-pc-windows-msvc/, and generates the MSVC import
  library mpv.lib from the DLL exports using dumpbin + lib.exe from the
  installed VS Build Tools.

  Idempotent: skips the download when the requested version is already
  provisioned. Re-run with -Version to upgrade/downgrade.

.EXAMPLE
  .\scripts\setup-mpv.ps1                 # latest stable release
  .\scripts\setup-mpv.ps1 -Version v0.41.0
#>
param(
    [string]$Version = "latest"
)

$ErrorActionPreference = 'Stop'
$RepoRoot   = Split-Path -Parent $PSScriptRoot
$TargetDir  = Join-Path $RepoRoot 'native\mpv\x86_64-pc-windows-msvc'
$BundleDir  = Join-Path $RepoRoot 'apps\app\src-tauri\bundle-resources\windows'
New-Item -ItemType Directory -Force -Path $BundleDir | Out-Null
$ApiRepo    = 'mpv-player/mpv'

function Resolve-MpvVersion {
    param([string]$Want)
    if ($Want -ne 'latest') { return $Want }
    $rel = Invoke-RestMethod "https://api.github.com/repos/$ApiRepo/releases/latest"
    return $rel.tag_name
}

function Find-MsvcTool([string]$Name) {
    $vswhere = "${env:ProgramFiles(x86)}\Microsoft Visual Studio\Installer\vswhere.exe"
    if (-not (Test-Path $vswhere)) { return $null }
    $vsPath = & $vswhere -latest -products '*' -property installationPath
    if (-not $vsPath) { return $null }
    $found = Get-ChildItem (Join-Path $vsPath 'VC\Tools\MSVC') -Directory |
        Sort-Object Name -Descending |
        ForEach-Object { Join-Path $_.FullName "bin\Hostx64\x64\$Name" } |
        Where-Object { Test-Path $_ } |
        Select-Object -First 1
    return $found
}

$tag = Resolve-MpvVersion $Version
$asset = "libmpv-$tag-x86_64-w64-mingw32-lgpl.zip"
$marker = Join-Path $TargetDir '.version'

if ((Test-Path $marker) -and ((Get-Content $marker -Raw).Trim() -eq $tag) -and
    (Test-Path (Join-Path $TargetDir 'mpv.lib'))) {
    Copy-Item (Join-Path $TargetDir 'libmpv-2.dll') (Join-Path $BundleDir 'libmpv-2.dll') -Force
    Write-Host "libmpv $tag already provisioned at $TargetDir — staged for bundling."
    exit 0
}

Write-Host "Provisioning libmpv $tag ..."
New-Item -ItemType Directory -Force -Path $TargetDir | Out-Null
$tmp = Join-Path $env:TEMP "go-player-mpv-$tag"
New-Item -ItemType Directory -Force -Path $tmp | Out-Null
$zip = Join-Path $tmp $asset

if (-not (Test-Path $zip)) {
    $url = "https://github.com/$ApiRepo/releases/download/$tag/$asset"
    Write-Host "  downloading $url"
    Invoke-WebRequest -Uri $url -OutFile $zip -UseBasicParsing
}

# The release zips contain a flat layout (libmpv-2.dll, include/, ...).
Write-Host "  extracting..."
Expand-Archive -Path $zip -DestinationPath $tmp -Force
$inner = Get-ChildItem $tmp -Directory | Where-Object Name -ne '' | Select-Object -First 1
$src = if ($inner -and (Test-Path (Join-Path $inner.FullName 'libmpv-2.dll'))) {
    $inner.FullName
} else { $tmp }

Copy-Item (Join-Path $src '*') $TargetDir -Recurse -Force
Set-Content -Path $marker -Value $tag

# Stage runtime DLL for Tauri bundling (tauri.windows.conf.json)
Copy-Item (Join-Path $TargetDir 'libmpv-2.dll') (Join-Path $BundleDir 'libmpv-2.dll') -Force
Write-Host "  staged libmpv-2.dll -> $BundleDir"

# --- Generate mpv.lib (MSVC import library) if missing ---------------------
$libPath = Join-Path $TargetDir 'mpv.lib'
$dllPath = Join-Path $TargetDir 'libmpv-2.dll'
if (-not (Test-Path $libPath)) {
    $dumpbin = Find-MsvcTool 'dumpbin.exe'
    $lib     = Find-MsvcTool 'lib.exe'
    if (-not $dumpbin -or -not $lib) {
        throw "dumpbin.exe/lib.exe not found — install VS Build Tools C++ workload first."
    }
    Write-Host "  generating mpv.lib via dumpbin + lib.exe..."
    $exportsTxt = Join-Path $tmp 'exports.txt'
    & $dumpbin /exports $dllPath | Out-File $exportsTxt -Encoding ascii
    $defPath = Join-Path $tmp 'mpv.def'
    $names = Select-String -Path $exportsTxt -Pattern '^\s+\d+\s+[0-9A-F]+\s+[0-9A-F]+\s+(\S+)' |
        ForEach-Object { $_.Matches[0].Groups[1].Value }
    if (-not $names) { throw "no exports found in $dllPath" }
    @('LIBRARY libmpv-2.dll', 'EXPORTS') + ($names | ForEach-Object { "    $_" }) |
        Set-Content $defPath -Encoding ascii
    & $lib /def:$defPath /machine:x64 /out:$libPath | Out-Null
    if (-not (Test-Path $libPath)) { throw "lib.exe failed to produce mpv.lib" }
    Remove-Item (Join-Path $TargetDir 'mpv.exp') -ErrorAction SilentlyContinue
}

Remove-Item $tmp -Recurse -Force -ErrorAction SilentlyContinue
Write-Host "Done. libmpv $tag provisioned at $TargetDir"
Write-Host "Next: cargo build will link against mpv.lib automatically (build.rs)."
