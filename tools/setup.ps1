# Builds what the website serves for Windows, in dist\web:
#
#   Notsy-Setup.exe        the installer (notsy.exe and THIRD-PARTY.md inside)
#   Notsy-<version>.exe    the program alone, what the updater downloads
#   latest.json            the update feed: version, url, sha256, size
#
#   .\tools\setup.ps1
#
# Copy the three into notsy-web\public\download\ to publish them (ask the
# user first: it goes online). The version is the workspace's.
param(
  [string] $Out = "dist\web",
  [string] $Site = "https://notsy.co/download"
)
$ErrorActionPreference = "Stop"
Set-Location (Split-Path $PSScriptRoot -Parent)

$version = (Select-String -Path Cargo.toml -Pattern '^version\s*=\s*"([^"]+)"' | Select-Object -First 1).Matches[0].Groups[1].Value

# A target dir of its own: target\release may hold the running Notsy.
# Cargo reports progress on stderr, which Windows PowerShell would take for a failure.
$ErrorActionPreference = "Continue"
cargo build -q --release -p notsy-win -p notsy-setup --target-dir target\stage 2>&1 | Where-Object { "$_" -match '^error' }
$ErrorActionPreference = "Stop"
if ($LASTEXITCODE) { throw "build failed" }

New-Item -ItemType Directory -Force $Out | Out-Null
$setup = Join-Path $Out "Notsy-Setup.exe"
$p = Start-Process -FilePath target\stage\release\notsy-setup.exe -ArgumentList "--pack", "`"$setup`"", "target\stage\release\notsy.exe", "THIRD-PARTY.md" -Wait -PassThru -NoNewWindow
if ($p.ExitCode) { throw "packing failed ($($p.ExitCode))" }

$app = Join-Path $Out "Notsy-$version.exe"
Copy-Item target\stage\release\notsy.exe $app
$item = Get-Item $app
$feed = [ordered]@{
  version = $version
  url     = "$Site/Notsy-$version.exe"
  sha256  = (Get-FileHash $app -Algorithm SHA256).Hash.ToLower()
  size    = $item.Length
}
$json = ($feed | ConvertTo-Json) + "`n"
[IO.File]::WriteAllText((Join-Path $Out "latest.json"), $json, (New-Object Text.UTF8Encoding $false))

Get-ChildItem $Out -File | Select-Object Name, @{ n = 'MB'; e = { [math]::Round($_.Length / 1MB, 2) } }
$json
