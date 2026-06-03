$ErrorActionPreference = "Stop"

$installRoot = Join-Path $env:LOCALAPPDATA "mdnav"
$binDir = Join-Path $installRoot "bin"

if (Test-Path $installRoot) {
    Remove-Item -LiteralPath $installRoot -Recurse -Force
    Write-Host "mdnav eliminado de $installRoot"
} else {
    Write-Host "mdnav no estaba instalado en $installRoot"
}

$userPath = [Environment]::GetEnvironmentVariable("Path", "User")
if ($userPath) {
    $entries = $userPath -split ';' | Where-Object { $_ -ne $binDir -and $_ -ne "" }
    [Environment]::SetEnvironmentVariable("Path", $entries -join ';', "User")
    Write-Host "Entrada eliminada del PATH del usuario."
}

$hookMarker = "mdnav --shell-hook powershell"
if (Test-Path $PROFILE) {
    $lines = Get-Content $PROFILE | Where-Object { $_ -notmatch [regex]::Escape($hookMarker) -and $_ -notmatch "function mdnav" -and $_ -notmatch "MDNAV_CD_FILE" -and $_ -notmatch "mdnav.exe.*@args" -and $_ -notmatch "Env:\\MDNAV" -and $_ -notmatch "Set-Location.*target" }
    Set-Content $PROFILE $lines -Encoding utf8
    Write-Host "Hook de PowerShell eliminado del perfil."
}

Write-Host "Desinstalacion completa. Abri una nueva terminal para aplicar los cambios."
