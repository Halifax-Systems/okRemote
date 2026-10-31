# Needs Visual Studio 2022 build tools (C++ workload) and WebView2. Produces MSI, NSIS setup and a portable zip.
$ErrorActionPreference = 'Stop'
Set-Location "$PSScriptRoot\..\app"
npm ci
npx tauri build
$out = "..\dist\0.1.0\windows"
if (Test-Path $out) { Remove-Item -Recurse -Force $out }
New-Item -ItemType Directory -Force $out | Out-Null
Copy-Item ..\target\release\bundle\msi\*.msi $out
Copy-Item ..\target\release\bundle\nsis\*.exe $out
$tmp = Join-Path $env:TEMP 'okremote-portable'
Remove-Item -Recurse -Force $tmp -ErrorAction SilentlyContinue
New-Item -ItemType Directory $tmp | Out-Null
Copy-Item ..\target\release\okremote.exe "$tmp\okRemote.exe"
New-Item -ItemType File "$tmp\portable" | Out-Null
Compress-Archive -Path "$tmp\*" -DestinationPath "$out\okRemote-portable.zip"
