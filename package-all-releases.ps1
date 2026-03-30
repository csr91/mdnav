$ErrorActionPreference = "Stop"

$repoRoot = Split-Path -Parent $MyInvocation.MyCommand.Path

& (Join-Path $repoRoot "package-release.ps1")
& (Join-Path $repoRoot "package-release-linux.ps1")

Write-Host "Paquetes generados en $(Join-Path $repoRoot 'dist')"
