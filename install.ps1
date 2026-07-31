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
    $InstallDir = Join-Path $HOME ".local/bin"
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

    New-Item -ItemType Directory -Force -Path $InstallDir | Out-Null
    $StagedBinary = Join-Path $InstallDir ".inro-install-$PID.exe"
    Copy-Item -Path $ExtractedBinary -Destination $StagedBinary -Force
    Move-Item -Path $StagedBinary -Destination (Join-Path $InstallDir "inro.exe") -Force
    $StagedBinary = $null

    Write-Host "Installed inro to $(Join-Path $InstallDir 'inro.exe')"
    $NormalizedInstallDir = $InstallDir.TrimEnd([char[]]"\/")
    $OnPath = ($env:PATH -split [IO.Path]::PathSeparator) |
        Where-Object { $_.TrimEnd([char[]]"\/") -ieq $NormalizedInstallDir }
    if (-not $OnPath) {
        Write-Host "Add $InstallDir to PATH to run inro."
    }
}
finally {
    if ($null -ne $StagedBinary) {
        Remove-Item -Force -ErrorAction SilentlyContinue $StagedBinary
    }
    Remove-Item -Recurse -Force -ErrorAction SilentlyContinue $TempDir
}
