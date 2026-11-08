#Requires -Version 5.1
<#
.SYNOPSIS
  Builds and installs Tandem on a Windows PC from this source folder.

.DESCRIPTION
  1. Installs missing build tools with winget: Visual Studio Build Tools (C++), Rust, Node.js,
     pnpm, CMake and LLVM. Windows asks for admin approval (UAC) for some of these.
  2. Builds the Tandem installer (NSIS .exe) with `pnpm tauri build`.
  3. Runs the installer.

  Run it with install-windows.bat in the repo root, or:
    powershell -ExecutionPolicy Bypass -File scripts\windows\setup.ps1

.PARAMETER SkipPrereqs
  Don't check or install build tools.
.PARAMETER NoInstall
  Build the installer but don't run it.
#>
param(
    [switch]$SkipPrereqs,
    [switch]$NoInstall
)

$ErrorActionPreference = 'Stop'
$Root = (Resolve-Path (Join-Path $PSScriptRoot '..\..')).Path

function Step([string]$Message) { Write-Host "`n==> $Message" -ForegroundColor Cyan }
function Note([string]$Message) { Write-Host "    $Message" -ForegroundColor DarkGray }
function Have([string]$Command) { [bool](Get-Command $Command -ErrorAction SilentlyContinue) }

function Update-SessionPath {
    $machine = [Environment]::GetEnvironmentVariable('Path', 'Machine')
    $user = [Environment]::GetEnvironmentVariable('Path', 'User')
    $env:Path = "$machine;$user;$env:USERPROFILE\.cargo\bin"
}

function Install-WithWinget([string]$Id, [string[]]$Extra = @()) {
    Note "winget install $Id"
    & winget install --id $Id --exact --silent --accept-package-agreements --accept-source-agreements @Extra
    # 0 = installed; -1978335189 / -1978335135 = already installed / no newer version.
    if ($LASTEXITCODE -ne 0 -and $LASTEXITCODE -ne -1978335189 -and $LASTEXITCODE -ne -1978335135) {
        throw "winget couldn't install $Id (exit code $LASTEXITCODE)."
    }
    Update-SessionPath
}

function Test-VcTools {
    $vswhere = Join-Path ${env:ProgramFiles(x86)} 'Microsoft Visual Studio\Installer\vswhere.exe'
    if (-not (Test-Path $vswhere)) { return $false }
    $path = & $vswhere -latest -products * -requires Microsoft.VisualStudio.Component.VC.Tools.x86.x64 -property installationPath
    return [bool]$path
}

Write-Host "Tandem setup for Windows" -ForegroundColor Green
Note "Source folder: $Root"

if ($Root.Length -gt 40) {
    Write-Warning ("This folder's path is long ($($Root.Length) characters). The speech engine's C++ build " +
        "can hit Windows' path-length limit. If the build fails, move the folder to something short like C:\tandem.")
}

if (-not $SkipPrereqs) {
    Step "Checking build tools"
    if (-not (Have 'winget')) {
        throw "winget isn't available. Install 'App Installer' from the Microsoft Store, then run this again."
    }

    if (Test-VcTools) { Note "Visual Studio C++ build tools: found" }
    else {
        Note "Installing Visual Studio 2022 Build Tools with the C++ workload (large download, several minutes)..."
        Install-WithWinget 'Microsoft.VisualStudio.2022.BuildTools' @(
            '--override', '--wait --passive --add Microsoft.VisualStudio.Workload.VCTools --includeRecommended')
        if (-not (Test-VcTools)) { throw "The C++ build tools didn't install. Install 'Desktop development with C++' from Visual Studio Installer, then rerun." }
    }

    if (Have 'cargo') { Note "Rust: $(& rustc --version)" }
    else {
        Install-WithWinget 'Rustlang.Rustup'
        & rustup default stable-msvc
    }

    $nodeOk = $false
    if (Have 'node') {
        $major = [int]((& node --version).TrimStart('v').Split('.')[0])
        $nodeOk = $major -ge 22
    }
    if ($nodeOk) { Note "Node.js: $(& node --version)" } else { Install-WithWinget 'OpenJS.NodeJS.LTS' }

    if (Have 'pnpm') { Note "pnpm: $(& pnpm --version)" }
    else {
        & npm install --global pnpm@11
        if ($LASTEXITCODE -ne 0) { throw "Couldn't install pnpm." }
        Update-SessionPath
    }

    if (Have 'cmake') { Note "CMake: found" } else { Install-WithWinget 'Kitware.CMake' }

    # whisper.cpp's Rust bindings are generated with bindgen, which needs libclang.
    if (Test-Path (Join-Path $env:ProgramFiles 'LLVM\bin\libclang.dll')) { Note "LLVM: found" }
    else { Install-WithWinget 'LLVM.LLVM' }
}

# The CMake installer doesn't always add itself to PATH.
$cmakeBin = Join-Path $env:ProgramFiles 'CMake\bin'
if (-not (Have 'cmake') -and (Test-Path $cmakeBin)) { $env:Path = "$env:Path;$cmakeBin" }

foreach ($tool in 'cargo', 'node', 'pnpm', 'cmake') {
    if (-not (Have $tool)) {
        throw "'$tool' still isn't on PATH. Close this window, open a new one, and run the script again."
    }
}

Step "Installing JavaScript packages"
Set-Location $Root
& pnpm install --frozen-lockfile
if ($LASTEXITCODE -ne 0) { throw "pnpm install failed." }

Step "Building Tandem (the first build takes 10-20 minutes)"
# Point bindgen at LLVM's libclang (whisper.cpp bindings). Its bundled pre-generated bindings
# assume a 64-bit C `long` and don't compile on Windows.
$llvmBin = Join-Path $env:ProgramFiles 'LLVM\bin'
if (-not $env:LIBCLANG_PATH -and (Test-Path (Join-Path $llvmBin 'libclang.dll'))) { $env:LIBCLANG_PATH = $llvmBin }
& pnpm tauri build --bundles nsis
if ($LASTEXITCODE -ne 0) { throw "The build failed. Scroll up for the first 'error' line." }

$installer = Get-ChildItem (Join-Path $Root 'target\release\bundle\nsis') -Filter '*-setup.exe' |
    Sort-Object LastWriteTime -Descending | Select-Object -First 1
if (-not $installer) { throw "Build finished but no installer was found in target\release\bundle\nsis." }

Step "Installer ready"
Note $installer.FullName

if ($NoInstall) {
    Note "Skipping install (-NoInstall). Copy that .exe to any Windows PC to install Tandem there."
    exit 0
}

Step "Running the installer"
Note "Windows may show 'Windows protected your PC' because this build isn't code-signed."
Note "Click 'More info', then 'Run anyway'."
Start-Process -FilePath $installer.FullName -Wait
Write-Host "`nDone. Start Tandem from the Start menu." -ForegroundColor Green
