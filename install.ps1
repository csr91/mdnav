$ErrorActionPreference = "Stop"

param(
    [string]$Version = "latest",
    [string]$Repo = "csr91/mdnav"
)

$installRoot = Join-Path $env:LOCALAPPDATA "mdnav"
$binDir = Join-Path $installRoot "bin"
$zipPath = Join-Path $env:TEMP "mdnav-windows-x86_64.zip"

New-Item -ItemType Directory -Path $binDir -Force | Out-Null

if ($Version -eq "latest") {
    $releaseApi = "https://api.github.com/repos/$Repo/releases/latest"
} else {
    $releaseApi = "https://api.github.com/repos/$Repo/releases/tags/$Version"
}

$release = Invoke-RestMethod -Uri $releaseApi
$asset = $release.assets | Where-Object { $_.name -eq "mdnav-windows-x86_64.zip" } | Select-Object -First 1

if (-not $asset) {
    throw "No se encontro el asset mdnav-windows-x86_64.zip en la release solicitada."
}

Invoke-WebRequest -Uri $asset.browser_download_url -OutFile $zipPath

$stagingDir = Join-Path $env:TEMP "mdnav-install"
if (Test-Path $stagingDir) {
    Remove-Item -LiteralPath $stagingDir -Recurse -Force
}

Expand-Archive -Path $zipPath -DestinationPath $stagingDir -Force
Copy-Item -LiteralPath (Join-Path $stagingDir "mdnav.exe") -Destination (Join-Path $binDir "mdnav.exe") -Force

$userPath = [Environment]::GetEnvironmentVariable("Path", "User")
$pathEntries = @()
if ($userPath) {
    $pathEntries = $userPath -split ';' | Where-Object { $_ -ne "" }
}

if ($pathEntries -notcontains $binDir) {
    $newPath = if ($userPath) { "$userPath;$binDir" } else { $binDir }
    [Environment]::SetEnvironmentVariable("Path", $newPath, "User")
    Write-Host "Se agrego $binDir al PATH del usuario. Abri una nueva terminal para usar 'mdnav'."
} else {
    Write-Host "$binDir ya estaba en PATH."
}

Remove-Item -LiteralPath $zipPath -Force -ErrorAction SilentlyContinue
Remove-Item -LiteralPath $stagingDir -Recurse -Force -ErrorAction SilentlyContinue

Write-Host "mdnav instalado en $binDir"
