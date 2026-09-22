[CmdletBinding()]
param(
    [switch]$WaitForExit
)

$ErrorActionPreference = "Stop"
$projectRoot = Split-Path -Parent $PSScriptRoot
$appPath = Join-Path $projectRoot "src-tauri\target\release\vlm_screenshot_action.exe"
$staleCheck = Join-Path $PSScriptRoot "release-stale.ps1"

& (Join-Path $PSScriptRoot "cleanup-cache.ps1") -Quiet

$needsBuild = -not (Test-Path -LiteralPath $appPath -PathType Leaf)
if (-not $needsBuild) {
    & $staleCheck -AppPath $appPath
    $needsBuild = $LASTEXITCODE -ne 0
}

if ($needsBuild) {
    Write-Output "[startup] Sources changed or no release exists; rebuilding..."
    $running = Get-Process -Name "vlm_screenshot_action" -ErrorAction SilentlyContinue
    foreach ($process in $running) {
        try {
            if ($process.Path -eq $appPath) {
                Stop-Process -Id $process.Id -Force
                $process.WaitForExit(5000)
            }
        }
        catch {
            throw "Could not stop the old release before rebuilding: $($_.Exception.Message)"
        }
    }
    & (Join-Path $PSScriptRoot "build-release.ps1")
}

Write-Output "[startup] Release executable: $appPath"
& (Join-Path $PSScriptRoot "launch-app.ps1") -AppPath $appPath -WaitForExit:$WaitForExit
