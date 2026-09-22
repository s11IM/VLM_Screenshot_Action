[CmdletBinding()]
param(
    [string]$OutputDirectory = "",
    [switch]$IncludeSettings,
    [switch]$IncludeHistory,
    [switch]$PublicRelease,
    [switch]$ArchiveOnly,
    [switch]$SkipBuild
)

$ErrorActionPreference = "Stop"
$projectRoot = Split-Path -Parent $PSScriptRoot
if ($PublicRelease -and ($IncludeSettings -or $IncludeHistory)) {
    throw "Public releases cannot include settings or history. Use a separate private migration archive."
}
if ([string]::IsNullOrWhiteSpace($OutputDirectory)) {
    $OutputDirectory = Join-Path $projectRoot $(if ($PublicRelease) { "release\public" } else { "release" })
}
elseif (-not [IO.Path]::IsPathRooted($OutputDirectory)) {
    $OutputDirectory = Join-Path $projectRoot $OutputDirectory
}

$appPath = Join-Path $projectRoot "src-tauri\target\release\vlm_screenshot_action.exe"
$staleCheck = Join-Path $PSScriptRoot "release-stale.ps1"
$setupDirectory = Join-Path $projectRoot "src-tauri\target\release\bundle\nsis"
$setup = Get-ChildItem -LiteralPath $setupDirectory -Filter "*x64-setup.exe" -File -ErrorAction SilentlyContinue |
    Sort-Object LastWriteTimeUtc -Descending |
    Select-Object -First 1

$appCurrent = Test-Path -LiteralPath $appPath -PathType Leaf
if ($appCurrent) {
    & $staleCheck -AppPath $appPath
    $appCurrent = $LASTEXITCODE -eq 0
}
$bundleCurrent = $appCurrent -and $null -ne $setup -and
    $setup.LastWriteTimeUtc -ge (Get-Item -LiteralPath $appPath).LastWriteTimeUtc

if (-not $bundleCurrent) {
    if ($SkipBuild) {
        if (-not $appCurrent) {
            throw "The release executable is missing or stale, and -SkipBuild was specified."
        }
        throw "The NSIS installer is missing or stale, and -SkipBuild was specified."
    }
    else {
        & (Join-Path $PSScriptRoot "build-release.ps1") -Bundle
        $setup = Get-ChildItem -LiteralPath $setupDirectory -Filter "*x64-setup.exe" -File -ErrorAction Stop |
            Sort-Object LastWriteTimeUtc -Descending |
            Select-Object -First 1
    }
}

if (-not (Test-Path -LiteralPath $appPath -PathType Leaf) -or $null -eq $setup) {
    throw "Release packaging artifacts are incomplete."
}

if ($IncludeSettings -or $IncludeHistory) {
    $running = @(Get-Process -Name "vlm_screenshot_action", "vlm-screen-bridge", "game-vision-companion" -ErrorAction SilentlyContinue)
    if ($running.Count -gt 0) {
        throw "Close VLM_Screenshot_Action before packaging settings or history."
    }
}

$portableDirectory = Join-Path $OutputDirectory "VLM_Screenshot_Action-Portable-x64"
$zipPath = Join-Path $OutputDirectory "VLM_Screenshot_Action-Portable-x64.zip"
$setupPath = Join-Path $OutputDirectory "VLM_Screenshot_Action-Setup-x64.exe"

$null = New-Item -ItemType Directory -Path $OutputDirectory -Force
foreach ($oldArtifact in @($portableDirectory, $zipPath)) {
    if (Test-Path -LiteralPath $oldArtifact) {
        Remove-Item -LiteralPath $oldArtifact -Recurse -Force -ErrorAction Stop
    }
}
$null = New-Item -ItemType Directory -Path $portableDirectory -Force

Copy-Item -LiteralPath $appPath -Destination (Join-Path $portableDirectory "VLM_Screenshot_Action.exe") -Force
Copy-Item -LiteralPath (Join-Path $PSScriptRoot "launch-app.ps1") -Destination (Join-Path $portableDirectory "launch.ps1") -Force
Copy-Item -LiteralPath (Join-Path $PSScriptRoot "portable-start.cmd") -Destination (Join-Path $portableDirectory "start.cmd") -Force
Copy-Item -LiteralPath (Join-Path $projectRoot "LICENSE") -Destination (Join-Path $portableDirectory "LICENSE") -Force

if ($IncludeSettings -or $IncludeHistory) {
    $sourceProfile = Join-Path $env:LOCALAPPDATA "com.vlmscreenshotaction.app\EBWebView\Default"
    $dataDirectory = Join-Path $portableDirectory "user-data"
    $null = New-Item -ItemType Directory -Path $dataDirectory -Force
    if ($IncludeSettings) {
        $settingsPath = Join-Path $sourceProfile "Local Storage"
        if (-not (Test-Path -LiteralPath $settingsPath -PathType Container)) {
            throw "No saved settings were found at $settingsPath."
        }
        Copy-Item -LiteralPath $settingsPath -Destination (Join-Path $dataDirectory "Local Storage") -Recurse -Force
    }
    if ($IncludeHistory) {
        $historyPath = Join-Path $sourceProfile "IndexedDB"
        if (-not (Test-Path -LiteralPath $historyPath -PathType Container)) {
            throw "No saved conversation history was found at $historyPath."
        }
        Copy-Item -LiteralPath $historyPath -Destination (Join-Path $dataDirectory "IndexedDB") -Recurse -Force
    }
}

if ($PublicRelease -and (Test-Path -LiteralPath (Join-Path $portableDirectory "user-data"))) {
    throw "Refusing to publish a package containing user-data."
}

Compress-Archive -Path (Join-Path $portableDirectory "*") -DestinationPath $zipPath -CompressionLevel Optimal
Copy-Item -LiteralPath $setup.FullName -Destination $setupPath -Force

& (Join-Path $portableDirectory "launch.ps1") -AppPath (Join-Path $portableDirectory "VLM_Screenshot_Action.exe") -ValidateOnly

if ($ArchiveOnly) {
    Remove-Item -LiteralPath $portableDirectory -Recurse -Force
}

$zipSize = [Math]::Round((Get-Item -LiteralPath $zipPath).Length / 1MB, 2)
$setupSize = [Math]::Round((Get-Item -LiteralPath $setupPath).Length / 1MB, 2)
Write-Output "[package] Portable ZIP: $zipPath ($zipSize MB)"
Write-Output "[package] Windows installer: $setupPath ($setupSize MB)"
if ($IncludeSettings) {
    Write-Warning "The portable ZIP contains saved settings, including the API key. Protect the archive."
}
