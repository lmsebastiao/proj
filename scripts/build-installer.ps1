<#
.SYNOPSIS
    Builds proj in release mode and packages it as dist\proj-setup-<version>.exe.

.DESCRIPTION
    Uses makensis from PATH or a standard NSIS install. If there is none, the pinned
    portable NSIS release is downloaded once into %LOCALAPPDATA%\proj-build (checked
    against its SHA-256); nothing is installed system-wide.

.EXAMPLE
    .\scripts\build-installer.ps1
    .\scripts\build-installer.ps1 -SkipBuild   # package the last release build
#>
param([switch]$SkipBuild)

$ErrorActionPreference = 'Stop'
$root = Split-Path -Parent $PSScriptRoot
Set-Location $root

$nsisVersion = '3.12'
$nsisSha256 = '56581f90db321581c5381193d796fffcf2d24b2f8fed2160a6c6a3baa67f2c4f'

function Find-Makensis {
    $onPath = Get-Command makensis -ErrorAction SilentlyContinue
    if ($onPath) { return $onPath.Source }

    $cache = Join-Path $env:LOCALAPPDATA 'proj-build'
    $candidates = @(
        "${env:ProgramFiles(x86)}\NSIS\makensis.exe",
        "$env:ProgramFiles\NSIS\makensis.exe",
        "$cache\nsis-$nsisVersion\makensis.exe"
    )
    foreach ($exe in $candidates) {
        if (Test-Path $exe) { return $exe }
    }

    Write-Host "NSIS not found; downloading portable NSIS $nsisVersion to $cache"
    New-Item -ItemType Directory -Force $cache | Out-Null
    $zip = Join-Path $cache "nsis-$nsisVersion.zip"
    $url = "https://downloads.sourceforge.net/project/nsis/NSIS%203/$nsisVersion/nsis-$nsisVersion.zip"
    # curl.exe ships with Windows 10+; SourceForge serves the file directly to it.
    & curl.exe --fail --silent --show-error --location --user-agent 'curl/8' --output $zip $url
    if ($LASTEXITCODE -ne 0) { throw "Downloading $url failed" }

    $hash = (Get-FileHash $zip -Algorithm SHA256).Hash
    if ($hash -ne $nsisSha256) {
        Remove-Item $zip
        throw "NSIS download has an unexpected SHA-256 ($hash); refusing to use it"
    }
    Expand-Archive $zip -DestinationPath $cache -Force
    Remove-Item $zip
    return "$cache\nsis-$nsisVersion\makensis.exe"
}

$version = (Select-String -Path Cargo.toml -Pattern '^version\s*=\s*"([^"]+)"' |
    Select-Object -First 1).Matches.Groups[1].Value
$targetDir = (cargo metadata --format-version 1 --no-deps | ConvertFrom-Json).target_directory
$exe = Join-Path $targetDir 'release\proj.exe'

if (-not $SkipBuild) {
    # A running copy of the build output locks the file and fails the link.
    Get-Process proj -ErrorAction SilentlyContinue |
        Where-Object { $_.Path -eq $exe } |
        Stop-Process -Force
    cargo build --release
    if ($LASTEXITCODE -ne 0) { throw 'cargo build failed' }
}
if (-not (Test-Path $exe)) { throw "$exe not found; build first" }

$makensis = Find-Makensis
New-Item -ItemType Directory -Force dist | Out-Null
$out = Join-Path $root "dist\proj-setup-$version.exe"

& $makensis /V2 "/DVERSION=$version" "/DEXE=$exe" "/DOUTFILE=$out" installer\proj.nsi
if ($LASTEXITCODE -ne 0) { throw 'makensis failed' }

$size = [math]::Round((Get-Item $out).Length / 1MB, 1)
Write-Host "Built $out ($size MB)"
