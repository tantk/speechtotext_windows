# Script to generate SHA256 checksums for model files
# Usage: .\generate_checksums.ps1 -ModelDir "C:\path\to\models" -OutputFile "checksums.json"

param(
    [Parameter(Mandatory=$true)]
    [string]$ModelDir,
    
    [Parameter(Mandatory=$false)]
    [string]$OutputFile = "checksums.json"
)

$ErrorActionPreference = "Stop"

function Get-FileChecksum {
    param([string]$FilePath)
    $hash = Get-FileHash -Path $FilePath -Algorithm SHA256
    return $hash.Hash
}

$checksums = @{}

if (-not (Test-Path $ModelDir)) {
    Write-Error "Model directory not found: $ModelDir"
    exit 1
}

Write-Host "Generating checksums for files in: $ModelDir"

Get-ChildItem -Path $ModelDir -File | ForEach-Object {
    $fileName = $_.Name
    $filePath = $_.FullName
    
    Write-Host "  Computing checksum for: $fileName"
    $hash = Get-FileChecksum -FilePath $filePath
    $checksums[$fileName] = "sha256:$hash"
    
    Write-Host "    -> $hash"
}

$output = @{
    checksums = $checksums
} | ConvertTo-Json -Depth 10

$output | Out-File -FilePath $OutputFile -Encoding UTF8
Write-Host "`nChecksums written to: $OutputFile"
Write-Host "`nCopy the 'checksums' section into your manifest.json file."
