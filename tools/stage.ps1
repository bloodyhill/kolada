# Builds Kolada for release and stages what ships in dist\Kolada: the app and
# the writing model's process. No model ships: the app downloads the one the
# person chooses into its data folder. -Model stages one beside the exe for a
# test build, which the app then uses over any choice (assist.rs `files`); it
# must be one of the catalogue's (crates\kolada-assist\src\models.rs), by its
# name and SHA-256.
param(
    [string]$Out = "dist\Kolada",
    [string]$Model = ""
)
Set-Location (Split-Path -Parent $PSScriptRoot)

if ($Model) {
    # The catalogue's models: the one list, which the training keeps.
    $catalogue = Get-Content crates\kolada-assist\src\models.rs -Raw
    $known = @{}
    foreach ($m in [regex]::Matches($catalogue, 'file:\s*"([^"]+)"[^}]*?sha256:\s*"([0-9A-Fa-f]{64})"')) { $known[$m.Groups[1].Value] = $m.Groups[2].Value.ToUpper() }
    $name = Split-Path $Model -Leaf
    if (-not $known.ContainsKey($name) -or (Get-FileHash $Model -Algorithm SHA256).Hash -ne $known[$name]) { throw "not a model in the catalogue: $Model" }
}

# A target dir of its own: target\release may hold the running Kolada.
# Cargo reports progress on stderr, which Windows PowerShell would take for
# an error: its exit code alone says how the build went.
& "$env:USERPROFILE\.cargo\bin\cargo.exe" build --release -p kolada-win -p kolada-ai --target-dir target\stage 2>&1 | ForEach-Object { "$_" } | Select-String -Pattern '^(error|warning)'
if ($LASTEXITCODE) { throw "build failed" }
$ErrorActionPreference = 'Stop'

New-Item -ItemType Directory -Force $Out | Out-Null
Copy-Item target\stage\release\kolada.exe, target\stage\release\kolada-ai.exe $Out
Copy-Item third_party\llama.cpp\LICENSE "$Out\LICENSE-llama.cpp.txt"
Copy-Item THIRD-PARTY.md $Out
# A model staged by an earlier run would be used over the person's choice.
Remove-Item -Recurse -Force "$Out\models" -ErrorAction SilentlyContinue
if ($Model) {
    New-Item -ItemType Directory -Force "$Out\models" | Out-Null
    Copy-Item $Model "$Out\models\"
    Copy-Item assets\models\LICENSE-LFM.txt "$Out\models\"
}
Get-ChildItem $Out -Recurse -File | Select-Object @{ n = 'File'; e = { $_.FullName.Substring((Resolve-Path $Out).Path.Length + 1) } }, @{ n = 'MB'; e = { [math]::Round($_.Length / 1MB, 2) } }
