[CmdletBinding()]
param(
    [ValidateRange(1, 8760)]
    [int]$MaxAgeHours = 24,

    [switch]$Quiet,

    [switch]$ForMigration
)

$projectRoot = Split-Path -Parent $PSScriptRoot
$cutoffUtc = [DateTime]::UtcNow.AddHours(-$MaxAgeHours)
$removedBytes = [int64]0
$removedFiles = 0
$removedGroups = 0
$warnings = 0

function Write-CleanupMessage {
    param([string]$Message)

    if (-not $Quiet) {
        Write-Output "[cache-cleanup] $Message"
    }
}

function Get-DirectoryStats {
    param([string]$Path)

    $files = @(Get-ChildItem -LiteralPath $Path -Recurse -Force -File -ErrorAction Stop)
    $size = ($files | Measure-Object -Property Length -Sum).Sum
    if ($null -eq $size) {
        $size = 0
    }

    return [pscustomobject]@{
        Bytes = [int64]$size
        Files = $files.Count
    }
}

function Remove-ExpiredCacheGroup {
    param([string]$RelativePath)

    $path = Join-Path $projectRoot $RelativePath
    if (-not (Test-Path -LiteralPath $path -PathType Container)) {
        return
    }

    try {
        $files = @(Get-ChildItem -LiteralPath $path -Recurse -Force -File -ErrorAction Stop)
        if ($files.Count -eq 0) {
            Remove-Item -LiteralPath $path -Recurse -Force -ErrorAction Stop
            $script:removedGroups += 1
            Write-CleanupMessage "Removed empty cache group: $RelativePath"
            return
        }

        $containsExpiredFile = $false
        foreach ($file in $files) {
            if ($file.LastWriteTimeUtc -lt $cutoffUtc) {
                $containsExpiredFile = $true
                break
            }
        }

        if (-not $containsExpiredFile) {
            return
        }

        $stats = Get-DirectoryStats -Path $path
        Remove-Item -LiteralPath $path -Recurse -Force -ErrorAction Stop
        $script:removedBytes += $stats.Bytes
        $script:removedFiles += $stats.Files
        $script:removedGroups += 1
        Write-CleanupMessage "Removed cache group containing files older than $MaxAgeHours hours: $RelativePath"
    }
    catch {
        $script:warnings += 1
        Write-Warning "[cache-cleanup] Could not clean ${RelativePath}: $($_.Exception.Message)"
    }
}

function Remove-ExpiredFile {
    param([string]$RelativePath)

    $path = Join-Path $projectRoot $RelativePath
    if (-not (Test-Path -LiteralPath $path -PathType Leaf)) {
        return
    }

    try {
        $file = Get-Item -LiteralPath $path -Force -ErrorAction Stop
        if ($file.LastWriteTimeUtc -ge $cutoffUtc) {
            return
        }

        $length = $file.Length
        Remove-Item -LiteralPath $path -Force -ErrorAction Stop
        $script:removedBytes += $length
        $script:removedFiles += 1
        Write-CleanupMessage "Removed expired file: $RelativePath"
    }
    catch {
        $script:warnings += 1
        Write-Warning "[cache-cleanup] Could not clean ${RelativePath}: $($_.Exception.Message)"
    }
}

function Remove-ExpiredFilesInDirectory {
    param([string]$RelativePath)

    $path = Join-Path $projectRoot $RelativePath
    if (-not (Test-Path -LiteralPath $path -PathType Container)) {
        return
    }

    try {
        $expiredFiles = @(
            Get-ChildItem -LiteralPath $path -Recurse -Force -File -ErrorAction Stop |
                Where-Object { $_.LastWriteTimeUtc -lt $cutoffUtc }
        )

        foreach ($file in $expiredFiles) {
            try {
                $length = $file.Length
                Remove-Item -LiteralPath $file.FullName -Force -ErrorAction Stop
                $script:removedBytes += $length
                $script:removedFiles += 1
            }
            catch {
                $script:warnings += 1
                Write-Warning "[cache-cleanup] Could not clean $($file.FullName): $($_.Exception.Message)"
            }
        }

        Get-ChildItem -LiteralPath $path -Recurse -Force -Directory -ErrorAction SilentlyContinue |
            Sort-Object -Property FullName -Descending |
            Where-Object { -not (Get-ChildItem -LiteralPath $_.FullName -Force -ErrorAction SilentlyContinue) } |
            Remove-Item -Force -ErrorAction SilentlyContinue
    }
    catch {
        $script:warnings += 1
        Write-Warning "[cache-cleanup] Could not scan ${RelativePath}: $($_.Exception.Message)"
    }
}

function Remove-GeneratedPath {
    param([string]$RelativePath)

    $path = Join-Path $projectRoot $RelativePath
    if (-not (Test-Path -LiteralPath $path)) {
        return
    }

    try {
        $item = Get-Item -LiteralPath $path -Force -ErrorAction Stop
        if ($item.PSIsContainer) {
            $stats = Get-DirectoryStats -Path $path
            Remove-Item -LiteralPath $path -Recurse -Force -ErrorAction Stop
            $script:removedBytes += $stats.Bytes
            $script:removedFiles += $stats.Files
            $script:removedGroups += 1
        }
        else {
            $length = $item.Length
            Remove-Item -LiteralPath $path -Force -ErrorAction Stop
            $script:removedBytes += $length
            $script:removedFiles += 1
        }
        Write-CleanupMessage "Removed generated path: $RelativePath"
    }
    catch {
        $script:warnings += 1
        Write-Warning "[cache-cleanup] Could not remove ${RelativePath}: $($_.Exception.Message)"
    }
}

$rustBuildRunning = @(
    Get-Process -Name "cargo", "rustc" -ErrorAction SilentlyContinue
).Count -gt 0

if ($ForMigration) {
    if ($rustBuildRunning) {
        throw "A Cargo or rustc process is running. Finish the build before migration cleanup."
    }

    @(
        "node_modules",
        "dist",
        "src-tauri\target",
        "desktop-core\target",
        "release\portable-smoke"
    ) | ForEach-Object { Remove-GeneratedPath -RelativePath $_ }

    $cachePath = Join-Path $projectRoot "cache"
    if (Test-Path -LiteralPath $cachePath -PathType Container) {
        Get-ChildItem -LiteralPath $cachePath -Force -ErrorAction SilentlyContinue |
            Where-Object { $_.Name -ne "README.md" } |
            ForEach-Object {
                Remove-GeneratedPath -RelativePath ("cache\" + $_.Name)
            }
    }

    $removedMb = [Math]::Round($removedBytes / 1MB, 2)
    Write-CleanupMessage "Migration cleanup removed $removedFiles files from $removedGroups generated group(s) ($removedMb MB)."
    if ($warnings -gt 0) {
        Write-CleanupMessage "Completed with $warnings warning(s)."
        exit 1
    }
    exit 0
}

if ($rustBuildRunning) {
    Write-CleanupMessage "Skipped Rust caches because a Cargo or rustc process is running."
}
else {
    @(
        "src-tauri\target\debug",
        "src-tauri\target\release\incremental",
        "src-tauri\target\release\examples"
        "desktop-core\target\debug"
        "desktop-core\target\release\incremental"
        "desktop-core\target\release\examples"
    ) | ForEach-Object { Remove-ExpiredCacheGroup -RelativePath $_ }

    @(
        "src-tauri\target\release\vlm_screenshot_action.pdb",
        "src-tauri\target\release\vlm_screenshot_action.d",
        "src-tauri\target\release\.cargo-lock",
        "src-tauri\target\.rustc_info.json",
        "src-tauri\target\CACHEDIR.TAG"
    ) | ForEach-Object { Remove-ExpiredFile -RelativePath $_ }
}

# cache\ is the unified temp-artifact directory and may be emptied manually at any
# time. Sweep each subdirectory so cache\README.md and the folder layout survive.
@(
    "cache\vite",
    "cache\logs"
) | ForEach-Object { Remove-ExpiredFilesInDirectory -RelativePath $_ }

$removedMb = [Math]::Round($removedBytes / 1MB, 2)
if ($removedFiles -gt 0 -or $removedGroups -gt 0) {
    Write-CleanupMessage "Removed $removedFiles expired files from $removedGroups cache group(s) ($removedMb MB); retention is $MaxAgeHours hours."
}
else {
    Write-CleanupMessage "No cache files exceeded the $MaxAgeHours-hour retention period."
}

if ($warnings -gt 0) {
    Write-CleanupMessage "Completed with $warnings warning(s)."
}
