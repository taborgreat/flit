# FLit dev install for Windows (session 2). Run from the repo root in PowerShell:
#   powershell -ExecutionPolicy Bypass -File tools\install\install-windows.ps1 [-NoBuild] [-SystemPlugins]
# What it does (the real installer is M6; this is the developer path):
#  1. builds flit-agent, fake-session and the plugin (VST3 + CLAP) in release (unless -NoBuild)
#  2. copies flit-agent.exe, fake-session.exe and the PyFLP helper to %LOCALAPPDATA%\flit\bin
#  3. makes a Python venv with pyflp at %LOCALAPPDATA%\flit\venv (level 2: reading .flp files)
#  4. copies the plugin bundles to the per-user plugin folders
#     (%LOCALAPPDATA%\Programs\Common\VST3 and \CLAP); with -SystemPlugins also to
#     C:\Program Files\Common Files\VST3 and \CLAP (one UAC prompt), which is where FL looks by default
#  5. copies the FL controller script into FL's hardware scripts folder
#  6. creates the loopMIDI ports "FLit In" and "FLit Out" (loopMIDI must be installed:
#     winget install TobiasErichsen.loopMIDI) by writing its registry list and restarting it
#  7. prints the one-time FL Studio steps (MIDI settings)
param(
    [switch]$NoBuild,
    [switch]$SystemPlugins
)
$ErrorActionPreference = "Stop"
$root = Split-Path -Parent (Split-Path -Parent (Split-Path -Parent $MyInvocation.MyCommand.Path))
Set-Location $root

# Toolchain on this box (see HANDOFF.md §3): rustup GNU host + portable mingw + cmake from the venv.
$mingw = Join-Path $env:LOCALAPPDATA "flit-tools\mingw64\bin"
if (Test-Path $mingw) { $env:PATH = "$mingw;$env:PATH" }
$cargoBin = Join-Path $env:USERPROFILE ".cargo\bin"
if (Test-Path $cargoBin) { $env:PATH = "$cargoBin;$env:PATH" }
$venvCmake = Join-Path $root ".venv\Scripts"
if (Test-Path $venvCmake) { $env:PATH = "$venvCmake;$env:PATH" }

if (-not $NoBuild) {
    cargo build --release -p flit-agent -p fake-session
    if ($LASTEXITCODE -ne 0) { throw "cargo build failed" }
    cargo xtask bundle flit-plugin --release
    if ($LASTEXITCODE -ne 0) { throw "cargo xtask bundle failed" }
}

# 2. agent + CLI + helper
$bin = Join-Path $env:LOCALAPPDATA "flit\bin"
New-Item -ItemType Directory -Force -Path $bin | Out-Null
Copy-Item -Force "target\release\flit-agent.exe" $bin
Copy-Item -Force "target\release\fake-session.exe" $bin
Copy-Item -Force "bridges\fl\flp\flit_flp.py" $bin

# 3. Python with pyflp for level 2 (project reading). Skipped quietly when there is no Python.
$venv = Join-Path $env:LOCALAPPDATA "flit\venv"
$py = Get-Command py -ErrorAction SilentlyContinue
if ($py) {
    if (-not (Test-Path (Join-Path $venv "Scripts\python.exe"))) {
        & py -3 -m venv $venv
    }
    & (Join-Path $venv "Scripts\python.exe") -m pip install -q pyflp
    if ($LASTEXITCODE -ne 0) { Write-Warning "pip install pyflp failed; level 2 (reading .flp files) stays off" }
} else {
    Write-Warning "No Python launcher (py) found; level 2 (reading .flp files) stays off. Install Python 3 and rerun."
}

# 4. plugins
$bundle = Join-Path $root "target\bundled"
$userVst3 = Join-Path $env:LOCALAPPDATA "Programs\Common\VST3"
$userClap = Join-Path $env:LOCALAPPDATA "Programs\Common\CLAP"
New-Item -ItemType Directory -Force -Path $userVst3, $userClap | Out-Null
Remove-Item -Recurse -Force (Join-Path $userVst3 "FLit.vst3") -ErrorAction SilentlyContinue
Copy-Item -Recurse -Force (Join-Path $bundle "FLit.vst3") $userVst3
Remove-Item -Recurse -Force (Join-Path $userVst3 "FLit.vst3\Contents\x86_64-linux") -ErrorAction SilentlyContinue
Copy-Item -Force (Join-Path $bundle "FLit.clap") $userClap
if ($SystemPlugins) {
    $cmd = "New-Item -ItemType Directory -Force -Path 'C:\Program Files\Common Files\VST3','C:\Program Files\Common Files\CLAP' | Out-Null; " +
           "Remove-Item -Recurse -Force 'C:\Program Files\Common Files\VST3\FLit.vst3' -ErrorAction SilentlyContinue; " +
           "Copy-Item -Recurse -Force '$bundle\FLit.vst3' 'C:\Program Files\Common Files\VST3\'; " +
           "Remove-Item -Recurse -Force 'C:\Program Files\Common Files\VST3\FLit.vst3\Contents\x86_64-linux' -ErrorAction SilentlyContinue; " +
           "Copy-Item -Force '$bundle\FLit.clap' 'C:\Program Files\Common Files\CLAP\'"
    Start-Process -FilePath powershell -Verb RunAs -Wait -ArgumentList "-NoProfile", "-Command", $cmd
}

# 5. FL controller script
$scripts = Join-Path ([Environment]::GetFolderPath("MyDocuments")) "Image-Line\FL Studio\Settings\Hardware\FLit"
New-Item -ItemType Directory -Force -Path $scripts | Out-Null
Copy-Item -Force "bridges\fl\fl-script\device_FLit.py" $scripts

# 6. loopMIDI ports (its port list lives in HKCU; it creates the ports when it starts)
$loop = "C:\Program Files (x86)\Tobias Erichsen\loopMIDI\loopMIDI.exe"
if (Test-Path $loop) {
    $ports = "HKCU:\Software\Tobias Erichsen\loopMIDI\Ports"
    if (-not (Test-Path $ports)) { New-Item -Path $ports -Force | Out-Null }
    foreach ($p in @("FLit In", "FLit Out")) {
        New-ItemProperty -Path $ports -Name $p -Value 0 -PropertyType DWord -Force | Out-Null
    }
    Get-Process loopMIDI -ErrorAction SilentlyContinue | Stop-Process -Force
    Start-Sleep -Seconds 1
    Start-Process -FilePath $loop
    Start-Sleep -Seconds 3
    $loopOk = $true
} else {
    $loopOk = $false
}

Write-Host ""
Write-Host "Installed:"
Write-Host "  $userVst3\FLit.vst3   (and $userClap\FLit.clap)"
if ($SystemPlugins) { Write-Host "  C:\Program Files\Common Files\VST3\FLit.vst3   (FL's default search folder)" }
Write-Host "  $bin\flit-agent.exe   (the plugin starts it on demand; it picks the FL bridge when the loopMIDI ports exist)"
Write-Host "  $bin\fake-session.exe (command line for the agent)"
Write-Host "  $scripts\device_FLit.py"
if ($loopOk) { Write-Host "  loopMIDI ports 'FLit In' and 'FLit Out' (loopMIDI restarted)" }
else { Write-Host "  loopMIDI NOT found: winget install TobiasErichsen.loopMIDI, then rerun this script" }
Write-Host ""
Write-Host "One-time steps in FL Studio (restart FL first so it sees the script and the ports):"
Write-Host "  1. Options > MIDI Settings, Output list: enable 'FLit Out', set Port to 42."
Write-Host "  2. Input list: enable 'FLit In', Controller type 'FLit (user)', Port 42."
Write-Host "     Leave 'FLit In' disabled in the Output list and 'FLit Out' disabled in the Input list."
Write-Host "  3. View > Script output > FLit tab should say 'init ok'."
Write-Host "  4. If FL does not list the FLit plugin: Options > Manage plugins, add $userVst3 as a plugin search folder"
Write-Host "     (or rerun with -SystemPlugins), then 'Find more plugins'."
Write-Host "  5. Add FLit on the master track. The panel goes green when the helper answers."
