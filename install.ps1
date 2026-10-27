# Kiwi One-Line Universal Installer for Windows (PowerShell)
# https://github.com/Tribal-Chief-001/Kiwi
# Run in PowerShell: irm https://raw.githubusercontent.com/Tribal-Chief-001/Kiwi/main/install.ps1 | iex

$ErrorActionPreference = "Stop"

Write-Host "🥝 Installing Kiwi: Zero-Footprint CLI Network Intelligence Engine for Windows..." -ForegroundColor Cyan

$Repo = "Tribal-Chief-001/Kiwi"
$ZipName = "kiwi-windows-x86_64.zip"
$DownloadUrl = "https://github.com/$Repo/releases/latest/download/$ZipName"

$InstallDir = "$env:USERPROFILE\.kiwi\bin"
if (!(Test-Path -Path $InstallDir)) {
    New-Item -ItemType Directory -Path $InstallDir -Force | Out-Null
}

$TempZip = "$env:TEMP\$ZipName"
$TempExtract = "$env:TEMP\kiwi-extract"

Write-Host "⬇️  Downloading Kiwi from GitHub Releases..." -ForegroundColor Yellow
try {
    [Net.ServicePointManager]::SecurityProtocol = [Net.SecurityProtocolType]::Tls12
    Invoke-WebRequest -Uri $DownloadUrl -OutFile $TempZip -UseBasicParsing
} catch {
    Write-Host "⚠️  Failed to download release zip directly from GitHub." -ForegroundColor Red
    if (Get-Command cargo -ErrorAction SilentlyContinue) {
        Write-Host "📦 Cargo detected! Compiling and installing from source..." -ForegroundColor Green
        cargo install --git "https://github.com/$Repo.git" --bin kiwi
        Write-Host "🎉 Kiwi installed! Run 'kiwi' to test." -ForegroundColor Green
        exit 0
    }
    Write-Host "❌ Could not complete installation. Please check https://github.com/$Repo/releases" -ForegroundColor Red
    exit 1
}

Write-Host "📦 Extracting $ZipName..." -ForegroundColor Yellow
if (Test-Path -Path $TempExtract) {
    Remove-Item -Path $TempExtract -Recurse -Force
}
Expand-Archive -Path $TempZip -DestinationPath $TempExtract -Force

$ExePath = Get-ChildItem -Path $TempExtract -Filter "kiwi.exe" -Recurse | Select-Object -First 1 -ExpandProperty FullName
if (!$ExePath) {
    Write-Host "❌ Could not find kiwi.exe in extracted archive." -ForegroundColor Red
    exit 1
}

Copy-Item -Path $ExePath -Destination "$InstallDir\kiwi.exe" -Force
Remove-Item -Path $TempZip -Force -ErrorAction SilentlyContinue
Remove-Item -Path $TempExtract -Recurse -Force -ErrorAction SilentlyContinue

# Ensure ~/.kiwi/bin is permanently in User PATH
$UserPath = [Environment]::GetEnvironmentVariable("PATH", "User")
if ($UserPath -notlike "*$InstallDir*") {
    $NewUserPath = "$UserPath;$InstallDir"
    [Environment]::SetEnvironmentVariable("PATH", $NewUserPath, "User")
    Write-Host "💡 Added $InstallDir permanently to your User PATH!" -ForegroundColor Green
}

# Update current session PATH immediately
if ($env:PATH -notlike "*$InstallDir*") {
    $env:PATH = "$env:PATH;$InstallDir"
}

Write-Host ""
Write-Host "════════════════════════════════════════════════════════════════" -ForegroundColor Cyan
Write-Host "  🎉 Kiwi successfully installed to $InstallDir\kiwi.exe!" -ForegroundColor Green
Write-Host "  ⚡ Type 'kiwi' from any terminal anytime to run speed tests!" -ForegroundColor Green
Write-Host "════════════════════════════════════════════════════════════════" -ForegroundColor Cyan
Write-Host ""
