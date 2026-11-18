<#
.SYNOPSIS
    Compila o Workploy e gera o instalador nativo (.exe) e o pacote portátil (.zip) em dist/.
#>
[CmdletBinding()]
param()

$ErrorActionPreference = 'Stop'

$RepoRoot = (Resolve-Path (Join-Path $PSScriptRoot '..')).Path
$AppManifest = Join-Path $RepoRoot 'app\Cargo.toml'
$InstallerManifest = Join-Path $RepoRoot 'installer\Cargo.toml'
$AppExe = Join-Path $RepoRoot 'app\target\release\workploy.exe'
$SetupExe = Join-Path $RepoRoot 'installer\target\release\workploy-setup.exe'
$DistDir = Join-Path $RepoRoot 'dist'
$VersionMatch = Select-String -Path $AppManifest -Pattern '^version\s*=\s*"([^"]+)"' | Select-Object -First 1
if (-not $VersionMatch) {
    throw "Não foi possível detectar a versão em $AppManifest."
}
$Version = $VersionMatch.Matches[0].Groups[1].Value

Write-Host "==> Compilando workploy (release)..." -ForegroundColor Cyan
& cargo build --release --manifest-path $AppManifest
if ($LASTEXITCODE -ne 0) {
    throw "Falha ao compilar app (código $LASTEXITCODE)."
}
if (-not (Test-Path $AppExe)) {
    throw "Executável esperado não encontrado: $AppExe"
}

Write-Host "==> Compilando workploy-setup (release)..." -ForegroundColor Cyan
& cargo build --release --manifest-path $InstallerManifest
if ($LASTEXITCODE -ne 0) {
    throw "Falha ao compilar installer (código $LASTEXITCODE)."
}
if (-not (Test-Path $SetupExe)) {
    throw "Instalador esperado não encontrado: $SetupExe"
}

New-Item -ItemType Directory -Path $DistDir -Force | Out-Null

$OutSetup = Join-Path $DistDir "Workploy-Setup-$Version.exe"
Copy-Item -Path $SetupExe -Destination $OutSetup -Force

$OutZip = Join-Path $DistDir "workploy-$Version-win64-portable.zip"
$StagingDir = Join-Path $DistDir "_staging_portable_$Version"
if (Test-Path $StagingDir) {
    Remove-Item -Path $StagingDir -Recurse -Force
}
New-Item -ItemType Directory -Path $StagingDir -Force | Out-Null

try {
    Copy-Item -Path $AppExe -Destination (Join-Path $StagingDir 'workploy.exe') -Force
    New-Item -ItemType File -Path (Join-Path $StagingDir 'portable.flag') -Force | Out-Null

    if (Test-Path $OutZip) {
        Remove-Item -Path $OutZip -Force
    }
    Compress-Archive -Path (Join-Path $StagingDir '*') -DestinationPath $OutZip -CompressionLevel Optimal
}
finally {
    if (Test-Path $StagingDir) {
        Remove-Item -Path $StagingDir -Recurse -Force
    }
}

Write-Host ""
Write-Host "==> Artefatos gerados em dist/:" -ForegroundColor Green
foreach ($artifact in @($OutSetup, $OutZip)) {
    $info = Get-Item $artifact
    if (Get-Command Get-FileHash -ErrorAction SilentlyContinue) {
        $hash = (Get-FileHash -Path $artifact -Algorithm SHA256).Hash
    } else {
        $stream = [System.IO.File]::OpenRead($artifact)
        try {
            $sha = [System.Security.Cryptography.SHA256]::Create()
            $hash = ([System.BitConverter]::ToString($sha.ComputeHash($stream)) -replace '-', '')
        } finally {
            $stream.Dispose()
        }
    }
    $sizeMb = [Math]::Round($info.Length / 1MB, 2)
    Write-Host ("  {0} ({1} bytes / {2} MB)" -f $info.Name, $info.Length, $sizeMb)
    Write-Host ("    SHA256: {0}" -f $hash)
}
