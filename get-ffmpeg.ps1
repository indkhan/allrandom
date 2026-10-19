# Downloads a static ffmpeg build (gyan.dev, LGPL/GPL) into the Tauri sidecar slot.
$ErrorActionPreference = "Stop"
$dest = "$PSScriptRoot\src-tauri\binaries\ffmpeg-x86_64-pc-windows-msvc.exe"
New-Item -ItemType Directory -Force (Split-Path $dest) | Out-Null
$zip = "$env:TEMP\ffmpeg-essentials.zip"
Invoke-WebRequest "https://www.gyan.dev/ffmpeg/builds/ffmpeg-release-essentials.zip" -OutFile $zip
$dir = "$env:TEMP\ffmpeg-essentials"
Expand-Archive $zip $dir -Force
Copy-Item (Get-ChildItem $dir -Recurse -Filter ffmpeg.exe | Select-Object -First 1).FullName $dest
Write-Host "ffmpeg -> $dest"
