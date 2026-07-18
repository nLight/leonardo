param(
    [string]$WhisperVersion = "v1.9.1",
    [string]$WhisperAsset = "whisper-cublas-12.4.0-bin-x64.zip",
    [string]$ModelRevision = "98aa99a0a9db05ae2342309f5096248665f7cba3",
    [string]$ModelName = "ggml-large-v3-turbo-q5_0.bin"
)

$ErrorActionPreference = "Stop"
Set-StrictMode -Version Latest

$ProjectRoot = Split-Path -Parent $PSScriptRoot
$DownloadRoot = Join-Path $ProjectRoot ".ci-cache"
$SidecarDir = Join-Path $ProjectRoot "src-tauri\resources\sidecars"
$ModelDir = Join-Path $ProjectRoot "src-tauri\resources\models"
$WhisperSha256 = "106a2030eff8998e4ef320fe72e263a78449e9040386ee27c41ea80b001b601b"
$ModelSha256 = "394221709cd5ad1f40c46e6031ca61bce88931e6e088c188294c6d5a55ffa7e2"
$ModelSize = 574041195

New-Item -ItemType Directory -Force -Path $DownloadRoot, $SidecarDir, $ModelDir | Out-Null

function Download-File {
    param([string]$Url, [string]$Destination)

    Write-Host "Downloading $Url"
    & curl.exe --fail --location --retry 3 --output $Destination $Url
    if ($LASTEXITCODE -ne 0) {
        throw "Download failed with exit code ${LASTEXITCODE}: $Url"
    }
}

function Assert-Sha256 {
    param([string]$Path, [string]$Expected)

    $Actual = (Get-FileHash -Algorithm SHA256 $Path).Hash.ToLowerInvariant()
    if ($Actual -ne $Expected.ToLowerInvariant()) {
        throw "SHA-256 mismatch for $Path. Expected $Expected, received $Actual."
    }
}

$WhisperExe = Join-Path $SidecarDir "whisper-cli.exe"
if (-not (Test-Path $WhisperExe)) {
    $WhisperArchive = Join-Path $DownloadRoot $WhisperAsset
    $WhisperUrl = "https://github.com/ggml-org/whisper.cpp/releases/download/$WhisperVersion/$WhisperAsset"
    Download-File $WhisperUrl $WhisperArchive
    Assert-Sha256 $WhisperArchive $WhisperSha256

    $WhisperExtracted = Join-Path $DownloadRoot "whisper"
    if (Test-Path $WhisperExtracted) {
        Remove-Item $WhisperExtracted -Recurse -Force
    }
    Expand-Archive $WhisperArchive -DestinationPath $WhisperExtracted
    $ExtractedWhisper = Get-ChildItem $WhisperExtracted -Recurse -Filter "whisper-cli.exe" | Select-Object -First 1
    if (-not $ExtractedWhisper) {
        throw "The official whisper.cpp archive did not contain whisper-cli.exe."
    }
    Copy-Item $ExtractedWhisper.FullName $WhisperExe -Force
    Get-ChildItem $ExtractedWhisper.Directory.FullName -Filter "*.dll" | Copy-Item -Destination $SidecarDir -Force
}

$FfmpegExe = Join-Path $SidecarDir "ffmpeg.exe"
$FfprobeExe = Join-Path $SidecarDir "ffprobe.exe"
if (-not (Test-Path $FfmpegExe) -or -not (Test-Path $FfprobeExe)) {
    $FfmpegArchive = Join-Path $DownloadRoot "ffmpeg-release-essentials.zip"
    Download-File "https://www.gyan.dev/ffmpeg/builds/ffmpeg-release-essentials.zip" $FfmpegArchive

    $FfmpegExtracted = Join-Path $DownloadRoot "ffmpeg"
    if (Test-Path $FfmpegExtracted) {
        Remove-Item $FfmpegExtracted -Recurse -Force
    }
    Expand-Archive $FfmpegArchive -DestinationPath $FfmpegExtracted
    $ExtractedFfmpeg = Get-ChildItem $FfmpegExtracted -Recurse -Filter "ffmpeg.exe" | Select-Object -First 1
    $ExtractedFfprobe = Get-ChildItem $FfmpegExtracted -Recurse -Filter "ffprobe.exe" | Select-Object -First 1
    if (-not $ExtractedFfmpeg -or -not $ExtractedFfprobe) {
        throw "The FFmpeg archive did not contain ffmpeg.exe and ffprobe.exe."
    }
    Copy-Item $ExtractedFfmpeg.FullName $FfmpegExe -Force
    Copy-Item $ExtractedFfprobe.FullName $FfprobeExe -Force
}

$ModelPath = Join-Path $ModelDir $ModelName
if (-not (Test-Path $ModelPath)) {
    $ModelUrl = "https://huggingface.co/ggerganov/whisper.cpp/resolve/$ModelRevision/$ModelName?download=true"
    Download-File $ModelUrl $ModelPath
}
if ((Get-Item $ModelPath).Length -ne $ModelSize) {
    throw "Unexpected model size for $ModelPath."
}
Assert-Sha256 $ModelPath $ModelSha256

$RequiredFiles = @($WhisperExe, $FfmpegExe, $FfprobeExe, $ModelPath)
foreach ($RequiredFile in $RequiredFiles) {
    if (-not (Test-Path $RequiredFile)) {
        throw "Required release resource is missing: $RequiredFile"
    }
}

Write-Host "Windows release resources are ready."
Write-Host "Whisper: $WhisperVersion / $WhisperAsset"
Write-Host "Model: $ModelName ($([math]::Round((Get-Item $ModelPath).Length / 1MB)) MB)"
