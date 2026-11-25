<#
.SYNOPSIS
Builds the Windows release of Planar into .\dist:
  Planar-<version>-x64-setup.exe     installer (Inno Setup)
  Planar-<version>-x64-portable.exe  the bare executable
  SHA256SUMS.txt                     checksums of both

.DESCRIPTION
Needs Rust with the MSVC toolchain (https://rustup.rs), the Visual Studio
Build Tools with the "Desktop development with C++" workload, and Inno Setup 6:
  winget install Rustlang.Rustup JRSoftware.InnoSetup
  winget install Microsoft.VisualStudio.2022.BuildTools --override "--passive --wait --add Microsoft.VisualStudio.Workload.VCTools --includeRecommended"

Code signing is optional. When a certificate is given, planar.exe, the
installer and its uninstaller are signed with signtool (Windows SDK) and
timestamped. Use either a certificate from the Windows certificate store
(also covers USB tokens and cloud certificates such as Certum SimplySign) by
its SHA-1 thumbprint, or a .pfx file.

.PARAMETER CertThumbprint
SHA-1 thumbprint of a code-signing certificate in the certificate store.
Default: $env:PLANAR_SIGN_THUMBPRINT.

.PARAMETER PfxFile
Path to a .pfx code-signing certificate. Default: $env:PLANAR_SIGN_PFX.

.PARAMETER PfxPassword
Password of the .pfx file. Default: $env:PLANAR_SIGN_PFX_PASSWORD.

.PARAMETER SkipTests
Don't run `cargo test` first.

.EXAMPLE
powershell -ExecutionPolicy Bypass -File scripts\build-windows.ps1
.EXAMPLE
powershell -ExecutionPolicy Bypass -File scripts\build-windows.ps1 -CertThumbprint 0123456789ABCDEF0123456789ABCDEF01234567
#>
param(
    [string]$CertThumbprint = $env:PLANAR_SIGN_THUMBPRINT,
    [string]$PfxFile = $env:PLANAR_SIGN_PFX,
    [string]$PfxPassword = $env:PLANAR_SIGN_PFX_PASSWORD,
    [string]$TimestampUrl = 'http://timestamp.digicert.com',
    [switch]$SkipTests
)
$ErrorActionPreference = 'Stop'
$root = Split-Path -Parent $PSScriptRoot
Set-Location $root

# Runs a native program and stops the script if it fails.
function Exec([string]$Exe, [string[]]$Arguments) {
    $old = $ErrorActionPreference
    $ErrorActionPreference = 'Continue'
    try { & $Exe @Arguments } finally { $ErrorActionPreference = $old }
    if ($LASTEXITCODE -ne 0) { throw "$(Split-Path -Leaf $Exe) failed with exit code $LASTEXITCODE" }
}

function Find-Tool([string]$Name, [string[]]$Candidates) {
    $cmd = Get-Command $Name -ErrorAction SilentlyContinue
    if ($cmd) { return $cmd.Source }
    foreach ($c in $Candidates) { if ($c -and (Test-Path $c)) { return (Resolve-Path $c).Path } }
    throw "$Name not found. See the requirements at the top of scripts\build-windows.ps1."
}

$cargo = Find-Tool 'cargo' @("$env:USERPROFILE\.cargo\bin\cargo.exe")
$iscc = Find-Tool 'ISCC' @("${env:ProgramFiles(x86)}\Inno Setup 6\ISCC.exe", "$env:ProgramFiles\Inno Setup 6\ISCC.exe", "$env:LOCALAPPDATA\Programs\Inno Setup 6\ISCC.exe")

$version = (Select-String -Path Cargo.toml -Pattern '^version\s*=\s*"([^"]+)"' | Select-Object -First 1).Matches[0].Groups[1].Value
$fileVersion = ($version -split '[-+]')[0]
Write-Host "==> Planar $version" -ForegroundColor Cyan

# --- Build ------------------------------------------------------------------
if (-not $SkipTests) {
    Write-Host '==> cargo test' -ForegroundColor Cyan
    Exec $cargo @('test', '--release')
}
Write-Host '==> cargo build' -ForegroundColor Cyan
Exec $cargo @('build', '--release')
$exe = Join-Path $root 'target\release\planar.exe'

# --- Signing ----------------------------------------------------------------
$signArgs = $null
if ($CertThumbprint) {
    $signArgs = @('/sha1', ($CertThumbprint -replace '\s', ''))
} elseif ($PfxFile) {
    $signArgs = @('/f', (Resolve-Path $PfxFile).Path)
    if ($PfxPassword) { $signArgs += @('/p', $PfxPassword) }
}
if ($signArgs) {
    $kits = Get-ChildItem "${env:ProgramFiles(x86)}\Windows Kits\10\bin\10.*\x64\signtool.exe" -ErrorAction SilentlyContinue |
        Sort-Object { [version]$_.Directory.Parent.Name } | Select-Object -Last 1
    $signtool = Find-Tool 'signtool' @($(if ($kits) { $kits.FullName }))
    $signArgs = @('sign', '/fd', 'sha256', '/tr', $TimestampUrl, '/td', 'sha256',
        '/d', 'Planar', '/du', 'https://github.com/MT-Blachetta/fileView') + $signArgs
    Write-Host '==> Signing planar.exe' -ForegroundColor Cyan
    Exec $signtool ($signArgs + @($exe))
} else {
    Write-Host '==> No certificate given: building UNSIGNED (see README, "Code signing")' -ForegroundColor Yellow
}

# --- Installer --------------------------------------------------------------
Write-Host '==> Inno Setup' -ForegroundColor Cyan
New-Item -ItemType Directory -Force dist | Out-Null
$isccArgs = @('/Q', "/DAppVersion=$version", "/DFileVersion=$fileVersion", "/DSourceExe=$exe")
if ($signArgs) {
    # Inno Setup substitutes $q with a quote and $f with the quoted file name.
    $quoted = $signArgs | ForEach-Object { if ($_ -match '\s') { '$q' + $_ + '$q' } else { $_ } }
    $isccArgs += @('/DSign', ('/Splanarsign=$q{0}$q {1} $f' -f $signtool, ($quoted -join ' ')))
}
Exec $iscc ($isccArgs + @('installer\planar.iss'))

$setup = Join-Path $root "dist\Planar-$version-x64-setup.exe"
$portable = Join-Path $root "dist\Planar-$version-x64-portable.exe"
Copy-Item $exe $portable -Force

# --- Checksums and report ----------------------------------------------------
$lines = foreach ($f in @($setup, $portable)) {
    '{0}  {1}' -f (Get-FileHash -Algorithm SHA256 $f).Hash.ToLower(), (Split-Path -Leaf $f)
}
[IO.File]::WriteAllLines((Join-Path $root 'dist\SHA256SUMS.txt'), [string[]]$lines)

Write-Host ''
foreach ($f in @($setup, $portable)) {
    $sig = Get-AuthenticodeSignature $f
    $state = if ($sig.Status -eq 'Valid') { "signed by $($sig.SignerCertificate.GetNameInfo('SimpleName', $false))" } else { "signature: $($sig.Status)" }
    Write-Host ('{0,-36} {1,8:N1} MB   {2}' -f (Split-Path -Leaf $f), ((Get-Item $f).Length / 1MB), $state)
}
Write-Host 'dist\SHA256SUMS.txt'
