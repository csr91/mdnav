$ErrorActionPreference = "Stop"

# Force TLS 1.2 — required by GitHub API (PowerShell 5.1 defaults to TLS 1.0)
[Net.ServicePointManager]::SecurityProtocol = [Net.SecurityProtocolType]::Tls12

$Version = "latest"
$Repo = "csr91/mdnav"
$headers = @{ "User-Agent" = "mdnav-installer" }

$installRoot = Join-Path $env:LOCALAPPDATA "mdnav"
$binDir = Join-Path $installRoot "bin"
$zipPath = Join-Path $env:TEMP "mdnav-windows-x86_64.zip"

New-Item -ItemType Directory -Path $binDir -Force | Out-Null

if ($Version -eq "latest") {
    $releaseApi = "https://api.github.com/repos/$Repo/releases/latest"
} else {
    $releaseApi = "https://api.github.com/repos/$Repo/releases/tags/$Version"
}

$release = Invoke-RestMethod -Uri $releaseApi -Headers $headers
$asset = $release.assets | Where-Object { $_.name -eq "mdnav-windows-x86_64.zip" } | Select-Object -First 1

if (-not $asset) {
    throw "No se encontro el asset mdnav-windows-x86_64.zip en la release solicitada."
}

Write-Host "Descargando mdnav $($release.tag_name)..."
Invoke-WebRequest -Uri $asset.browser_download_url -OutFile $zipPath -Headers $headers

$stagingDir = Join-Path $env:TEMP "mdnav-install"
if (Test-Path $stagingDir) {
    Remove-Item -LiteralPath $stagingDir -Recurse -Force
}
New-Item -ItemType Directory -Path $stagingDir -Force | Out-Null

Expand-Archive -Path $zipPath -DestinationPath $stagingDir -Force

$exeSrc = Join-Path $stagingDir "mdnav.exe"
if (-not (Test-Path $exeSrc)) {
    throw "No se encontro mdnav.exe dentro del zip descargado."
}
Copy-Item -LiteralPath $exeSrc -Destination (Join-Path $binDir "mdnav.exe") -Force

$userPath = [Environment]::GetEnvironmentVariable("Path", "User")
$pathEntries = @()
if ($userPath) {
    $pathEntries = $userPath -split ';' | Where-Object { $_ -ne "" }
}

if ($pathEntries -notcontains $binDir) {
    $newPath = if ($userPath) { "$userPath;$binDir" } else { $binDir }
    [Environment]::SetEnvironmentVariable("Path", $newPath, "User")
    Write-Host "Se agrego $binDir al PATH del usuario."
    Write-Host "Abri una nueva terminal para usar 'mdnav'."
} else {
    Write-Host "$binDir ya estaba en PATH."
}

Remove-Item -LiteralPath $zipPath -Force -ErrorAction SilentlyContinue
Remove-Item -LiteralPath $stagingDir -Recurse -Force -ErrorAction SilentlyContinue

Write-Host "mdnav $($release.tag_name) instalado en $binDir"
