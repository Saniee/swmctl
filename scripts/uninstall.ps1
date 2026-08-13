[CmdletBinding()]
param()

$ErrorActionPreference = "Stop"
$installDir = if ($env:SWMCTL_INSTALL_DIR) { $env:SWMCTL_INSTALL_DIR } else { Join-Path $env:LOCALAPPDATA "swmctl\bin" }
$binary = Join-Path $installDir "swmctl.exe"
if (Test-Path -LiteralPath $binary) {
    Remove-Item -LiteralPath $binary -Force
    Write-Output "Removed $binary"
} else {
    Write-Output "swmctl is not installed at $binary"
}
