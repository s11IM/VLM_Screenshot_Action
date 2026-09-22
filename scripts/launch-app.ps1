[CmdletBinding()]
param(
    [string]$AppPath = "",
    [string]$BundledDataPath = "",
    [switch]$WaitForExit,
    [switch]$ValidateOnly
)

$ErrorActionPreference = "Stop"
if ([string]::IsNullOrWhiteSpace($AppPath)) {
    $AppPath = Join-Path $PSScriptRoot "VLM_Screenshot_Action.exe"
}
elseif (-not [IO.Path]::IsPathRooted($AppPath)) {
    $AppPath = Join-Path (Get-Location).Path $AppPath
}
if ([string]::IsNullOrWhiteSpace($BundledDataPath)) {
    $BundledDataPath = Join-Path $PSScriptRoot "user-data"
}
elseif (-not [IO.Path]::IsPathRooted($BundledDataPath)) {
    $BundledDataPath = Join-Path (Get-Location).Path $BundledDataPath
}

function Test-WebView2Runtime {
    $roots = @(
        (Join-Path ${env:ProgramFiles(x86)} "Microsoft\EdgeWebView\Application"),
        (Join-Path $env:ProgramFiles "Microsoft\EdgeWebView\Application"),
        (Join-Path $env:LOCALAPPDATA "Microsoft\EdgeWebView\Application")
    ) | Where-Object { -not [string]::IsNullOrWhiteSpace($_) }

    foreach ($root in $roots) {
        if (-not (Test-Path -LiteralPath $root -PathType Container)) {
            continue
        }
        $runtime = Get-ChildItem -LiteralPath $root -Directory -ErrorAction SilentlyContinue |
            Where-Object { $_.Name -match "^\d+\.\d+\.\d+\.\d+$" } |
            ForEach-Object { Join-Path $_.FullName "msedgewebview2.exe" } |
            Where-Object { Test-Path -LiteralPath $_ -PathType Leaf } |
            Select-Object -First 1
        if ($null -ne $runtime) {
            return $true
        }
    }
    return $false
}

function Install-WebView2Runtime {
    $downloadUrl = "https://go.microsoft.com/fwlink/p/?LinkId=2124703"
    $bootstrapper = Join-Path $env:TEMP "MicrosoftEdgeWebview2Setup.exe"
    Write-Output "[launcher] WebView2 Runtime is missing; downloading the Microsoft bootstrapper..."
    [Net.ServicePointManager]::SecurityProtocol = [Net.ServicePointManager]::SecurityProtocol -bor [Net.SecurityProtocolType]::Tls12
    Invoke-WebRequest -UseBasicParsing -Uri $downloadUrl -OutFile $bootstrapper
    $installer = Start-Process -FilePath $bootstrapper -ArgumentList "/silent", "/install" -Wait -PassThru
    Remove-Item -LiteralPath $bootstrapper -Force -ErrorAction SilentlyContinue
    if (-not (Test-WebView2Runtime)) {
        throw "WebView2 Runtime installation failed with exit code $($installer.ExitCode). Install it from https://developer.microsoft.com/microsoft-edge/webview2/."
    }
}

function Import-BundledUserData {
    if (-not (Test-Path -LiteralPath $BundledDataPath -PathType Container)) {
        return
    }

    $targetProfile = Join-Path $env:LOCALAPPDATA "com.vlmscreenshotaction.app\EBWebView\Default"
    foreach ($area in @("Local Storage", "IndexedDB")) {
        $source = Join-Path $BundledDataPath $area
        $target = Join-Path $targetProfile $area
        if ((Test-Path -LiteralPath $source -PathType Container) -and
            -not (Test-Path -LiteralPath $target)) {
            $null = New-Item -ItemType Directory -Path $targetProfile -Force
            Copy-Item -LiteralPath $source -Destination $target -Recurse -Force
            Write-Output "[launcher] Imported bundled $area data for this Windows user."
        }
    }
}

if (-not [Environment]::Is64BitOperatingSystem) {
    throw "This build requires 64-bit Windows 10 or Windows 11."
}

$resolvedApp = Get-Item -LiteralPath $AppPath -ErrorAction Stop
if (-not (Test-WebView2Runtime)) {
    if ($ValidateOnly) {
        throw "Microsoft Edge WebView2 Runtime is not installed."
    }
    Install-WebView2Runtime
}

if ($ValidateOnly) {
    Write-Output "[launcher] Validation passed for $($resolvedApp.FullName)."
    exit 0
}

Import-BundledUserData

$running = Get-Process -Name $resolvedApp.BaseName -ErrorAction SilentlyContinue
if ($running.Count -gt 0) {
    Write-Output "[launcher] VLM_Screenshot_Action is already running."
    exit 0
}

$process = Start-Process -FilePath $resolvedApp.FullName -WorkingDirectory $resolvedApp.DirectoryName -PassThru
if ($WaitForExit) {
    $process.WaitForExit()
    exit $process.ExitCode
}

Start-Sleep -Seconds 3
$process.Refresh()
if ($process.HasExited) {
    $logPath = Join-Path $env:LOCALAPPDATA "com.vlmscreenshotaction.app\logs\vlm_screenshot_action.log"
    throw "The app exited during startup with code $($process.ExitCode). Log: $logPath"
}
Write-Output "[launcher] VLM_Screenshot_Action started successfully."
