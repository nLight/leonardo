param(
    [ValidateSet("Vulkan", "CPU")]
    [string]$GpuBackend = "Vulkan",
    [switch]$SkipEngineBuild,
    [switch]$SkipModelDownload
)

$ErrorActionPreference = "Stop"
Set-StrictMode -Version Latest

$ProjectRoot = Split-Path -Parent $PSScriptRoot
$BuildRoot = Join-Path $ProjectRoot ".build-windows"
$WhisperSource = Join-Path $BuildRoot "whisper.cpp"
$WhisperBuild = Join-Path $WhisperSource "build"
$SidecarDir = Join-Path $ProjectRoot "src-tauri\resources\sidecars"
$ModelDir = Join-Path $ProjectRoot "src-tauri\resources\models"
$ModelName = "ggml-large-v3-turbo-q5_0.bin"
$ModelPath = Join-Path $ModelDir $ModelName
$WhisperVersion = "v1.8.1"

New-Item -ItemType Directory -Force -Path $BuildRoot, $SidecarDir, $ModelDir | Out-Null

if (-not $SkipEngineBuild) {
    if (-not (Test-Path $WhisperSource)) {
        git clone --depth 1 --branch $WhisperVersion https://github.com/ggml-org/whisper.cpp.git $WhisperSource
    }

    $CmakeOptions = @("-S", $WhisperSource, "-B", $WhisperBuild, "-DWHISPER_BUILD_EXAMPLES=ON")
    if ($GpuBackend -eq "Vulkan") {
        if (-not $env:VULKAN_SDK) {
            throw "VULKAN_SDK is not set. Install the Vulkan SDK on this build machine, or run with -GpuBackend CPU. End users do not need the SDK."
        }
        $CmakeOptions += "-DGGML_VULKAN=ON"
    }

    cmake @CmakeOptions
    cmake --build $WhisperBuild --config Release --parallel

    $WhisperOutput = Join-Path $WhisperBuild "bin\Release"
    if (-not (Test-Path (Join-Path $WhisperOutput "whisper-cli.exe"))) {
        throw "whisper-cli.exe was not produced at $WhisperOutput"
    }
    Copy-Item (Join-Path $WhisperOutput "*") $SidecarDir -Recurse -Force

    $FfmpegArchive = Join-Path $BuildRoot "ffmpeg-release-essentials.zip"
    $FfmpegExtracted = Join-Path $BuildRoot "ffmpeg"
    if (-not (Test-Path $FfmpegArchive)) {
        Invoke-WebRequest "https://www.gyan.dev/ffmpeg/builds/ffmpeg-release-essentials.zip" -OutFile $FfmpegArchive
    }
    if (Test-Path $FfmpegExtracted) {
        Remove-Item $FfmpegExtracted -Recurse -Force
    }
    Expand-Archive $FfmpegArchive -DestinationPath $FfmpegExtracted
    $FfmpegExe = Get-ChildItem $FfmpegExtracted -Recurse -Filter "ffmpeg.exe" | Select-Object -First 1
    $FfprobeExe = Get-ChildItem $FfmpegExtracted -Recurse -Filter "ffprobe.exe" | Select-Object -First 1
    if (-not $FfmpegExe -or -not $FfprobeExe) {
        throw "FFmpeg executables were not found in the downloaded archive."
    }
    Copy-Item $FfmpegExe.FullName (Join-Path $SidecarDir "ffmpeg.exe") -Force
    Copy-Item $FfprobeExe.FullName (Join-Path $SidecarDir "ffprobe.exe") -Force
}

if (-not $SkipModelDownload -and -not (Test-Path $ModelPath)) {
    $ModelUrl = "https://huggingface.co/ggerganov/whisper.cpp/resolve/main/$ModelName?download=true"
    Write-Host "Downloading the $ModelName transcription model (about 574 MB)..."
    Invoke-WebRequest $ModelUrl -OutFile $ModelPath
}

if (-not (Test-Path (Join-Path $SidecarDir "whisper-cli.exe"))) {
    throw "Missing whisper-cli.exe in $SidecarDir"
}
if (-not (Test-Path (Join-Path $SidecarDir "ffmpeg.exe"))) {
    throw "Missing ffmpeg.exe in $SidecarDir"
}
if (-not (Test-Path $ModelPath)) {
    throw "Missing model at $ModelPath"
}

Push-Location $ProjectRoot
try {
    npm ci
    npm run tauri build
} finally {
    Pop-Location
}

Write-Host "Windows installer created under src-tauri\target\release\bundle\nsis"
