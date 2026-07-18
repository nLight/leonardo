# Architecture

```text
Capture folders
      │ recursive scan (read-only)
      ▼
Tauri / Rust library ───────► local library.json
      │
      ├─ FFprobe ────────────► audio track metadata / mic selection
      │
      ├─ FFmpeg executable ─► selected or mixed temporary 16 kHz WAV
      │
      └─ whisper.cpp ───────► timestamped SRT
                                  │
                                  ▼
                    title + overview + chapters
                                  │
              ┌───────────────────┴──────────────────┐
              ▼                                      ▼
      searchable desktop UI                 SRT / FCPXML export
```

The webview owns presentation and transient selection state. Rust owns filesystem access, persistence, process execution, subtitle parsing, derived metadata, and exports. Long transcription commands run on Tauri's blocking worker pool, keeping the window responsive.

Original media is only read. Temporary audio is written under the app cache and removed after a successful transcription. Exports are the only files written beside a user's recordings, and they are isolated in a `Leonardo Exports` directory.
