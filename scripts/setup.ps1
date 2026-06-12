# One-time machine setup for Telenn on Windows: Rust toolchain and npm packages.
# Tauri also requires the MSVC C++ build tools and the WebView2 runtime; this
# script checks for them and points to the installer rather than installing them
# silently. Safe to re-run. On Linux/macOS, use setup.sh instead.
$ErrorActionPreference = 'Stop'
$root = Split-Path -Parent (Split-Path -Parent $MyInvocation.MyCommand.Path)

# Rust: install rustup if cargo is missing. The exact version and components come
# from rust-toolchain.toml, which rustup honours automatically.
$cargoExe = Join-Path $env:USERPROFILE '.cargo\bin\cargo.exe'
if (-not (Get-Command cargo -ErrorAction SilentlyContinue) -and -not (Test-Path $cargoExe)) {
    Write-Host 'Installing rustup...'
    $init = Join-Path $env:TEMP 'rustup-init.exe'
    Invoke-WebRequest -Uri 'https://win.rustup.rs/x86_64' -OutFile $init
    & $init -y --default-toolchain stable --profile minimal
}

# Trigger installation of the pinned toolchain and its components.
$rustup = Join-Path $env:USERPROFILE '.cargo\bin\rustup.exe'
if (Get-Command rustup -ErrorAction SilentlyContinue) { $rustup = 'rustup' }
Push-Location $root
& $rustup show | Out-Null
Pop-Location

# WebView2 runtime check (warn only).
$webview2 = Get-ItemProperty -Path `
    'HKLM:\SOFTWARE\WOW6432Node\Microsoft\EdgeUpdate\Clients\{F3017226-FE2A-4295-8BDF-00C3A9A7E4C5}' `
    -ErrorAction SilentlyContinue
if (-not $webview2) {
    Write-Warning 'WebView2 runtime not detected. Install it from https://developer.microsoft.com/microsoft-edge/webview2/'
}

# MSVC C++ build tools check (warn only).
if (-not (Get-Command cl.exe -ErrorAction SilentlyContinue)) {
    Write-Warning 'MSVC C++ build tools not found on PATH. Install "Desktop development with C++" via Visual Studio / Build Tools 2022.'
}

Write-Host 'Installing npm packages...'
Push-Location $root
npm install
Pop-Location

Write-Host 'Setup complete. Run ./check.sh to verify.'
