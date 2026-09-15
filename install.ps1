<#
.SYNOPSIS
    Install git-branch-manager on Windows.

.DESCRIPTION
    Downloads the newest release, verifies its checksum when the release
    publishes one, and installs the binary. macOS and Linux users want
    install.sh instead.

.EXAMPLE
    irm https://raw.githubusercontent.com/bruno-brant/rust-git-branch-manager/main/install.ps1 | iex

.EXAMPLE
    .\install.ps1 -Version v0.2.1 -Dir C:\tools
#>
#Requires -Version 5.1
[CmdletBinding()]
param(
    [string] $Version,
    [string] $Dir = (Join-Path $env:LOCALAPPDATA 'Programs\git-branch-manager')
)

$ErrorActionPreference = 'Stop'
# Windows PowerShell 5.1 on older systems still defaults to TLS 1.0, which
# GitHub refuses; and its progress bar makes Invoke-WebRequest crawl.
[Net.ServicePointManager]::SecurityProtocol = [Net.SecurityProtocolType]::Tls12
$ProgressPreference = 'SilentlyContinue'
$Repo = 'bruno-brant/rust-git-branch-manager'
$Bin = 'git-branch-manager.exe'

# Only an x86-64 build is published. Windows on ARM runs it under emulation, so
# this is a warning rather than a refusal.
if ($env:PROCESSOR_ARCHITECTURE -eq 'ARM64') {
    Write-Warning 'No native ARM64 build; the x86-64 binary will run under emulation.'
}

if (-not $Version) {
    # The API returns the tag directly. Resolving the /releases/latest redirect
    # would avoid the API's rate limit, but reading the final URL differs
    # between PowerShell 5.1 and 7, and this is identical in both.
    $Version = (Invoke-RestMethod "https://api.github.com/repos/$Repo/releases/latest").tag_name
}

$Asset = "git-branch-manager-$Version-windows-x86_64"
$Base = "https://github.com/$Repo/releases/download/$Version"
Write-Host "Installing git-branch-manager $Version into $Dir"

$tmp = (New-Item -ItemType Directory -Path (Join-Path $env:TEMP ("gbm-" + [guid]::NewGuid()))).FullName
try {
    $zip = Join-Path $tmp "$Asset.zip"
    Invoke-WebRequest -Uri "$Base/$Asset.zip" -OutFile $zip

    # Releases before v0.2.1 have no SHA256SUMS. Verify whenever it is
    # published, and say plainly when it is not.
    $sums = Join-Path $tmp 'SHA256SUMS'
    $haveSums = $true
    try { Invoke-WebRequest -Uri "$Base/SHA256SUMS" -OutFile $sums } catch { $haveSums = $false }

    if ($haveSums) {
        $line = Get-Content $sums |
            Where-Object { $_ -match "\s\*?$([regex]::Escape("$Asset.zip"))$" } |
            Select-Object -First 1
        if (-not $line) { throw "SHA256SUMS has no entry for $Asset.zip" }
        $expected = ($line -split '\s+')[0]
        $actual = (Get-FileHash -Algorithm SHA256 -Path $zip).Hash.ToLower()
        if ($expected.ToLower() -ne $actual) {
            throw "checksum mismatch for $Asset.zip`n  expected $expected`n  actual   $actual"
        }
        Write-Host 'Checksum verified.'
    }
    else {
        Write-Warning 'This release publishes no SHA256SUMS; the download was not verified.'
    }

    Expand-Archive -Path $zip -DestinationPath $tmp -Force
    $exe = Join-Path $tmp "$Asset\$Bin"
    if (-not (Test-Path $exe)) { throw "archive did not contain $Bin where expected" }

    New-Item -ItemType Directory -Path $Dir -Force | Out-Null
    Copy-Item $exe (Join-Path $Dir $Bin) -Force
    Write-Host "Installed $(Join-Path $Dir $Bin)"
}
finally {
    Remove-Item $tmp -Recurse -Force -ErrorAction SilentlyContinue
}

$onPath = ($env:PATH -split ';') -contains $Dir
if ($onPath) {
    Write-Host 'Run it from inside any git repository: git-branch-manager'
}
else {
    Write-Host ""
    Write-Host "$Dir is not on your PATH. Add it for future sessions with:"
    Write-Host "  [Environment]::SetEnvironmentVariable('PATH', `"`$env:PATH;$Dir`", 'User')"
}
