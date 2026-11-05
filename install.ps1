# Builds Scout Desktop and installs it for the current user, with a Start menu entry so Windows search finds it.
#   powershell -ExecutionPolicy Bypass -File install.ps1            install or update
#   powershell -ExecutionPolicy Bypass -File install.ps1 -Uninstall
param([switch]$Uninstall, [switch]$Desktop)
$ErrorActionPreference = 'Stop'

$dest = Join-Path $env:LOCALAPPDATA 'Programs\Scout'
$exe = Join-Path $dest 'Scout.exe'
$startMenu = Join-Path ([Environment]::GetFolderPath('Programs')) 'Scout.lnk'
$desktopLink = Join-Path ([Environment]::GetFolderPath('Desktop')) 'Scout.lnk'
$uninstallKey = 'HKCU:\Software\Microsoft\Windows\CurrentVersion\Uninstall\ScoutDesktop'

Get-Process Scout -ErrorAction SilentlyContinue | Where-Object { $_.Path -eq $exe } | Stop-Process -Force

if ($Uninstall) {
    Remove-Item $startMenu, $desktopLink -Force -ErrorAction SilentlyContinue
    Remove-Item $uninstallKey -Recurse -Force -ErrorAction SilentlyContinue
    # Registered by the app for its Windows notifications.
    Remove-Item 'HKCU:\Software\Classes\AppUserModelId\ScoutCode.Scout' -Recurse -Force -ErrorAction SilentlyContinue
    Remove-Item $dest -Recurse -Force -ErrorAction SilentlyContinue
    Write-Host 'Scout Desktop removed. Your chats and login in ~/.scoutcode are kept.'
    return
}

Push-Location $PSScriptRoot
try {
    # The web UI goes into the exe, so it's built first.
    npm --prefix ui ci
    if ($LASTEXITCODE -ne 0) { throw 'npm ci failed' }
    npm --prefix ui run build
    if ($LASTEXITCODE -ne 0) { throw 'the UI build failed' }
    cargo build --release
    if ($LASTEXITCODE -ne 0) { throw 'cargo build failed' }
} finally { Pop-Location }

New-Item -ItemType Directory -Force $dest | Out-Null
Get-Process Scout -ErrorAction SilentlyContinue | Where-Object { $_.Path -eq $exe } | Stop-Process -Force
Copy-Item (Join-Path $PSScriptRoot 'target\release\scout-desktop.exe') $exe -Force
Copy-Item $PSCommandPath (Join-Path $dest 'install.ps1') -Force

$shell = New-Object -ComObject WScript.Shell
$links = @($startMenu) + $(if ($Desktop) { @($desktopLink) } else { @() })
foreach ($path in $links) {
    $lnk = $shell.CreateShortcut($path)
    $lnk.TargetPath = $exe
    $lnk.WorkingDirectory = $dest
    $lnk.IconLocation = "$exe,0"
    $lnk.Description = 'Scout: a coding agent for your projects'
    $lnk.Save()
}

# Lists it under Settings > Apps so it can be removed like any other app.
New-Item $uninstallKey -Force | Out-Null
$version = [regex]::Match((Get-Content (Join-Path $PSScriptRoot 'Cargo.toml') -Raw), '(?m)^version = "([^"]+)"').Groups[1].Value
$props = @{
    DisplayName = 'Scout'; DisplayIcon = "$exe,0"; Publisher = 'ScoutCode'; DisplayVersion = $version
    InstallLocation = $dest; NoModify = 1; NoRepair = 1
    UninstallString = "powershell.exe -NoProfile -ExecutionPolicy Bypass -File `"$dest\install.ps1`" -Uninstall"
}
foreach ($k in $props.Keys) { New-ItemProperty $uninstallKey -Name $k -Value $props[$k] -Force | Out-Null }

Write-Host "Installed to $exe. Search for Scout in the Start menu."
