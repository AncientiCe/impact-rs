$ErrorActionPreference = "Stop"

$Repo = if ($env:IMPACT_REPO) { $env:IMPACT_REPO } else { "AncientiCe/impact-rs" }
$InstallDir = if ($env:IMPACT_INSTALL_DIR) { $env:IMPACT_INSTALL_DIR } else { Join-Path $env:LOCALAPPDATA "Programs\impact\bin" }
$TempDir = Join-Path ([System.IO.Path]::GetTempPath()) ("impact-" + [System.Guid]::NewGuid().ToString("N"))
New-Item -ItemType Directory -Force -Path $TempDir | Out-Null

function Get-Target {
    # PowerShell 5.1's .NET Framework may not expose RuntimeInformation.OSArchitecture,
    # so fall back to the PROCESSOR_ARCHITECTURE environment variable.
    $arch = $null
    try {
        $arch = [System.Runtime.InteropServices.RuntimeInformation]::OSArchitecture.ToString()
    } catch { }
    if (-not $arch) {
        $arch = if ($env:PROCESSOR_ARCHITEW6432) { $env:PROCESSOR_ARCHITEW6432 } else { $env:PROCESSOR_ARCHITECTURE }
    }
    switch -Regex ($arch) {
        "^(X64|AMD64)$" { return "x86_64-pc-windows-msvc" }
        default { throw "Unsupported architecture: '$arch' (only 64-bit x86 Windows binaries are shipped today; build from source with cargo install --path crates/impact-cli otherwise)" }
    }
}

try {
    $Target = Get-Target

    $VersionOverride = if ($env:IMPACT_VERSION) { $env:IMPACT_VERSION } else { $null }
    $LocalArchive = if ($env:IMPACT_LOCAL_ARCHIVE) { $env:IMPACT_LOCAL_ARCHIVE } else { $null }

    if ($VersionOverride -eq "local") {
        if (-not $LocalArchive) {
            throw "IMPACT_LOCAL_ARCHIVE is required when IMPACT_VERSION=local"
        }
        $Archive = $LocalArchive
    } else {
        if ($VersionOverride) {
            $Tag = $VersionOverride
        } else {
            $Release = Invoke-RestMethod -Uri "https://api.github.com/repos/$Repo/releases/latest"
            $Tag = $Release.tag_name
        }
        $Version = $Tag.TrimStart("v")
        $Asset = "impact-$Version-$Target.zip"
        $Archive = Join-Path $TempDir $Asset
        $Checksum = Join-Path $TempDir "impact-$Target.sha256"
        Invoke-WebRequest -Uri "https://github.com/$Repo/releases/download/$Tag/$Asset" -OutFile $Archive
        Invoke-WebRequest -Uri "https://github.com/$Repo/releases/download/$Tag/impact-$Target.sha256" -OutFile $Checksum

        $Expected = ((Get-Content $Checksum | Select-Object -First 1) -split "\s+")[0]
        $Actual = (Get-FileHash -Algorithm SHA256 $Archive).Hash.ToLowerInvariant()
        if ($Actual -ne $Expected.ToLowerInvariant()) {
            throw "Checksum mismatch for $Asset"
        }
    }

    New-Item -ItemType Directory -Force -Path $InstallDir | Out-Null
    Expand-Archive -Path $Archive -DestinationPath $TempDir -Force
    $Binary = Get-ChildItem -Path $TempDir -Recurse -Filter "impact.exe" | Select-Object -First 1
    if (-not $Binary) {
        throw "Archive did not contain impact.exe"
    }
    Copy-Item $Binary.FullName (Join-Path $InstallDir "impact.exe") -Force

    # Add the install dir to the *user* PATH in the registry (never use `setx PATH "...;%PATH%"`:
    # PowerShell does not expand %PATH%, and setx would overwrite the user PATH with a literal string).
    $UserPath = [Environment]::GetEnvironmentVariable("Path", "User")
    $UserParts = if ($UserPath) { ($UserPath -split ";") | Where-Object { $_ } } else { @() }
    if ($UserParts -notcontains $InstallDir) {
        $NewUserPath = if ($UserPath) { ($UserPath.TrimEnd(";") + ";" + $InstallDir) } else { $InstallDir }
        [Environment]::SetEnvironmentVariable("Path", $NewUserPath, "User")
        Write-Host "Added $InstallDir to your user PATH. Restart your terminal to pick it up."
    }
    if ((($env:PATH -split ";") | Where-Object { $_ }) -notcontains $InstallDir) {
        $env:PATH = "$InstallDir;$env:PATH"
    }

    Write-Host "impact installed to $InstallDir\impact.exe"
    Write-Host "Next: impact index <project>; impact query <file>"
    Write-Host "Or register the MCP server: claude mcp add impact -- impact mcp"
} finally {
    Remove-Item -Recurse -Force $TempDir -ErrorAction SilentlyContinue
}
