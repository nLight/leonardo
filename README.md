# Leonardo

Leonardo is a local-first Windows recording library for gaming creators. Point it at OBS or capture folders and it turns otherwise anonymous video files into a searchable catalog with timestamped commentary, generated titles, short overviews, edit markers, and subtitles.

No Python runtime, virtual environment, cloud upload, or account is used. The packaged app runs a native [whisper.cpp](https://github.com/ggml-org/whisper.cpp) engine and FFmpeg behind a Tauri desktop interface.

## What works

- Recursively indexes MP4, MKV, MOV, AVI, WebM, M4V, MPEG, MTS, and M2TS recordings.
- Stores the library and transcripts locally in the Tauri app-data folder.
- Extracts mono 16 kHz audio with FFmpeg and transcribes it with `whisper-cli`.
- Understands multi-track MKV audio with automatic microphone detection, a fixed microphone-track setting, or an all-track mix.
- Searches generated titles, summaries, filenames, and every transcript segment.
- Lazily generates and caches real video-frame previews with the bundled FFmpeg.
- Rescans configured capture folders without losing existing transcripts or metadata.
- Supports select-all, shift-range selection, batch transcription, sorting, and keyboard shortcuts (`Ctrl+A`, `Ctrl+K`, `F5`, and `Esc`).
- Creates timestamped chapters and an extractive session overview.
- Preserves original filenames and video files; generated names remain metadata.
- Exports SRT subtitles.
- Exports an FCPXML selects timeline containing the source clip and chapter markers for DaVinci Resolve.
- Includes a polished sample-data preview for evaluating the workflow before importing media.

## End-user experience

A release is a normal per-user `Leonardo_x64-setup.exe`. The installer contains the app, FFmpeg, a native whisper.cpp build, and the selected model. Windows 10/11 already includes the WebView2 runtime in normal installations. An RTX card is used through the bundled Vulkan-enabled whisper.cpp build, requiring only a current NVIDIA graphics driver.

The Settings screen also accepts explicit engine and model paths for development or advanced model swapping.

### Multi-track recording audio

Leonardo probes every audio stream with FFprobe before transcription. The global audio policy can be configured as:

- **Detect microphone:** prefer track metadata containing `Mic`, `Microphone`, `Voice`, `Commentary`, or `Headset`; otherwise use a unique mono track, then fall back to the first audio stream.
- **Always use track number:** use the same one-based audio-track number for every recording, which is ideal for a consistent OBS configuration.
- **Mix every audio track:** combine all streams with FFmpeg's normalized `amix` filter.

The selected source is saved with the recording and shown beside its filename. Mixing is intentionally opt-in because OBS Track 1 is often already a combined program mix; including that together with its individual stems would duplicate the audio.

## Run a developer build

Prerequisites: Node.js 20+, Rust 1.87+, and the normal [Tauri prerequisites](https://v2.tauri.app/start/prerequisites/).

```powershell
npm install
npm run tauri dev
```

For transcription in a developer build, put the following files under `src-tauri/resources` or choose them in Settings:

```text
resources/
  sidecars/
    ffmpeg.exe
    ffprobe.exe
    whisper-cli.exe
    ...whisper/ggml runtime DLLs...
  models/
    ggml-large-v3-turbo-q5_0.bin
```

## Produce the Windows installer

Run this on a Windows build machine with Visual Studio C++ Build Tools, CMake, the Rust MSVC toolchain, Node.js, and the Vulkan SDK:

```powershell
.\scripts\build-windows.ps1
```

The script builds whisper.cpp with Vulkan acceleration, gathers its runtime files, downloads FFmpeg and the 574 MB quantized large-v3-turbo model, and creates an NSIS installer. This is a release-engineering script; people installing Leonardo do not run it and do not need those developer tools.

Before distributing binaries publicly, review and ship the corresponding notices/source obligations for the exact FFmpeg build selected by the script.

### GitHub Actions

The `Build Windows installer` workflow builds and tests Leonardo on `windows-latest` for pull requests, pushes to `main`, tags, and manual runs. It downloads a pinned official whisper.cpp CUDA 12.4 package, verifies the transcription model by SHA-256, caches the large native resources, and uploads `Leonardo_*_x64-setup.exe` as a 14-day workflow artifact.

To build an installer without creating a release, open **Actions → Build Windows installer → Run workflow**. To publish the installer as a GitHub Release asset, push a version tag:

```powershell
git tag v0.1.0
git push origin v0.1.0
```

The tag job waits for the same tested Windows build and then creates the release automatically using the repository's built-in `GITHUB_TOKEN`; no repository secrets are required.

## DaVinci Resolve handoff

Select **Resolve markers** for a transcribed recording. Leonardo writes an `.fcpxml` file into a `Leonardo Exports` folder next to the source recording. In Resolve, use **File → Import → Timeline** and choose the FCPXML file. The original recording is placed on a timeline and the generated key moments arrive as markers. SRT export is available beside it.

Direct project creation through Resolve's scripting API is intentionally kept as a later integration. FCPXML works without running a background service or installing a Python environment.

## Current boundary

The first release uses fast extractive logic for titles, summaries, and chapter descriptions after Whisper transcription. A small local LLM can be added later as an optional native sidecar without changing the library format or requiring Python.
