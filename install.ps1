# Voussoir installer for Windows.
#
# Installs the toolchain dependencies (Git, Bun, Rust, WebView2 check), builds
# the CLI from source, and puts `voussoir` on PATH for the current user.
#
#   powershell -ExecutionPolicy Bypass -File install.ps1 [-Prefix DIR] [-Repo URL] [-NoDeps]
#
# Every dependency step is skipped when the tool is already present, so
# re-running on a provisioned machine touches nothing but the build.

[CmdletBinding()]
param(
  [string]$Prefix = (Join-Path $env:LOCALAPPDATA "voussoir"),
  [string]$Repo = "https://github.com/smegasberla/voussoir.git",
  [switch]$NoDeps
)

$ErrorActionPreference = "Stop"

function Fail([string]$Message) {
  # throw, not exit: the script is also run via `irm ... | iex`, where an
  # exit would take the user's whole terminal session with it.
  throw "voussoir-install: error: $Message"
}

function Refresh-Path {
  $env:Path = [Environment]::GetEnvironmentVariable("Path", "Machine") + ";" +
              [Environment]::GetEnvironmentVariable("Path", "User")
}

function Test-Command([string]$Name) {
  return [bool](Get-Command $Name -ErrorAction SilentlyContinue)
}

# --- source detection first: Git is only needed to clone ---------------------

$scriptDir = $null
if ($MyInvocation.MyCommand.Path) {
  $scriptDir = Split-Path -Parent $MyInvocation.MyCommand.Path
}
$src = $null
if ($scriptDir -and (Test-Path (Join-Path $scriptDir "cli\package.json")) -and
    (Test-Path (Join-Path $scriptDir "Cargo.toml"))) {
  $src = $scriptDir
  Write-Host "voussoir-install: building from the checkout at $src"
}

# --- dependencies -----------------------------------------------------------

if (-not $NoDeps) {
  if (-not $src -and -not (Test-Command "git")) {
    if (Test-Command "winget") {
      Write-Host "voussoir-install: installing Git"
      winget install --id Git.Git -e --source winget --accept-package-agreements --accept-source-agreements | Out-Host
      Refresh-Path
    }
    if (-not (Test-Command "git")) { Fail "git is missing and winget could not install it; install Git first" }
  }

  if (-not (Test-Command "bun")) {
    Write-Host "voussoir-install: installing Bun"
    if (Test-Command "winget") {
      winget install --id Oven-sh.Bun -e --source winget --accept-package-agreements --accept-source-agreements | Out-Host
      Refresh-Path
    } else {
      irm https://bun.sh/install.ps1 | iex
      Refresh-Path
    }
    if (-not (Test-Command "bun")) { Fail "Bun did not land on PATH; open a new terminal and re-run" }
  }

  if (-not (Test-Command "cargo")) {
    Write-Host "voussoir-install: installing Rust"
    if (-not (Test-Command "winget")) {
      Fail "cargo is missing and winget is not available; install Rust (rustup) first"
    }
    winget install --id Rustlang.Rustup -e --source winget --accept-package-agreements --accept-source-agreements | Out-Host
    Refresh-Path
    if (-not (Test-Command "cargo") -and (Test-Command "rustup-init")) {
      rustup-init -y --profile minimal | Out-Host
      Refresh-Path
    }
    if (-not (Test-Command "cargo")) { Fail "cargo is still missing; run rustup-init from a new terminal" }
  }
}

# The shell links against the WebView2 runtime; it ships with Windows 10/11
# but not everywhere. A file check, no download: the app owner installs it.
$pf86 = ${env:ProgramFiles(x86)}
if ($pf86) {
  $webView2 = Join-Path $pf86 "Microsoft\EdgeWebView\Application"
  if (-not (Test-Path $webView2)) {
    Write-Warning "WebView2 runtime not found; install it with: winget install Microsoft.EdgeWebView2Runtime"
  }
}

# --- clone when not run from a checkout -------------------------------------

if (-not $src) {
  $src = Join-Path $env:LOCALAPPDATA "voussoir\src"
  if (Test-Path (Join-Path $src ".git")) {
    Write-Host "voussoir-install: updating $src"
    git -C $src pull --ff-only
    if ($LASTEXITCODE -ne 0) { Write-Warning "could not update; building what is there" }
  } else {
    Write-Host "voussoir-install: cloning $Repo to $src"
    New-Item -ItemType Directory -Force -Path (Split-Path -Parent $src) | Out-Null
    git clone --depth 1 $Repo $src
    if ($LASTEXITCODE -ne 0) { Fail "clone failed; pass -Repo with a reachable URL" }
  }
}

# --- build ------------------------------------------------------------------

Write-Host "voussoir-install: building the CLI"
Push-Location (Join-Path $src "cli")
try {
  bun install
  if ($LASTEXITCODE -ne 0) { Fail "bun install failed" }
  bun run build
  if ($LASTEXITCODE -ne 0) { Fail "CLI build failed" }
} finally {
  Pop-Location
}

$built = Join-Path $src "cli\dist\voussoir.exe"
if (-not (Test-Path $built)) { $built = Join-Path $src "cli\dist\voussoir" }
if (-not (Test-Path $built)) { Fail "build finished but dist\voussoir(.exe) is missing" }

# --- install ----------------------------------------------------------------

$binDir = Join-Path $Prefix "bin"
New-Item -ItemType Directory -Force -Path $binDir | Out-Null
Copy-Item $built (Join-Path $binDir "voussoir.exe") -Force

# `voussoir update` rebuilds the source rather than downloading a release, so
# record which checkout this binary came from.
$share = Join-Path $env:LOCALAPPDATA "voussoir"
New-Item -ItemType Directory -Force -Path $share | Out-Null
Set-Content -Path (Join-Path $share "source") -Value $src -NoNewline
Write-Host "voussoir-install: recorded the source checkout in $share\source"

# Record the commit too. `update` compares it against the checkout's HEAD and
# only rebuilds when `cli\` changed, so an install with no stamp always looks
# stale; this is what makes "already current" true on Windows.
$commit = git -C $src rev-parse HEAD 2>$null
if ($LASTEXITCODE -eq 0 -and $commit) {
  Set-Content -Path (Join-Path $share "built") -Value $commit -NoNewline
  Write-Host "voussoir-install: recorded the built commit in $share\built"
} else {
  Remove-Item (Join-Path $share "built") -ErrorAction SilentlyContinue
  Write-Warning "no git commit recorded; the next update will rebuild once"
}
Write-Host "voussoir-install: installed $(Join-Path $binDir 'voussoir.exe')"

$userPath = [Environment]::GetEnvironmentVariable("Path", "User")
$inUserPath = $userPath -and ($userPath -like "*$binDir*")
if (-not $inUserPath) {
  $newPath = if ($userPath) { "$userPath;$binDir" } else { $binDir }
  [Environment]::SetEnvironmentVariable("Path", $newPath, "User")
  Write-Host "voussoir-install: added $binDir to your user PATH (new terminals pick it up)"
}
$env:Path = "$env:Path;$binDir"

# --- verify -----------------------------------------------------------------

Write-Host "voussoir-install: verifying the installed binary"
& (Join-Path $binDir "voussoir.exe") help *> $null
if ($LASTEXITCODE -ne 0) { Fail "the installed voussoir failed to run" }

Write-Host "voussoir-install: done. next: voussoir init myapp; cd myapp; voussoir dev   (voussoir doctor checks a project)"
