$ErrorActionPreference = "Stop"

$repoRoot = Split-Path -Parent $MyInvocation.MyCommand.Path
$binaryPath = Join-Path $repoRoot "target\release\mdnav.exe"
$distRoot = Join-Path $repoRoot "dist"
$packageRoot = Join-Path $distRoot "mdnav-windows-x86_64"
$zipPath = Join-Path $distRoot "mdnav-windows-x86_64.zip"

& (Join-Path $repoRoot "build-release.ps1")

if (-not (Test-Path $binaryPath)) {
    throw "No se encontro el binario release en $binaryPath"
}

$cargoToml = Get-Content -LiteralPath (Join-Path $repoRoot "Cargo.toml")
$versionLine = $cargoToml | Where-Object { $_ -match '^version\s*=' } | Select-Object -First 1
$version = ($versionLine -replace '^version\s*=\s*"', '') -replace '"\s*$', ''
$actualVersion = & $binaryPath --version
if ($actualVersion -ne "mdnav $version") {
    throw "El binario release dice '$actualVersion', se esperaba 'mdnav $version'"
}

if (Test-Path $packageRoot) {
    Remove-Item -LiteralPath $packageRoot -Recurse -Force
}

if (Test-Path $zipPath) {
    Remove-Item -LiteralPath $zipPath -Force
}

New-Item -ItemType Directory -Path $packageRoot -Force | Out-Null
Copy-Item -LiteralPath $binaryPath -Destination (Join-Path $packageRoot "mdnav.exe")
Copy-Item -LiteralPath (Join-Path $repoRoot "README.md") -Destination (Join-Path $packageRoot "README.md")

Compress-Archive -Path (Join-Path $packageRoot "*") -DestinationPath $zipPath

Write-Host "Paquete generado en: $zipPath"
