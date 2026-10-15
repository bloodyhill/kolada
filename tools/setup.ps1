# Builds what the website serves for Windows, in dist\web:
#
#   Kolada-Setup.exe        the installer: kolada.exe, its writing helper
#                          kolada-ai.exe with llama.cpp's license
#                          (LICENSE-llama.cpp.txt), and THIRD-PARTY.md inside.
#                          No writing model: the app downloads the one the
#                          person chooses
#   Kolada-<version>.exe    the program alone, what the updater downloads
#   kolada-ai-<version>.exe the writing helper of that version: updates bring
#                          it along, and a copy without one fetches it
#   feed.json              the update feed: version, url, sha256, size, and
#                          the helper's helper_url, helper_sha256, helper_size
#   latest.json            the same for installs from before the rename to
#                          Kolada, which ask https://notsy.co/download/latest.json:
#                          its download is on that site (the app takes a
#                          download only from the site it asked)
#
#   .\tools\setup.ps1
#
# Copy the four into notsy-web\public\download\ to publish them (ask the
# user first: it goes online). The version is the workspace's.
param(
  [string] $Out = "dist\web",
  [string] $Site = "https://kolada.app/download",
  [string] $OldSite = "https://notsy.co/download"
)
$ErrorActionPreference = "Stop"
Set-Location (Split-Path $PSScriptRoot -Parent)

$version = (Select-String -Path Cargo.toml -Pattern '^version\s*=\s*"([^"]+)"' | Select-Object -First 1).Matches[0].Groups[1].Value

# A target dir of its own: target\release may hold the running Kolada.
# Cargo reports progress on stderr, which Windows PowerShell would take for a failure.
$ErrorActionPreference = "Continue"
# kolada-ai builds llama.cpp from the third_party\llama.cpp submodule.
if (-not (Test-Path third_party\llama.cpp\LICENSE)) { throw "third_party\llama.cpp is not checked out: git submodule update --init" }
cargo build -q --release -p kolada-win -p kolada-setup -p kolada-ai --target-dir target\stage 2>&1 | Where-Object { "$_" -match '^error' }
$ErrorActionPreference = "Stop"
if ($LASTEXITCODE) { throw "build failed" }

# The installer names each file it carries as it is named here.
$license = "target\stage\LICENSE-llama.cpp.txt"
Copy-Item third_party\llama.cpp\LICENSE $license

New-Item -ItemType Directory -Force $Out | Out-Null
$setup = Join-Path $Out "Kolada-Setup.exe"
$p = Start-Process -FilePath target\stage\release\kolada-setup.exe -ArgumentList "--pack", "`"$setup`"", "target\stage\release\kolada.exe", "target\stage\release\kolada-ai.exe", "THIRD-PARTY.md", $license -Wait -PassThru -NoNewWindow
if ($p.ExitCode) { throw "packing failed ($($p.ExitCode))" }

$app = Join-Path $Out "Kolada-$version.exe"
Copy-Item target\stage\release\kolada.exe $app
$item = Get-Item $app
$sha = (Get-FileHash $app -Algorithm SHA256).Hash.ToLower()
# The writing helper of this version: an update brings it along, and a copy
# without one fetches it (kolada_core::update::Feed::helper).
$helper = Join-Path $Out "kolada-ai-$version.exe"
Copy-Item target\stage\release\kolada-ai.exe $helper
$helperItem = Get-Item $helper
$helperSha = (Get-FileHash $helper -Algorithm SHA256).Hash.ToLower()
foreach ($f in @(@("feed.json", $Site), @("latest.json", $OldSite))) {
  $feed = [ordered]@{
    version       = $version
    url           = "$($f[1])/Kolada-$version.exe"
    sha256        = $sha
    size          = $item.Length
    helper_url    = "$($f[1])/kolada-ai-$version.exe"
    helper_sha256 = $helperSha
    helper_size   = $helperItem.Length
  }
  $json = ($feed | ConvertTo-Json) + "`n"
  [IO.File]::WriteAllText((Join-Path $Out $f[0]), $json, (New-Object Text.UTF8Encoding $false))
}

Get-ChildItem $Out -File | Select-Object Name, @{ n = 'MB'; e = { [math]::Round($_.Length / 1MB, 2) } }
$json
