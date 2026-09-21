<#
.SYNOPSIS
Download and verify the inro binary without installing or running it.
.PARAMETER Version
Release version, with an optional v prefix; defaults to latest.
.PARAMETER InstallDir
Output directory; defaults to the current directory. Existing files are not replaced.
#>
[CmdletBinding()]
param(
    [string]$Version = $env:INRO_VERSION,
    [Alias("To")]
    [string]$InstallDir = $env:INRO_INSTALL_DIR
)

$ErrorActionPreference = "Stop"
Set-StrictMode -Version Latest

if ([string]::IsNullOrWhiteSpace($Version)) {
    $Version = "latest"
}
if ([string]::IsNullOrWhiteSpace($InstallDir)) {
    $InstallDir = (Get-Location).Path
}

if ($env:OS -ne "Windows_NT") {
    throw "install.ps1 supports Windows only; use install.sh on Linux or macOS"
}
$Architecture = if ($env:PROCESSOR_ARCHITEW6432) {
    $env:PROCESSOR_ARCHITEW6432
}
else {
    $env:PROCESSOR_ARCHITECTURE
}
if ($Architecture -ine "AMD64") {
    throw "Windows releases currently require x86_64"
}

$InstallDir = $ExecutionContext.SessionState.Path.GetUnresolvedProviderPathFromPSPath($InstallDir)
$TargetPath = Join-Path $InstallDir "inro.exe"
if ($null -ne (Get-Item -LiteralPath $TargetPath -Force -ErrorAction SilentlyContinue)) {
    throw "$TargetPath already exists; choose another directory with -InstallDir"
}

$Asset = "inro-windows-x86_64-msvc.zip"
$ReleasesUrl = if ($env:INRO_RELEASES_URL) {
    $env:INRO_RELEASES_URL.TrimEnd("/")
}
else {
    "https://github.com/Yangmoooo/inro/releases"
}

if ($Version -eq "latest") {
    $ReleaseUrl = "$ReleasesUrl/latest/download"
    $VersionLabel = "latest"
}
else {
    if ($Version -notmatch "^v?[0-9][0-9A-Za-z.+-]*$") {
        throw "invalid version: $Version"
    }
    $Version = $Version -replace "^v", ""
    $ReleaseUrl = "$ReleasesUrl/download/v$Version"
    $VersionLabel = "v$Version"
}

function Get-InroFile {
    param(
        [Parameter(Mandatory)] [string]$Uri,
        [Parameter(Mandatory)] [string]$OutFile
    )

    Invoke-WebRequest -UseBasicParsing -Uri $Uri -OutFile $OutFile
}

$TempDir = Join-Path ([IO.Path]::GetTempPath()) ("inro-install-" + [guid]::NewGuid())
$StagedBinary = $null

try {
    New-Item -ItemType Directory -Force -Path $TempDir | Out-Null
    $Archive = Join-Path $TempDir $Asset
    $Checksums = Join-Path $TempDir "SHA256SUMS"
    $ExtractDir = Join-Path $TempDir "extracted"

    Write-Host "Downloading inro $VersionLabel..."
    Get-InroFile -Uri "$ReleaseUrl/$Asset" -OutFile $Archive
    Get-InroFile -Uri "$ReleaseUrl/SHA256SUMS" -OutFile $Checksums

    $Expected = $null
    foreach ($Line in Get-Content $Checksums) {
        $Fields = $Line.Trim() -split "\s+", 2
        if ($Fields.Count -eq 2 -and $Fields[1].TrimStart("*") -ceq $Asset) {
            $Expected = $Fields[0].ToLowerInvariant()
            break
        }
    }
    if ($null -eq $Expected -or $Expected -notmatch "^[0-9a-f]{64}$") {
        throw "SHA256SUMS has no valid entry for $Asset"
    }

    $Actual = (Get-FileHash -Algorithm SHA256 -Path $Archive).Hash.ToLowerInvariant()
    if ($Actual -cne $Expected) {
        throw "checksum verification failed for $Asset"
    }

    Expand-Archive -Path $Archive -DestinationPath $ExtractDir
    $ExtractedBinary = Join-Path $ExtractDir "inro.exe"
    if (-not (Test-Path -PathType Leaf $ExtractedBinary)) {
        throw "release archive does not contain inro.exe"
    }

    [IO.Directory]::CreateDirectory($InstallDir) | Out-Null
    $StagedBinary = Join-Path $InstallDir (".inro-download-" + [guid]::NewGuid() + ".exe")
    [IO.File]::Copy($ExtractedBinary, $StagedBinary, $false)
    # The two-argument Move overload refuses an existing destination, including links.
    [IO.File]::Move($StagedBinary, $TargetPath)
    $StagedBinary = $null

    $CommandPath = $TargetPath.Replace("'", "''")
    Write-Host "Downloaded and verified: $TargetPath"
    Write-Host ""
    Write-Host "Run directly:"
    Write-Host "  & '$CommandPath' --help"
    Write-Host ""
    Write-Host "To let inro manage itself:"
    Write-Host "  & '$CommandPath' source update"
    Write-Host "  & '$CommandPath' install inro"
    Write-Host ""
    Write-Host "Keep this downloaded copy outside your configured bin_dir (default ~/.local/bin) for self-management."
    Write-Host "After confirming the managed inro works, you can remove this downloaded copy."
    Write-Host "Alternatively, move it into a directory on PATH for manual use."
}
finally {
    if ($null -ne $StagedBinary) {
        Remove-Item -LiteralPath $StagedBinary -Force -ErrorAction SilentlyContinue
    }
    Remove-Item -Recurse -Force -ErrorAction SilentlyContinue $TempDir
}
