$ErrorActionPreference = "Stop"

# 1. Define custom Profile Directory in user's home directory
$ProfileDir = Join-Path $HOME ".quotify\BrowserProfile"
if (-not (Test-Path $ProfileDir)) {
    Write-Host "Creating browser profile directory at $ProfileDir..." -ForegroundColor Gray
    New-Item -ItemType Directory -Path $ProfileDir -Force | Out-Null
} else {
    Write-Host "Using browser profile directory at $ProfileDir" -ForegroundColor Gray
}

# 2. Locate Chrome installation path
$ChromePaths = @(
    "C:\Program Files\Google\Chrome\Application\chrome.exe",
    "C:\Program Files (x86)\Google\Chrome\Application\chrome.exe"
)

# Read from registry as primary lookup
$RegistryPath = Get-ItemPropertyValue -Path "HKLM:\SOFTWARE\Microsoft\Windows\CurrentVersion\App Paths\chrome.exe" -Name "(Default)" -ErrorAction SilentlyContinue
if ($RegistryPath) {
    $ChromePaths = @($RegistryPath) + $ChromePaths
}

$ChromeExe = $null
foreach ($Path in $ChromePaths) {
    if (Test-Path $Path) {
        $ChromeExe = $Path
        break
    }
}

if (-not $ChromeExe) {
    Write-Error "Google Chrome executable was not found. Please make sure Chrome is installed."
    return
}

# 3. Launch Chrome in Debug mode
Write-Host "Launching Google Chrome with remote debugging on port 9222..." -ForegroundColor Green
Write-Host "Executable: $ChromeExe" -ForegroundColor Gray
Write-Host "Profile: $ProfileDir" -ForegroundColor Gray

Start-Process $ChromeExe -ArgumentList "--remote-debugging-port=9222", "--user-data-dir=$ProfileDir"

Write-Host "`nChrome started successfully!" -ForegroundColor Green
Write-Host "Please navigate to your target site (e.g. xiaomimimo.com or claude.ai) in the new Chrome window and log in." -ForegroundColor Yellow
Write-Host "Ensure to check 'Allow remote debugging for this browser instance' on the chrome://inspect page if prompted." -ForegroundColor Yellow
