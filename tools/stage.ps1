# Builds Kolada for release and stages what ships in dist\Kolada: the app, the
# writing model's process, and the model with its license. The model lives
# beside the exe, never inside it. -Model picks another of the known models
# (the app is built to look for that file).
param(
    [string]$Out = "dist\Kolada",
    [string]$Model = "$env:LOCALAPPDATA\Kolada\models\notsy-writer-1.2b-Q4_K_M.gguf"
)
Set-Location (Split-Path -Parent $PSScriptRoot)

# The models a build may ship, from tools\train's runs, renamed.
$known = @{
    'notsy-writer-1.2b-Q4_K_M.gguf' = '6F7FD5E9D9878621EA784A53786F2E5617F519118A04B53DD469C33BFF561857' # run "ctx"
    'notsy-writer-350m-Q4_K_M.gguf' = 'B4BCDF4A2F3C172F552D0EE16C38506B3A1C3B327589DB430870016D23BD42F8' # run "350m"
}
$name = Split-Path $Model -Leaf
if (-not $known.ContainsKey($name) -or (Get-FileHash $Model -Algorithm SHA256).Hash -ne $known[$name]) { throw "not a known model: $Model" }

# A target dir of its own: target\release may hold the running Kolada.
# Cargo reports progress on stderr, which Windows PowerShell would take for
# an error: its exit code alone says how the build went.
$env:KOLADA_MODEL_FILE = $name
& "$env:USERPROFILE\.cargo\bin\cargo.exe" build --release -p kolada-win -p kolada-ai --target-dir target\stage 2>&1 | ForEach-Object { "$_" } | Select-String -Pattern '^(error|warning)'
$built = $LASTEXITCODE
Remove-Item Env:KOLADA_MODEL_FILE
if ($built) { throw "build failed" }
$ErrorActionPreference = 'Stop'

New-Item -ItemType Directory -Force "$Out\models" | Out-Null
Copy-Item target\stage\release\kolada.exe, target\stage\release\kolada-ai.exe $Out
Copy-Item $Model "$Out\models\"
Copy-Item assets\models\LICENSE-LFM.txt "$Out\models\"
Copy-Item third_party\llama.cpp\LICENSE "$Out\LICENSE-llama.cpp.txt"
Copy-Item THIRD-PARTY.md $Out
Get-ChildItem $Out -Recurse -File | Select-Object @{ n = 'File'; e = { $_.FullName.Substring((Resolve-Path $Out).Path.Length + 1) } }, @{ n = 'MB'; e = { [math]::Round($_.Length / 1MB, 2) } }
