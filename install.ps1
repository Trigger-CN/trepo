$ErrorActionPreference = "Stop"
$repository = "Trigger-CN/trepo"
$baseUrl = "https://github.com/$repository/releases/latest/download"
$installDir = if ($env:TREPO_INSTALL_DIR) { $env:TREPO_INSTALL_DIR } else { Join-Path $env:LOCALAPPDATA "Programs\trepo\bin" }

if ($env:PROCESSOR_ARCHITECTURE -ne "AMD64") { throw "trepo only publishes a Windows x86_64 asset." }
$temp = Join-Path ([IO.Path]::GetTempPath()) ("trepo-install-" + [Guid]::NewGuid())
New-Item -ItemType Directory -Path $temp | Out-Null
try {
    $sumsPath = Join-Path $temp "SHA256SUMS"
    Invoke-WebRequest -UseBasicParsing "$baseUrl/SHA256SUMS" -OutFile $sumsPath
    $assetMatches = @(Get-Content $sumsPath | ForEach-Object {
        if ($_ -match '^([0-9A-Fa-f]{64})\s+\*?(?:\./)?(trepo-v[^/\s]+-windows-x86_64\.zip)$') {
            [PSCustomObject]@{ Hash = $Matches[1].ToLowerInvariant(); Name = $Matches[2] }
        }
    })
    if ($assetMatches.Count -ne 1) { throw "Could not select one windows-x86_64 asset from SHA256SUMS." }
    $asset = $assetMatches[0]
    $archivePath = Join-Path $temp $asset.Name
    Invoke-WebRequest -UseBasicParsing "$baseUrl/$($asset.Name)" -OutFile $archivePath
    $actual = (Get-FileHash -Algorithm SHA256 $archivePath).Hash.ToLowerInvariant()
    if ($actual -ne $asset.Hash) { throw "Checksum mismatch for $($asset.Name)." }
    Expand-Archive -LiteralPath $archivePath -DestinationPath $temp
    $package = [IO.Path]::GetFileNameWithoutExtension($asset.Name)
    $binary = Join-Path $temp "$package\trepo.exe"
    if (-not (Test-Path -LiteralPath $binary -PathType Leaf)) { throw "Archive does not contain $package\trepo.exe." }
    New-Item -ItemType Directory -Force -Path $installDir | Out-Null
    Copy-Item -LiteralPath $binary -Destination (Join-Path $installDir "trepo.exe") -Force
    $userPath = [Environment]::GetEnvironmentVariable("Path", "User")
    $parts = @($userPath -split ';' | Where-Object { $_ })
    if ($parts -notcontains $installDir) {
        [Environment]::SetEnvironmentVariable("Path", (($parts + $installDir) -join ';'), "User")
        Write-Host "Added $installDir to the user PATH. Open a new terminal to use it."
    }
    Write-Host "Installed trepo to $installDir\trepo.exe"
} finally {
    Remove-Item -LiteralPath $temp -Recurse -Force -ErrorAction SilentlyContinue
}
