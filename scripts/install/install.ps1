# Installs the latest published ArchGuard release on Windows.
#
#   irm https://raw.githubusercontent.com/svenhartmann/ArchGuard/main/scripts/install/install.ps1 | iex
#
# Environment:
#   ARCHGUARD_VERSION       version to install, e.g. 0.1.3 (default: latest release)
#   ARCHGUARD_INSTALL_DIR   target directory (default: %LOCALAPPDATA%\Programs\archguard)
#   ARCHGUARD_DOWNLOAD_URL  base URL with the release assets (default: GitHub release of that version)
$ErrorActionPreference = 'Stop'
$ProgressPreference = 'SilentlyContinue'

& {
    $repo = 'svenhartmann/ArchGuard'
    $installDir = if ($env:ARCHGUARD_INSTALL_DIR) { $env:ARCHGUARD_INSTALL_DIR } else { Join-Path $env:LOCALAPPDATA 'Programs\archguard' }
    [Net.ServicePointManager]::SecurityProtocol = [Net.ServicePointManager]::SecurityProtocol -bor [Net.SecurityProtocolType]::Tls12

    # Only x64 is built; Windows on ARM runs it through x64 emulation.
    $target = 'x86_64-pc-windows-msvc'
    if ($env:PROCESSOR_ARCHITECTURE -eq 'ARM64') { Write-Host 'Windows ARM64: installing the x64 build (runs under emulation).' }

    $version = $env:ARCHGUARD_VERSION
    if (-not $version) {
        # The API only lists published releases, never drafts or pre-releases.
        $version = (Invoke-RestMethod "https://api.github.com/repos/$repo/releases/latest" -UseBasicParsing).tag_name
    }
    $version = $version.TrimStart('v')
    if (-not $version) { throw 'archguard install: could not determine the latest release' }
    $base = if ($env:ARCHGUARD_DOWNLOAD_URL) { $env:ARCHGUARD_DOWNLOAD_URL } else { "https://github.com/$repo/releases/download/v$version" }

    $stem = "archguard-$version-$target"
    $archive = "$stem.zip"
    $tmp = Join-Path ([IO.Path]::GetTempPath()) ("archguard-" + [Guid]::NewGuid())
    New-Item -ItemType Directory -Path $tmp | Out-Null
    try {
        Write-Host "Downloading ArchGuard $version for $target ..."
        Invoke-WebRequest "$base/$archive" -OutFile (Join-Path $tmp $archive) -UseBasicParsing
        Invoke-WebRequest "$base/SHA256SUMS" -OutFile (Join-Path $tmp 'SHA256SUMS') -UseBasicParsing

        $line = Get-Content (Join-Path $tmp 'SHA256SUMS') | Where-Object { ($_ -split '\s+')[1] -eq $archive } | Select-Object -First 1
        if (-not $line) { throw "archguard install: no checksum for $archive in SHA256SUMS" }
        $expected = ($line -split '\s+')[0]
        $actual = (Get-FileHash (Join-Path $tmp $archive) -Algorithm SHA256).Hash
        if ($actual -ne $expected) { throw "archguard install: checksum mismatch for $archive" }

        Expand-Archive (Join-Path $tmp $archive) -DestinationPath (Join-Path $tmp 'x')
        $binary = Join-Path $tmp "x\$stem\archguard.exe"
        if (-not (Test-Path $binary)) { throw "archguard install: archive does not contain $stem\archguard.exe" }

        New-Item -ItemType Directory -Force -Path $installDir | Out-Null
        Copy-Item $binary (Join-Path $installDir 'archguard.exe') -Force
    } finally {
        Remove-Item $tmp -Recurse -Force -ErrorAction SilentlyContinue
    }

    Write-Host "Installed $installDir\archguard.exe"
    $userPath = [Environment]::GetEnvironmentVariable('Path', 'User')
    if (-not (($userPath -split ';') -contains $installDir)) {
        [Environment]::SetEnvironmentVariable('Path', (@($userPath, $installDir) | Where-Object { $_ }) -join ';', 'User')
        $env:Path = "$env:Path;$installDir"
        Write-Host "Added $installDir to your user PATH (new terminals pick it up)."
    }
    Write-Host 'Update later with: archguard update'
}
