$ErrorActionPreference = "Stop"
Set-StrictMode -Version Latest

$RepoRoot = Split-Path -Parent $PSScriptRoot
$TempDir = Join-Path ([IO.Path]::GetTempPath()) ("inro-installer-test-" + [guid]::NewGuid())
$ServerProcess = $null
$PreviousReleasesUrl = $env:INRO_RELEASES_URL

try {
    $Asset = "inro-windows-x86_64-msvc.zip"
    $ReleaseDir = Join-Path $TempDir "server/releases/latest/download"
    $PayloadDir = Join-Path $TempDir "payload"
    $InstallDir = Join-Path $TempDir "install"
    New-Item -ItemType Directory -Force -Path $ReleaseDir, $PayloadDir | Out-Null

    Set-Content -Path (Join-Path $PayloadDir "inro.exe") -Value "fake-windows-inro" -NoNewline
    $Archive = Join-Path $ReleaseDir $Asset
    Compress-Archive -Path (Join-Path $PayloadDir "*") -DestinationPath $Archive
    $Checksum = (Get-FileHash -Algorithm SHA256 -Path $Archive).Hash.ToLowerInvariant()
    Set-Content -Path (Join-Path $ReleaseDir "SHA256SUMS") -Value "$Checksum  $Asset"

    $Listener = [Net.Sockets.TcpListener]::new([Net.IPAddress]::Loopback, 0)
    $Listener.Start()
    $Port = ([Net.IPEndPoint]$Listener.LocalEndpoint).Port
    $Listener.Stop()

    $ServerProcess = Start-Process python -ArgumentList @(
        "-m", "http.server", "$Port", "--bind", "127.0.0.1", "--directory",
        (Join-Path $TempDir "server")
    ) -PassThru -WindowStyle Hidden

    $ChecksumUrl = "http://127.0.0.1:$Port/releases/latest/download/SHA256SUMS"
    $Started = $false
    foreach ($Attempt in 1..20) {
        try {
            Invoke-WebRequest -UseBasicParsing -Uri $ChecksumUrl | Out-Null
            $Started = $true
            break
        }
        catch {
            Start-Sleep -Milliseconds 100
        }
    }
    if (-not $Started) {
        throw "local release server did not start"
    }

    $env:INRO_RELEASES_URL = "http://127.0.0.1:$Port/releases"
    & (Join-Path $RepoRoot "install.ps1") -InstallDir $InstallDir

    $Installed = Join-Path $InstallDir "inro.exe"
    if (-not (Test-Path -PathType Leaf $Installed)) {
        throw "installer did not create inro.exe"
    }
    if ((Get-Content -Raw $Installed) -ne "fake-windows-inro") {
        throw "installed binary did not match the release payload"
    }

    Remove-Item $Archive
    Set-Content -Path (Join-Path $PayloadDir "inro.exe") -Value "updated-windows-inro" -NoNewline
    Compress-Archive -Path (Join-Path $PayloadDir "*") -DestinationPath $Archive
    $Checksum = (Get-FileHash -Algorithm SHA256 -Path $Archive).Hash.ToLowerInvariant()
    Set-Content -Path (Join-Path $ReleaseDir "SHA256SUMS") -Value "$Checksum  $Asset"

    & (Join-Path $RepoRoot "install.ps1") -InstallDir $InstallDir
    if ((Get-Content -Raw $Installed) -ne "updated-windows-inro") {
        throw "installer did not replace an existing inro.exe"
    }
}
finally {
    $env:INRO_RELEASES_URL = $PreviousReleasesUrl
    if ($null -ne $ServerProcess -and -not $ServerProcess.HasExited) {
        Stop-Process -Id $ServerProcess.Id -Force
    }
    Remove-Item -Recurse -Force -ErrorAction SilentlyContinue $TempDir
}
