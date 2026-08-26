[CmdletBinding()]
param(
    [string]$Version = "latest"
)

$ErrorActionPreference = "Stop"
$repository = if ($env:SWMCTL_REPOSITORY) { $env:SWMCTL_REPOSITORY } else { "Saniee/swmctl" }

if ($Version -eq "latest") {
    $release = Invoke-RestMethod -Uri "https://api.github.com/repos/$repository/releases/latest"
    $tag = $release.tag_name
} else {
    $tag = $Version
}

$asset = "swmctl-x86_64-pc-windows-msvc.exe"
$baseUrl = "https://github.com/$repository/releases/download/$tag"
$tempDir = Join-Path ([System.IO.Path]::GetTempPath()) ([System.Guid]::NewGuid().ToString())
New-Item -ItemType Directory -Path $tempDir | Out-Null
try {
    $binaryPath = Join-Path $tempDir $asset
    $checksumsPath = Join-Path $tempDir "checksums.txt"
    Invoke-WebRequest -Uri "$baseUrl/$asset" -OutFile $binaryPath
    Invoke-WebRequest -Uri "$baseUrl/checksums.txt" -OutFile $checksumsPath

    $expected = ((Get-Content $checksumsPath | Where-Object { $_ -match "^([0-9a-fA-F]+)  $([regex]::Escape($asset))$" }) -split "\s+")[0]
    $actual = (Get-FileHash -Algorithm SHA256 -Path $binaryPath).Hash
    if ([string]::IsNullOrWhiteSpace($expected) -or $expected.ToUpperInvariant() -ne $actual.ToUpperInvariant()) {
        throw "Checksum verification failed."
    }

    $installDir = if ($env:SWMCTL_INSTALL_DIR) { $env:SWMCTL_INSTALL_DIR } else { Join-Path $env:LOCALAPPDATA "swmctl\bin" }
    New-Item -ItemType Directory -Force -Path $installDir | Out-Null
    Copy-Item $binaryPath (Join-Path $installDir "swmctl.exe") -Force
    Write-Output "Installed swmctl $tag to $(Join-Path $installDir 'swmctl.exe')"

    $onPath = ($env:PATH -split ';' | Where-Object { $_.TrimEnd('\') -eq $installDir.TrimEnd('\') })
    if (-not $onPath) {
        Write-Output "Add this directory to PATH before running swmctl:"
        Write-Output "  setx PATH `"$installDir;`$env:PATH`""
    }
} finally {
    Remove-Item $tempDir -Recurse -Force -ErrorAction SilentlyContinue
}
