$ErrorActionPreference = "Stop"

$vsDevCmd = "C:\BuildTools\Common7\Tools\VsDevCmd.bat"
if (-not (Test-Path $vsDevCmd)) {
    throw "No se encontro VsDevCmd.bat en $vsDevCmd"
}

$docsPath = if ($args.Count -gt 0) { $args[0] } else { "Documentation/docs" }
$cdFile = Join-Path $env:TEMP "mdnav-pending-cd.txt"

if (Test-Path $cdFile) {
    Remove-Item -LiteralPath $cdFile -Force
}

$env:MDNAV_CD_FILE = $cdFile
try {
    cmd /c "call `"$vsDevCmd`" -arch=x64 -host_arch=x64 && set PATH=%USERPROFILE%\.cargo\bin;%PATH% && cargo run -- `"$docsPath`""
} finally {
    Remove-Item Env:MDNAV_CD_FILE -ErrorAction SilentlyContinue
}

if (Test-Path $cdFile) {
    $targetDir = (Get-Content $cdFile -Raw).Trim()
    Remove-Item -LiteralPath $cdFile -Force -ErrorAction SilentlyContinue

    if ($targetDir) {
        Set-Location -LiteralPath $targetDir
        Write-Host "mdnav cambio la terminal a: $targetDir"
    }
}
