[CmdletBinding()]
param(
    [switch]$Bundle
)

$ErrorActionPreference = "Stop"
$projectRoot = Split-Path -Parent $PSScriptRoot

function Get-RequiredCommand {
    param(
        [string]$Name,
        [string]$InstallHint
    )

    $command = Get-Command $Name -ErrorAction SilentlyContinue
    if ($null -eq $command) {
        throw "$Name was not found. $InstallHint"
    }
    return $command
}

function Import-VisualStudioEnvironment {
    $vcvarsCandidates = @()
    $vswhere = Join-Path ${env:ProgramFiles(x86)} "Microsoft Visual Studio\Installer\vswhere.exe"
    if (Test-Path -LiteralPath $vswhere -PathType Leaf) {
        $installationPath = & $vswhere -latest -products * -requires Microsoft.VisualStudio.Component.VC.Tools.x86.x64 -property installationPath
        if ($LASTEXITCODE -eq 0 -and -not [string]::IsNullOrWhiteSpace($installationPath)) {
            $vcvarsCandidates += Join-Path $installationPath.Trim() "VC\Auxiliary\Build\vcvars64.bat"
        }
    }

    foreach ($edition in @("BuildTools", "Community", "Professional", "Enterprise")) {
        $vcvarsCandidates += Join-Path ${env:ProgramFiles(x86)} "Microsoft Visual Studio\2022\$edition\VC\Auxiliary\Build\vcvars64.bat"
    }

    $vcvars = $vcvarsCandidates |
        Where-Object { Test-Path -LiteralPath $_ -PathType Leaf } |
        Select-Object -First 1
    if ([string]::IsNullOrWhiteSpace($vcvars)) {
        throw "Visual Studio 2022 C++ Build Tools were not found. Install the Desktop development with C++ workload."
    }

    $environmentCommand = 'call "' + $vcvars + '" >nul && set'
    $environmentLines = & $env:ComSpec /d /c $environmentCommand
    if ($LASTEXITCODE -ne 0) {
        throw "Failed to initialize the Visual Studio C++ build environment."
    }
    foreach ($line in $environmentLines) {
        if ($line -match "^([^=]+)=(.*)$") {
            [Environment]::SetEnvironmentVariable($matches[1], $matches[2], "Process")
        }
    }
}

$node = Get-RequiredCommand -Name "node.exe" -InstallHint "Install Node.js 24.12+ LTS (see .node-version)."
$npm = Get-RequiredCommand -Name "npm.cmd" -InstallHint "Install Node.js with npm."
$null = Get-RequiredCommand -Name "cargo.exe" -InstallHint "Install rustup with the MSVC toolchain; rust-toolchain.toml selects the compiler."

$nodeVersionText = (& $node.Source --version).Trim().TrimStart("v")
$nodeVersion = [Version]$nodeVersionText
$nodeSupported = $nodeVersion.Major -eq 24 -and $nodeVersion -ge [Version]"24.12.0"
if (-not $nodeSupported) {
    throw "Node.js $nodeVersion is outside the supported baseline. Install Node.js 24.12+ LTS (see .node-version)."
}

$dependencyStamp = Join-Path $projectRoot "node_modules\.package-lock.json"
$dependencyInputs = @(
    (Join-Path $projectRoot "package.json"),
    (Join-Path $projectRoot "package-lock.json")
)
$installDependencies = -not (Test-Path -LiteralPath $dependencyStamp -PathType Leaf)
if (-not $installDependencies) {
    $stampTime = (Get-Item -LiteralPath $dependencyStamp).LastWriteTimeUtc
    $installDependencies = @(
        $dependencyInputs |
            Where-Object { (Get-Item -LiteralPath $_).LastWriteTimeUtc -gt $stampTime }
    ).Count -gt 0
}

Push-Location $projectRoot
try {
    if ($installDependencies) {
        Write-Output "[build] Restoring npm dependencies with npm ci..."
        & $npm.Source ci --no-audit --no-fund
        if ($LASTEXITCODE -ne 0) {
            throw "npm ci failed with exit code $LASTEXITCODE."
        }
    }

    Import-VisualStudioEnvironment
    $tauriArguments = @("run", "tauri", "--", "build")
    if ($Bundle) {
        $tauriArguments += @("--bundles", "nsis")
    }
    else {
        $tauriArguments += "--no-bundle"
    }
    $tauriArguments += @("--", "--locked")

    Write-Output "[build] Building the Windows x64 release..."
    & $npm.Source @tauriArguments
    if ($LASTEXITCODE -ne 0) {
        throw "Tauri build failed with exit code $LASTEXITCODE."
    }
}
finally {
    Pop-Location
}

$appPath = Join-Path $projectRoot "src-tauri\target\release\vlm_screenshot_action.exe"
if (-not (Test-Path -LiteralPath $appPath -PathType Leaf)) {
    throw "The release executable was not created at $appPath."
}
Write-Output "[build] Release executable: $appPath"
