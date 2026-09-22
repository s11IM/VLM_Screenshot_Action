[CmdletBinding()]
param(
    [Parameter(Mandatory = $true)]
    [string]$AppPath
)

if (-not (Test-Path -LiteralPath $AppPath -PathType Leaf)) {
    exit 1
}

$projectRoot = Split-Path -Parent $PSScriptRoot
$appTimestamp = (Get-Item -LiteralPath $AppPath).LastWriteTimeUtc
$sourceDirectories = @(
    (Join-Path $projectRoot "src"),
    (Join-Path $projectRoot "desktop-core\src"),
    (Join-Path $projectRoot "src-tauri\src"),
    (Join-Path $projectRoot "src-tauri\capabilities")
)
$sourceFiles = @(
    (Join-Path $projectRoot "index.html"),
    (Join-Path $projectRoot "package.json"),
    (Join-Path $projectRoot "package-lock.json"),
    (Join-Path $projectRoot ".node-version"),
    (Join-Path $projectRoot "rust-toolchain.toml"),
    (Join-Path $projectRoot "desktop-core\Cargo.toml"),
    (Join-Path $projectRoot "desktop-core\Cargo.lock"),
    (Join-Path $projectRoot "tsconfig.json"),
    (Join-Path $projectRoot "vite.config.ts"),
    (Join-Path $projectRoot "src-tauri\build.rs"),
    (Join-Path $projectRoot "src-tauri\Cargo.toml"),
    (Join-Path $projectRoot "src-tauri\Cargo.lock"),
    (Join-Path $projectRoot "src-tauri\tauri.conf.json"),
    (Join-Path $projectRoot "src-tauri\windows-app-manifest.xml")
)

foreach ($directory in $sourceDirectories) {
    if (-not (Test-Path -LiteralPath $directory -PathType Container)) {
        continue
    }
    $newerSource = Get-ChildItem -LiteralPath $directory -Recurse -File |
        Where-Object { $_.LastWriteTimeUtc -gt $appTimestamp } |
        Select-Object -First 1
    if ($null -ne $newerSource) {
        exit 1
    }
}

foreach ($file in $sourceFiles) {
    if ((Test-Path -LiteralPath $file -PathType Leaf) -and
        (Get-Item -LiteralPath $file).LastWriteTimeUtc -gt $appTimestamp) {
        exit 1
    }
}

exit 0
