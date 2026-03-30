$ErrorActionPreference = "Stop"

$repoRoot = Split-Path -Parent $MyInvocation.MyCommand.Path
$binaryPath = Join-Path $repoRoot "target\x86_64-unknown-linux-musl\release\mdnav"
$distRoot = Join-Path $repoRoot "dist"
$packageRoot = Join-Path $distRoot "mdnav-linux-x86_64"
$archivePath = Join-Path $distRoot "mdnav-linux-x86_64.tar.gz"

& (Join-Path $repoRoot "build-release-linux.ps1")

if (-not (Test-Path $binaryPath)) {
    throw "No se encontro el binario Linux release en $binaryPath"
}

if (Test-Path $packageRoot) {
    Remove-Item -LiteralPath $packageRoot -Recurse -Force
}

if (Test-Path $archivePath) {
    Remove-Item -LiteralPath $archivePath -Force
}

New-Item -ItemType Directory -Path $packageRoot -Force | Out-Null
Copy-Item -LiteralPath $binaryPath -Destination (Join-Path $packageRoot "mdnav")
Copy-Item -LiteralPath (Join-Path $repoRoot "README.md") -Destination (Join-Path $packageRoot "README.md")

tar -czf $archivePath -C $packageRoot .

Write-Host "Paquete generado en: $archivePath"
