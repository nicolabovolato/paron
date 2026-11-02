# Installs the newest release of Paron. Run it in a PowerShell as administrator.
#
#   irm https://paron.bovolato.dev/install.ps1 | iex
#
# PARON_VERSION       install this version, for example 0.1.0
# PARON_INSTALL_DIR   install into this directory, the default is C:\Program Files\Paron

# `iex` runs the script in the scope of the caller: the block keeps its variables out of it.
& {
  $ErrorActionPreference = 'Stop'
  $ProgressPreference = 'SilentlyContinue'

  $repository = 'nicolabovolato/paron'
  # A 32-bit PowerShell sees `C:\Program Files (x86)` as ProgramFiles. ProgramW6432 is the 64-bit folder.
  $programFiles = if ($env:ProgramW6432) { $env:ProgramW6432 } else { $env:ProgramFiles }
  $directory = if ($env:PARON_INSTALL_DIR) { $env:PARON_INSTALL_DIR } else { Join-Path $programFiles 'Paron' }

  # `exit` would close the PowerShell that runs `iex`.
  function Fail($message) {
    throw "error: $message"
  }

  $runtime = [object].Assembly.GetType('System.Runtime.InteropServices.RuntimeInformation')
  $architecture = if ($runtime) { $runtime::OSArchitecture } elseif ([Environment]::Is64BitOperatingSystem) { 'X64' }
  $processor = switch ($architecture) {
    'X64' { 'x86_64' }
    'Arm64' { 'aarch64' }
    default { Fail "Paron has no release for $_" }
  }

  try {
    $releases = Invoke-RestMethod "https://api.github.com/repos/$repository/releases?per_page=100"
  } catch {
    Fail 'could not get the list of releases from GitHub'
  }

  $version = $env:PARON_VERSION
  if (-not $version) {
    $version = $releases.tag_name |
      Where-Object { $_ -like 'paron-cli-v*' } |
      Select-Object -First 1 |
      ForEach-Object { $_ -replace '^paron-cli-v', '' }
    if (-not $version) { Fail 'GitHub lists no release of Paron' }
  }

  $archive = "paron-$version-$processor-pc-windows-msvc.zip"

  # GitHub records a SHA-256 digest for each release file.
  $asset = $releases |
    Where-Object { $_.tag_name -eq "paron-cli-v$version" } |
    ForEach-Object { $_.assets } |
    Where-Object { $_.name -eq $archive } |
    Select-Object -First 1
  if (-not $asset -or $asset.digest -notlike 'sha256:*') { Fail "GitHub lists no file $archive" }
  $expected = $asset.digest -replace '^sha256:', ''

  $work = Join-Path ([System.IO.Path]::GetTempPath()) ([System.IO.Path]::GetRandomFileName())
  New-Item -ItemType Directory -Path $work | Out-Null

  try {
    Write-Host "downloading $archive"
    try {
      Invoke-WebRequest -UseBasicParsing -Uri $asset.browser_download_url -OutFile (Join-Path $work $archive)
    } catch {
      Fail "could not download $archive"
    }

    $actual = (Get-FileHash -Algorithm SHA256 (Join-Path $work $archive)).Hash.ToLowerInvariant()
    if ($actual -ne $expected) { Fail "the SHA-256 of $archive is $actual, and GitHub records $expected" }

    Expand-Archive -Path (Join-Path $work $archive) -DestinationPath $work

    try {
      New-Item -ItemType Directory -Force -Path $directory | Out-Null
      Copy-Item -Force (Join-Path $work 'paron.exe') (Join-Path $directory 'paron.exe')
    } catch {
      Fail "cannot write to ${directory}: $($_.Exception.Message)"
    }
  } finally {
    Remove-Item -Recurse -Force $work
  }

  Write-Host "installed $(& (Join-Path $directory 'paron.exe') --version)"
  Write-Host "at $(Join-Path $directory 'paron.exe')"

  # Windows puts no program directory in PATH, so the installer does it.
  $machinePath = [Environment]::GetEnvironmentVariable('Path', 'Machine')
  if (($machinePath -split ';') -notcontains $directory) {
    try {
      [Environment]::SetEnvironmentVariable('Path', "$machinePath;$directory", 'Machine')
      $env:Path = "$env:Path;$directory"
      Write-Host "added $directory to PATH"
    } catch {
      Write-Host "note: $directory is not in your PATH"
    }
  }
}
