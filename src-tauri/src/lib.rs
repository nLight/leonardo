mod candidates;
mod signals;

use base64::Engine;
use candidates::Highlights;
use serde::{Deserialize, Serialize};
use signals::SignalTrack;
#[cfg(target_os = "windows")]
use std::os::windows::{fs::MetadataExt, process::CommandExt};
use std::{
    collections::{hash_map::DefaultHasher, HashMap, HashSet},
    fs,
    hash::{Hash, Hasher},
    io::Read,
    path::{Path, PathBuf},
    process::Command,
    sync::Mutex,
    time::{SystemTime, UNIX_EPOCH},
};
use tauri::{AppHandle, Manager, State};
use walkdir::{DirEntry, WalkDir};

const MEDIA_EXTENSIONS: &[&str] = &[
    "mp4", "mkv", "mov", "avi", "webm", "m4v", "mpg", "mpeg", "mts", "m2ts",
];
// Recycle-bin index records can inherit the original video extension but are only
// a few hundred bytes. Real recordings comfortably exceed this conservative floor.
const MIN_MEDIA_FILE_BYTES: u64 = 64 * 1024;

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct TranscriptSegment {
    start_ms: u64,
    end_ms: u64,
    text: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Chapter {
    start_ms: u64,
    title: String,
    description: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct AudioTrackInfo {
    number: usize,
    stream_index: u32,
    title: String,
    codec: String,
    channels: u32,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Recording {
    id: String,
    path: String,
    file_name: String,
    display_title: String,
    extension: String,
    size_bytes: u64,
    modified_at: u64,
    duration_ms: Option<u64>,
    status: String,
    progress: u8,
    summary: String,
    language: Option<String>,
    #[serde(default)]
    audio_tracks: Vec<AudioTrackInfo>,
    #[serde(default)]
    audio_source: Option<String>,
    transcript: Vec<TranscriptSegment>,
    chapters: Vec<Chapter>,
    error: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct AppSettings {
    #[serde(default)]
    media_folders: Vec<String>,
    #[serde(default)]
    ffmpeg_path: String,
    #[serde(default)]
    ffprobe_path: String,
    #[serde(default)]
    whisper_path: String,
    #[serde(default)]
    model_path: String,
    #[serde(default = "default_language")]
    language: String,
    #[serde(default = "default_audio_mode")]
    audio_mode: String,
    #[serde(default = "default_microphone_track")]
    microphone_track: usize,
}

impl Default for AppSettings {
    fn default() -> Self {
        Self {
            media_folders: Vec::new(),
            ffmpeg_path: String::new(),
            ffprobe_path: String::new(),
            whisper_path: String::new(),
            model_path: String::new(),
            language: default_language(),
            audio_mode: default_audio_mode(),
            microphone_track: default_microphone_track(),
        }
    }
}

fn default_language() -> String {
    "auto".into()
}

fn default_audio_mode() -> String {
    "auto".into()
}

fn default_microphone_track() -> usize {
    1
}

#[derive(Debug, Deserialize)]
struct FfprobeAudioResponse {
    #[serde(default)]
    streams: Vec<FfprobeAudioStream>,
}

#[derive(Debug, Deserialize)]
struct FfprobeAudioStream {
    index: u32,
    #[serde(default)]
    codec_name: String,
    #[serde(default)]
    channels: u32,
    #[serde(default)]
    tags: HashMap<String, String>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct LibrarySnapshot {
    #[serde(default)]
    recordings: Vec<Recording>,
    #[serde(default)]
    settings: AppSettings,
}

struct LibraryState(Mutex<LibrarySnapshot>);

#[tauri::command]
fn load_library(state: State<'_, LibraryState>) -> Result<LibrarySnapshot, String> {
    state
        .0
        .lock()
        .map(|library| library.clone())
        .map_err(lock_error)
}

#[tauri::command]
fn choose_and_scan_folder(
    app: AppHandle,
    state: State<'_, LibraryState>,
) -> Result<LibrarySnapshot, String> {
    let Some(folder) = rfd::FileDialog::new()
        .set_title("Choose your capture folder")
        .pick_folder()
    else {
        return state
            .0
            .lock()
            .map(|library| library.clone())
            .map_err(lock_error);
    };

    let mut library = state.0.lock().map_err(lock_error)?;
    let folder_string = folder.to_string_lossy().to_string();
    if !library
        .settings
        .media_folders
        .iter()
        .any(|existing| existing == &folder_string)
    {
        library.settings.media_folders.push(folder_string);
    }
    scan_library(&mut library)?;
    save_library(&app, &library)?;
    Ok(library.clone())
}

#[tauri::command]
fn rescan_media_folders(
    app: AppHandle,
    state: State<'_, LibraryState>,
) -> Result<LibrarySnapshot, String> {
    let mut library = state.0.lock().map_err(lock_error)?;
    scan_library(&mut library)?;
    save_library(&app, &library)?;
    Ok(library.clone())
}

#[tauri::command]
fn save_settings(
    app: AppHandle,
    state: State<'_, LibraryState>,
    settings: AppSettings,
) -> Result<AppSettings, String> {
    let mut library = state.0.lock().map_err(lock_error)?;
    library.settings = settings;
    save_library(&app, &library)?;
    Ok(library.settings.clone())
}

#[tauri::command]
async fn transcribe_recording(
    app: AppHandle,
    state: State<'_, LibraryState>,
    id: String,
) -> Result<Recording, String> {
    let (recording, settings) = {
        let mut library = state.0.lock().map_err(lock_error)?;
        let item = library
            .recordings
            .iter_mut()
            .find(|item| item.id == id)
            .ok_or_else(|| "Recording not found".to_string())?;
        item.status = "processing".into();
        item.progress = 5;
        item.error = None;
        (item.clone(), library.settings.clone())
    };

    let worker_app = app.clone();
    let result = tauri::async_runtime::spawn_blocking(move || {
        process_recording(&worker_app, recording, settings)
    })
    .await
    .map_err(|error| format!("Transcription worker stopped: {error}"))?;

    let mut library = state.0.lock().map_err(lock_error)?;
    let index = library
        .recordings
        .iter()
        .position(|item| item.id == id)
        .ok_or_else(|| "Recording disappeared from the library".to_string())?;
    match result {
        Ok(updated) => {
            library.recordings[index] = updated.clone();
            save_library(&app, &library)?;
            Ok(updated)
        }
        Err(error) => {
            library.recordings[index].status = "error".into();
            library.recordings[index].progress = 0;
            library.recordings[index].error = Some(error.clone());
            save_library(&app, &library)?;
            Err(error)
        }
    }
}

#[tauri::command]
async fn recording_thumbnail(
    app: AppHandle,
    state: State<'_, LibraryState>,
    id: String,
) -> Result<String, String> {
    let (recording, settings) = {
        let library = state.0.lock().map_err(lock_error)?;
        let recording = library
            .recordings
            .iter()
            .find(|item| item.id == id)
            .cloned()
            .ok_or_else(|| "Recording not found".to_string())?;
        (recording, library.settings.clone())
    };

    tauri::async_runtime::spawn_blocking(move || {
        create_recording_thumbnail(&app, &settings, &recording)
    })
    .await
    .map_err(|error| format!("Thumbnail worker stopped: {error}"))?
}

/// Run — or reuse — the deterministic signal pass for one recording.
///
/// The pass is I/O bound and reads the whole file, so its result is cached beside the library
/// and only recomputed when the caller explicitly asks for a refresh.
#[tauri::command]
async fn recording_signals(
    app: AppHandle,
    state: State<'_, LibraryState>,
    id: String,
    refresh: Option<bool>,
) -> Result<SignalTrack, String> {
    let (recording, settings) = {
        let library = state.0.lock().map_err(lock_error)?;
        let recording = library
            .recordings
            .iter()
            .find(|item| item.id == id)
            .cloned()
            .ok_or_else(|| "Recording not found".to_string())?;
        (recording, library.settings.clone())
    };

    tauri::async_runtime::spawn_blocking(move || {
        analyze_recording_signals(&app, &settings, &recording, refresh.unwrap_or(false))
    })
    .await
    .map_err(|error| format!("Signal worker stopped: {error}"))?
}

/// Candidate clips and dead air for one recording.
///
/// Deriving them is pure arithmetic over a cached signal track, so nothing here is stored — only
/// the pass underneath it is.
#[tauri::command]
async fn recording_highlights(
    app: AppHandle,
    state: State<'_, LibraryState>,
    id: String,
    refresh: Option<bool>,
) -> Result<Highlights, String> {
    let (recording, settings) = {
        let library = state.0.lock().map_err(lock_error)?;
        let recording = library
            .recordings
            .iter()
            .find(|item| item.id == id)
            .cloned()
            .ok_or_else(|| "Recording not found".to_string())?;
        (recording, library.settings.clone())
    };

    tauri::async_runtime::spawn_blocking(move || {
        analyze_recording_signals(&app, &settings, &recording, refresh.unwrap_or(false))
            .map(|track| candidates::build_highlights(&track))
    })
    .await
    .map_err(|error| format!("Signal worker stopped: {error}"))?
}

#[tauri::command]
fn open_recording(
    state: State<'_, LibraryState>,
    id: String,
    seek_ms: Option<u64>,
) -> Result<(), String> {
    let library = state.0.lock().map_err(lock_error)?;
    let recording = library
        .recordings
        .iter()
        .find(|item| item.id == id)
        .ok_or_else(|| "Recording not found".to_string())?;
    open_path(Path::new(&recording.path), seek_ms)
}

#[tauri::command]
fn export_srt(state: State<'_, LibraryState>, id: String) -> Result<String, String> {
    let library = state.0.lock().map_err(lock_error)?;
    let recording = library
        .recordings
        .iter()
        .find(|item| item.id == id)
        .ok_or_else(|| "Recording not found".to_string())?;
    let export_path = export_path_for(recording, "srt")?;
    let contents = recording
        .transcript
        .iter()
        .enumerate()
        .map(|(index, segment)| {
            format!(
                "{}\n{} --> {}\n{}\n",
                index + 1,
                srt_time(segment.start_ms),
                srt_time(segment.end_ms),
                segment.text
            )
        })
        .collect::<Vec<_>>()
        .join("\n");
    fs::write(&export_path, contents)
        .map_err(|error| format!("Could not write subtitles: {error}"))?;
    Ok(export_path.to_string_lossy().to_string())
}

#[tauri::command]
fn export_resolve_markers(state: State<'_, LibraryState>, id: String) -> Result<String, String> {
    let library = state.0.lock().map_err(lock_error)?;
    let recording = library
        .recordings
        .iter()
        .find(|item| item.id == id)
        .ok_or_else(|| "Recording not found".to_string())?;
    let export_path = export_path_for(recording, "fcpxml")?;
    let duration = recording
        .duration_ms
        .or_else(|| recording.transcript.last().map(|segment| segment.end_ms))
        .unwrap_or(1_000);
    let path_url = file_url(&recording.path);
    let markers = recording
        .chapters
        .iter()
        .map(|chapter| {
            format!(
                "<marker start=\"{}/1000s\" value=\"{}\" note=\"{}\"/>",
                chapter.start_ms,
                xml_escape(&chapter.title),
                xml_escape(&chapter.description)
            )
        })
        .collect::<Vec<_>>()
        .join("");
    let contents = format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE fcpxml>
<fcpxml version="1.10">
  <resources>
    <format id="r1" name="FFVideoFormat1080p30" frameDuration="1/30s" width="1920" height="1080" colorSpace="1-1-1 (Rec. 709)"/>
    <asset id="r2" name="{}" start="0s" duration="{}/1000s" hasVideo="1" hasAudio="1" format="r1">
      <media-rep kind="original-media" src="{}"/>
    </asset>
  </resources>
  <library><event name="Leonardo Selects"><project name="{}">
    <sequence format="r1" duration="{}/1000s" tcStart="0s" tcFormat="NDF" audioLayout="stereo" audioRate="48k">
      <spine><asset-clip name="{}" ref="r2" offset="0s" start="0s" duration="{}/1000s">{}</asset-clip></spine>
    </sequence>
  </project></event></library>
</fcpxml>
"#,
        xml_escape(&recording.file_name),
        duration,
        xml_escape(&path_url),
        xml_escape(&recording.display_title),
        duration,
        xml_escape(&recording.file_name),
        duration,
        markers
    );
    fs::write(&export_path, contents)
        .map_err(|error| format!("Could not write Resolve timeline: {error}"))?;
    Ok(export_path.to_string_lossy().to_string())
}

fn process_recording(
    app: &AppHandle,
    mut recording: Recording,
    settings: AppSettings,
) -> Result<Recording, String> {
    let ffmpeg = resolve_tool(app, &settings.ffmpeg_path, "ffmpeg");
    let whisper = resolve_tool(app, &settings.whisper_path, "whisper-cli");
    let model = resolve_model(app, &settings.model_path)?;
    let work_dir = app
        .path()
        .app_cache_dir()
        .map_err(|error| format!("Could not locate cache: {error}"))?
        .join("jobs")
        .join(&recording.id);
    fs::create_dir_all(&work_dir)
        .map_err(|error| format!("Could not create transcription workspace: {error}"))?;
    let wav_path = work_dir.join("audio.wav");
    let output_base = work_dir.join("transcript");

    let audio_tracks = probe_audio_tracks(app, &settings, &recording.path)?;
    let mut ffmpeg_command = hidden_command(&ffmpeg);
    ffmpeg_command
        .args(["-hide_banner", "-loglevel", "error", "-y", "-i"])
        .arg(&recording.path);
    let audio_source = configure_audio_mapping(&mut ffmpeg_command, &audio_tracks, &settings)?;
    let ffmpeg_output = ffmpeg_command
        .args(["-vn", "-ar", "16000", "-ac", "1", "-c:a", "pcm_s16le"])
        .arg(&wav_path)
        .output()
        .map_err(|error| tool_start_error("FFmpeg", &ffmpeg, error))?;
    if !ffmpeg_output.status.success() {
        return Err(format!(
            "FFmpeg could not extract the audio. {}",
            stderr_text(&ffmpeg_output.stderr)
        ));
    }

    let mut whisper_command = hidden_command(&whisper);
    whisper_command
        .arg("-m")
        .arg(&model)
        .arg("-f")
        .arg(&wav_path)
        .arg("-osrt")
        .arg("-of")
        .arg(&output_base);
    if !settings.language.trim().is_empty() {
        whisper_command.arg("-l").arg(&settings.language);
    }
    let whisper_output = whisper_command
        .output()
        .map_err(|error| tool_start_error("whisper.cpp", &whisper, error))?;
    if !whisper_output.status.success() {
        return Err(format!(
            "whisper.cpp could not transcribe the audio. {}",
            stderr_text(&whisper_output.stderr)
        ));
    }

    let srt_path = output_base.with_extension("srt");
    let srt = fs::read_to_string(&srt_path).map_err(|error| {
        format!("Transcription finished but its SRT could not be read: {error}")
    })?;
    let transcript = parse_srt(&srt)?;
    if transcript.is_empty() {
        return Err("The engine returned an empty transcript. Check that the recording has an audible commentary track.".into());
    }
    let chapters = build_chapters(&transcript);
    recording.display_title = build_title(&recording, &transcript);
    recording.summary = build_summary(&transcript, &chapters);
    recording.duration_ms = probe_duration(app, &settings, &recording.path)
        .or_else(|| transcript.last().map(|segment| segment.end_ms));
    recording.language = Some(settings.language);
    recording.audio_tracks = audio_tracks;
    recording.audio_source = Some(audio_source);
    recording.transcript = transcript;
    recording.chapters = chapters;
    recording.status = "ready".into();
    recording.progress = 100;
    recording.error = None;
    let _ = fs::remove_file(wav_path);
    Ok(recording)
}

fn scan_library(library: &mut LibrarySnapshot) -> Result<(), String> {
    let existing = library
        .recordings
        .drain(..)
        .map(|recording| (recording.path.clone(), recording))
        .collect::<HashMap<_, _>>();
    let mut found_paths = HashSet::new();
    let mut recordings = Vec::new();
    for folder in &library.settings.media_folders {
        for entry in WalkDir::new(folder)
            .follow_links(false)
            .into_iter()
            .filter_entry(should_visit_entry)
            .filter_map(Result::ok)
            .filter(|entry| entry.file_type().is_file())
        {
            let path = entry.path();
            let extension = path
                .extension()
                .and_then(|value| value.to_str())
                .unwrap_or_default()
                .to_lowercase();
            if !MEDIA_EXTENSIONS.contains(&extension.as_str()) {
                continue;
            }
            let path_string = path.to_string_lossy().to_string();
            if !found_paths.insert(path_string.clone()) {
                continue;
            }
            let metadata = entry
                .metadata()
                .map_err(|error| format!("Could not inspect {}: {error}", path.display()))?;
            if !is_indexable_media(path, &metadata) {
                continue;
            }
            let modified_at = metadata
                .modified()
                .ok()
                .and_then(system_time_ms)
                .unwrap_or_default();
            if let Some(mut previous) = existing.get(&path_string).cloned() {
                previous.size_bytes = metadata.len();
                previous.modified_at = modified_at;
                recordings.push(previous);
            } else {
                let file_name = path
                    .file_name()
                    .and_then(|value| value.to_str())
                    .unwrap_or("recording")
                    .to_string();
                let display_title = path
                    .file_stem()
                    .and_then(|value| value.to_str())
                    .unwrap_or("Untitled recording")
                    .replace(['_', '-'], " ");
                recordings.push(Recording {
                    id: recording_id(&path_string),
                    path: path_string,
                    file_name,
                    display_title,
                    extension,
                    size_bytes: metadata.len(),
                    modified_at,
                    duration_ms: None,
                    status: "new".into(),
                    progress: 0,
                    summary: String::new(),
                    language: None,
                    audio_tracks: Vec::new(),
                    audio_source: None,
                    transcript: Vec::new(),
                    chapters: Vec::new(),
                    error: None,
                });
            }
        }
    }
    recordings.sort_by(|left, right| right.modified_at.cmp(&left.modified_at));
    library.recordings = recordings;
    Ok(())
}

fn should_visit_entry(entry: &DirEntry) -> bool {
    if entry.depth() == 0 {
        return true;
    }
    let name = entry.file_name().to_string_lossy();
    if name.starts_with('.') || is_ignored_system_name(&name) {
        return false;
    }
    entry
        .metadata()
        .map(|metadata| !has_hidden_or_system_attributes(&metadata))
        .unwrap_or(true)
}

fn is_indexable_media(path: &Path, metadata: &fs::Metadata) -> bool {
    metadata.is_file()
        && metadata.len() >= MIN_MEDIA_FILE_BYTES
        && !has_hidden_or_system_attributes(metadata)
        && !is_ignored_path(path)
}

fn is_ignored_system_name(name: &str) -> bool {
    matches!(
        name.to_ascii_lowercase().as_str(),
        "$recycle.bin" | "recycler" | "system volume information"
    )
}

fn is_ignored_path(path: &Path) -> bool {
    path.components().any(|component| {
        let name = component.as_os_str().to_string_lossy();
        name.starts_with('.') || is_ignored_system_name(&name)
    })
}

fn has_hidden_or_system_attributes(metadata: &fs::Metadata) -> bool {
    #[cfg(target_os = "windows")]
    {
        const FILE_ATTRIBUTE_HIDDEN: u32 = 0x2;
        const FILE_ATTRIBUTE_SYSTEM: u32 = 0x4;
        return metadata.file_attributes() & (FILE_ATTRIBUTE_HIDDEN | FILE_ATTRIBUTE_SYSTEM) != 0;
    }
    #[cfg(not(target_os = "windows"))]
    {
        let _ = metadata;
        false
    }
}

fn remove_invalid_index_entries(library: &mut LibrarySnapshot) -> bool {
    let previous_len = library.recordings.len();
    library.recordings.retain(|recording| {
        recording.size_bytes >= MIN_MEDIA_FILE_BYTES && !is_ignored_path(Path::new(&recording.path))
    });
    library.recordings.len() != previous_len
}

fn create_recording_thumbnail(
    app: &AppHandle,
    settings: &AppSettings,
    recording: &Recording,
) -> Result<String, String> {
    let thumbnail_dir = app
        .path()
        .app_data_dir()
        .map_err(|error| format!("Could not locate persistent thumbnail storage: {error}"))?
        .join("thumbnails");
    fs::create_dir_all(&thumbnail_dir)
        .map_err(|error| format!("Could not create persistent thumbnail storage: {error}"))?;
    let file_name = format!("v1-{}-{}.jpg", recording.id, recording.modified_at);
    let thumbnail_path = thumbnail_dir.join(&file_name);

    if !thumbnail_is_valid(&thumbnail_path) {
        let _ = fs::remove_file(&thumbnail_path);
        if let Ok(legacy_dir) = app.path().app_cache_dir() {
            let legacy_path = legacy_dir
                .join("thumbnails")
                .join(format!("{}-{}.jpg", recording.id, recording.modified_at));
            if thumbnail_is_valid(&legacy_path) {
                let _ = fs::copy(legacy_path, &thumbnail_path);
            }
        }
    }

    if !thumbnail_is_valid(&thumbnail_path) {
        let ffmpeg = resolve_tool(app, &settings.ffmpeg_path, "ffmpeg");
        let seek_seconds = recording
            .duration_ms
            .map(|duration| (duration as f64 / 1_000.0 * 0.12).clamp(1.0, 30.0))
            .unwrap_or(3.0);
        let temporary_path = thumbnail_dir.join(format!("{file_name}.pending.jpg"));
        let _ = fs::remove_file(&temporary_path);
        let mut command = hidden_command(&ffmpeg);
        let output = command
            .args(["-hide_banner", "-loglevel", "error", "-y", "-ss"])
            .arg(format!("{seek_seconds:.3}"))
            .arg("-i")
            .arg(&recording.path)
            .args([
                "-frames:v",
                "1",
                "-an",
                "-sn",
                "-vf",
                "scale=320:180:force_original_aspect_ratio=decrease,pad=320:180:(ow-iw)/2:(oh-ih)/2",
                "-q:v",
                "6",
            ])
            .arg(&temporary_path)
            .output()
            .map_err(|error| tool_start_error("FFmpeg", &ffmpeg, error))?;
        if !output.status.success() || !thumbnail_is_valid(&temporary_path) {
            let _ = fs::remove_file(&temporary_path);
            return Err(format!(
                "FFmpeg could not create a preview. {}",
                stderr_text(&output.stderr)
            ));
        }
        fs::rename(&temporary_path, &thumbnail_path)
            .map_err(|error| format!("Could not save generated preview: {error}"))?;

        if let Ok(entries) = fs::read_dir(&thumbnail_dir) {
            let obsolete_prefix = format!("v1-{}-", recording.id);
            for entry in entries.filter_map(Result::ok) {
                let path = entry.path();
                let is_obsolete = path != thumbnail_path
                    && path
                        .file_name()
                        .and_then(|name| name.to_str())
                        .is_some_and(|name| name.starts_with(&obsolete_prefix));
                if is_obsolete {
                    let _ = fs::remove_file(path);
                }
            }
        }
    }

    let bytes = fs::read(&thumbnail_path)
        .map_err(|error| format!("Could not read generated preview: {error}"))?;
    Ok(format!(
        "data:image/jpeg;base64,{}",
        base64::engine::general_purpose::STANDARD.encode(bytes)
    ))
}

fn thumbnail_is_valid(path: &Path) -> bool {
    let Ok(metadata) = fs::metadata(path) else {
        return false;
    };
    if !metadata.is_file() || metadata.len() < 512 {
        return false;
    }
    let Ok(mut file) = fs::File::open(path) else {
        return false;
    };
    let mut header = [0_u8; 3];
    file.read_exact(&mut header).is_ok() && header == [0xff, 0xd8, 0xff]
}

fn analyze_recording_signals(
    app: &AppHandle,
    settings: &AppSettings,
    recording: &Recording,
    refresh: bool,
) -> Result<SignalTrack, String> {
    let cache_path = signal_track_path(app, recording)?;
    if !refresh {
        if let Some(track) = read_cached_signal_track(&cache_path) {
            return Ok(track);
        }
    }

    let ffmpeg = resolve_tool(app, &settings.ffmpeg_path, "ffmpeg");
    let work_dir = app
        .path()
        .app_cache_dir()
        .map_err(|error| format!("Could not locate cache: {error}"))?
        .join("signals")
        .join(&recording.id);
    fs::create_dir_all(&work_dir)
        .map_err(|error| format!("Could not create analysis workspace: {error}"))?;
    let scene_path = work_dir.join(signals::SCENE_FILE);
    let loudness_path = work_dir.join(signals::LOUDNESS_FILE);
    let _ = fs::remove_file(&scene_path);
    let _ = fs::remove_file(&loudness_path);

    // Both passes run with the workspace as their working directory so the filter graphs can
    // name their output files without a Windows path, where the drive colon and the backslashes
    // would collide with filter argument syntax.
    let mut video_command = hidden_command(&ffmpeg);
    video_command
        .current_dir(&work_dir)
        .args([
            "-hide_banner",
            "-nostats",
            "-loglevel",
            "info",
            "-skip_frame",
            "nokey",
            "-i",
        ])
        .arg(&recording.path)
        .args(["-an", "-sn", "-map", "0:v:0", "-vf"])
        .arg(signals::video_filter_graph())
        .args(["-f", "null", "-"]);
    let video_output = video_command
        .output()
        .map_err(|error| tool_start_error("FFmpeg", &ffmpeg, error))?;
    if !video_output.status.success() {
        return Err(format!(
            "FFmpeg could not analyse the picture. {}",
            stderr_text(&video_output.stderr)
        ));
    }

    let audio_tracks = probe_audio_tracks(app, settings, &recording.path)?;
    let mut audio_command = hidden_command(&ffmpeg);
    audio_command
        .current_dir(&work_dir)
        .args(["-hide_banner", "-nostats", "-loglevel", "info", "-i"])
        .arg(&recording.path);
    let audio_source = configure_audio_analysis(
        &mut audio_command,
        &audio_tracks,
        settings,
        &signals::audio_filter_chain(),
    )?;
    audio_command.args(["-vn", "-sn", "-f", "null", "-"]);
    let audio_output = audio_command
        .output()
        .map_err(|error| tool_start_error("FFmpeg", &ffmpeg, error))?;
    if !audio_output.status.success() {
        return Err(format!(
            "FFmpeg could not analyse the audio. {}",
            stderr_text(&audio_output.stderr)
        ));
    }

    let track = signals::build_signal_track(
        &fs::read_to_string(&scene_path).unwrap_or_default(),
        &String::from_utf8_lossy(&video_output.stderr),
        &fs::read_to_string(&loudness_path).unwrap_or_default(),
        &String::from_utf8_lossy(&audio_output.stderr),
        recording
            .duration_ms
            .or_else(|| probe_duration(app, settings, &recording.path)),
        audio_source,
    );
    if track.scene_scores.is_empty() && track.loudness.is_empty() {
        return Err(
            "The analysis pass produced no signals. Check that the recording still decodes.".into(),
        );
    }

    let json = serde_json::to_string(&track)
        .map_err(|error| format!("Could not serialize the signal track: {error}"))?;
    fs::write(&cache_path, json)
        .map_err(|error| format!("Could not save the signal track: {error}"))?;
    prune_signal_cache(&cache_path, &recording.id);
    let _ = fs::remove_dir_all(&work_dir);
    Ok(track)
}

fn signal_track_path(app: &AppHandle, recording: &Recording) -> Result<PathBuf, String> {
    let directory = app
        .path()
        .app_data_dir()
        .map_err(|error| format!("Could not locate persistent signal storage: {error}"))?
        .join("signals");
    fs::create_dir_all(&directory)
        .map_err(|error| format!("Could not create persistent signal storage: {error}"))?;
    Ok(directory.join(format!(
        "v{}-{}-{}.json",
        signals::SIGNAL_TRACK_VERSION,
        recording.id,
        recording.modified_at
    )))
}

fn read_cached_signal_track(path: &Path) -> Option<SignalTrack> {
    let track: SignalTrack = fs::read_to_string(path)
        .ok()
        .and_then(|json| serde_json::from_str(&json).ok())?;
    (track.version == signals::SIGNAL_TRACK_VERSION).then_some(track)
}

/// Drop tracks left over from an earlier version of the schema or an earlier edit of the file.
fn prune_signal_cache(current: &Path, recording_id: &str) {
    let Some(directory) = current.parent() else {
        return;
    };
    let Ok(entries) = fs::read_dir(directory) else {
        return;
    };
    for entry in entries.filter_map(Result::ok) {
        let path = entry.path();
        let is_obsolete = path != current
            && path
                .file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.contains(recording_id));
        if is_obsolete {
            let _ = fs::remove_file(path);
        }
    }
}

fn parse_srt(contents: &str) -> Result<Vec<TranscriptSegment>, String> {
    let normalized = contents.replace("\r\n", "\n");
    let mut segments = Vec::new();
    for block in normalized.split("\n\n") {
        let lines = block.lines().collect::<Vec<_>>();
        let Some(timing_index) = lines.iter().position(|line| line.contains(" --> ")) else {
            continue;
        };
        let mut times = lines[timing_index].split(" --> ");
        let start_ms = parse_srt_time(times.next().unwrap_or_default())?;
        let end_ms = parse_srt_time(times.next().unwrap_or_default())?;
        let text = lines
            .iter()
            .skip(timing_index + 1)
            .map(|line| line.trim())
            .filter(|line| !line.is_empty())
            .collect::<Vec<_>>()
            .join(" ");
        if !text.is_empty() {
            segments.push(TranscriptSegment {
                start_ms,
                end_ms,
                text,
            });
        }
    }
    Ok(segments)
}

fn parse_srt_time(value: &str) -> Result<u64, String> {
    let parts = value
        .trim()
        .replace('.', ",")
        .split([':', ','])
        .map(str::parse::<u64>)
        .collect::<Result<Vec<_>, _>>()
        .map_err(|_| format!("Invalid subtitle timestamp: {value}"))?;
    if parts.len() != 4 {
        return Err(format!("Invalid subtitle timestamp: {value}"));
    }
    Ok(parts[0] * 3_600_000 + parts[1] * 60_000 + parts[2] * 1_000 + parts[3])
}

fn build_chapters(transcript: &[TranscriptSegment]) -> Vec<Chapter> {
    if transcript.is_empty() {
        return Vec::new();
    }
    let total = transcript
        .last()
        .map(|segment| segment.end_ms)
        .unwrap_or_default();
    let interval = (total / 6).clamp(180_000, 900_000);
    let keywords = [
        "boss", "fight", "mission", "match", "quest", "build", "level", "found", "win", "lost",
        "attempt", "team", "map", "final", "start",
    ];
    let mut chapters = Vec::new();
    let mut target = 0;
    while target <= total && chapters.len() < 8 {
        let window_end = target.saturating_add(interval);
        let candidates = transcript
            .iter()
            .filter(|segment| segment.start_ms >= target && segment.start_ms < window_end)
            .collect::<Vec<_>>();
        if let Some(segment) = candidates.iter().max_by_key(|segment| {
            let lower = segment.text.to_lowercase();
            keywords
                .iter()
                .filter(|keyword| lower.contains(**keyword))
                .count()
                * 10
                + segment.text.len().min(120)
        }) {
            let clean = clean_sentence(&segment.text);
            let title = short_phrase(&clean, 7);
            chapters.push(Chapter {
                start_ms: segment.start_ms,
                title,
                description: short_text(&clean, 150),
            });
        }
        target = window_end;
        if interval == 0 {
            break;
        }
    }
    deduplicate_chapters(chapters)
}

fn deduplicate_chapters(chapters: Vec<Chapter>) -> Vec<Chapter> {
    let mut seen = HashSet::new();
    chapters
        .into_iter()
        .filter(|chapter| seen.insert(chapter.title.to_lowercase()))
        .collect()
}

fn build_title(recording: &Recording, transcript: &[TranscriptSegment]) -> String {
    let source = transcript
        .iter()
        .take(12)
        .max_by_key(|segment| segment.text.len())
        .map(|segment| clean_sentence(&segment.text))
        .unwrap_or_default();
    if source.len() >= 12 {
        short_phrase(&source, 10)
    } else {
        recording.display_title.clone()
    }
}

fn build_summary(transcript: &[TranscriptSegment], chapters: &[Chapter]) -> String {
    let opening = transcript
        .iter()
        .take(10)
        .find(|segment| segment.text.split_whitespace().count() >= 7)
        .map(|segment| clean_sentence(&segment.text))
        .unwrap_or_default();
    let moments = chapters
        .iter()
        .skip(1)
        .take(3)
        .map(|chapter| chapter.title.clone())
        .collect::<Vec<_>>();
    if moments.is_empty() {
        return short_text(&opening, 260);
    }
    format!(
        "{} Key moments include {}.",
        ensure_period(&short_text(&opening, 170)),
        natural_list(&moments)
    )
}

fn short_phrase(value: &str, words: usize) -> String {
    let phrase = value
        .trim_matches(|character: char| !character.is_alphanumeric())
        .split_whitespace()
        .take(words)
        .collect::<Vec<_>>()
        .join(" ");
    if phrase.is_empty() {
        "Untitled recording".into()
    } else {
        title_case_first(&phrase)
    }
}

fn short_text(value: &str, max_chars: usize) -> String {
    if value.chars().count() <= max_chars {
        return ensure_period(value);
    }
    let shortened = value.chars().take(max_chars).collect::<String>();
    let boundary = shortened.rfind(' ').unwrap_or(shortened.len());
    format!(
        "{}…",
        shortened[..boundary].trim_end_matches(['.', ',', ' '])
    )
}

fn clean_sentence(value: &str) -> String {
    value
        .replace("[SPEAKER_TURN]", "")
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .trim()
        .to_string()
}

fn ensure_period(value: &str) -> String {
    let trimmed = value.trim();
    if trimmed.is_empty() || trimmed.ends_with(['.', '!', '?', '…']) {
        trimmed.into()
    } else {
        format!("{trimmed}.")
    }
}

fn title_case_first(value: &str) -> String {
    let mut chars = value.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().collect::<String>() + chars.as_str(),
        None => String::new(),
    }
}

fn natural_list(values: &[String]) -> String {
    match values {
        [] => String::new(),
        [one] => one.clone(),
        [first, second] => format!("{first} and {second}"),
        _ => format!(
            "{}, and {}",
            values[..values.len() - 1].join(", "),
            values.last().unwrap()
        ),
    }
}

fn probe_audio_tracks(
    app: &AppHandle,
    settings: &AppSettings,
    path: &str,
) -> Result<Vec<AudioTrackInfo>, String> {
    let ffprobe = resolve_tool(app, &settings.ffprobe_path, "ffprobe");
    let output = hidden_command(&ffprobe)
        .args([
            "-v",
            "error",
            "-select_streams",
            "a",
            "-show_entries",
            "stream=index,codec_name,channels:stream_tags=title,handler_name",
            "-of",
            "json",
        ])
        .arg(path)
        .output()
        .map_err(|error| tool_start_error("FFprobe", &ffprobe, error))?;
    if !output.status.success() {
        return Err(format!(
            "FFprobe could not inspect the audio tracks. {}",
            stderr_text(&output.stderr)
        ));
    }
    let response: FfprobeAudioResponse = serde_json::from_slice(&output.stdout)
        .map_err(|error| format!("FFprobe returned invalid audio-track data: {error}"))?;
    let tracks = response
        .streams
        .into_iter()
        .enumerate()
        .map(|(position, stream)| {
            let title = stream
                .tags
                .get("title")
                .or_else(|| stream.tags.get("handler_name"))
                .filter(|value| !value.trim().is_empty())
                .cloned()
                .unwrap_or_else(|| format!("Audio track {}", position + 1));
            AudioTrackInfo {
                number: position + 1,
                stream_index: stream.index,
                title,
                codec: stream.codec_name,
                channels: stream.channels,
            }
        })
        .collect::<Vec<_>>();
    if tracks.is_empty() {
        Err("This recording does not contain an audio track.".into())
    } else {
        Ok(tracks)
    }
}

/// Which audio a pass should listen to, once the configured policy has been applied.
enum AudioSelection {
    /// Every track combined, carrying how many there are.
    Mix(usize),
    /// A single zero-based track index.
    Track(usize),
}

fn select_audio_source(
    tracks: &[AudioTrackInfo],
    settings: &AppSettings,
) -> Result<(AudioSelection, String), String> {
    match settings.audio_mode.as_str() {
        "mix" if tracks.len() > 1 => Ok((
            AudioSelection::Mix(tracks.len()),
            format!("Mixed {} audio tracks", tracks.len()),
        )),
        "mix" => Ok((
            AudioSelection::Track(0),
            track_label(&tracks[0], "only audio track"),
        )),
        "track" => {
            let number = settings.microphone_track.max(1);
            let track = tracks.get(number - 1).ok_or_else(|| {
                let available = tracks
                    .iter()
                    .map(|track| format!("{} ({})", track.number, track.title))
                    .collect::<Vec<_>>()
                    .join(", ");
                format!(
                    "Microphone track {number} does not exist in this recording. Available audio tracks: {available}"
                )
            })?;
            Ok((
                AudioSelection::Track(number - 1),
                track_label(track, "configured microphone track"),
            ))
        }
        "auto" => {
            let (index, reason) = select_automatic_microphone_track(tracks);
            Ok((
                AudioSelection::Track(index),
                track_label(&tracks[index], reason),
            ))
        }
        value => Err(format!("Unknown audio selection mode: {value}")),
    }
}

fn amix_filter(count: usize) -> String {
    format!("amix=inputs={count}:duration=longest:dropout_transition=0:normalize=1")
}

fn mix_inputs(count: usize) -> String {
    (0..count)
        .map(|index| format!("[0:a:{index}]"))
        .collect::<String>()
}

fn configure_audio_mapping(
    command: &mut Command,
    tracks: &[AudioTrackInfo],
    settings: &AppSettings,
) -> Result<String, String> {
    let (selection, label) = select_audio_source(tracks, settings)?;
    match selection {
        AudioSelection::Mix(count) => {
            command
                .arg("-filter_complex")
                .arg(format!("{}{}[aout]", mix_inputs(count), amix_filter(count)))
                .args(["-map", "[aout]"]);
        }
        AudioSelection::Track(index) => {
            command.arg("-map").arg(format!("0:a:{index}"));
        }
    }
    Ok(label)
}

/// Map the same audio a transcription would use, with an analysis chain appended.
///
/// Analysis always goes through `-filter_complex` because a simple `-af` chain cannot be
/// attached to a stream that a complex graph already produces, which is what the mixing policy
/// builds.
fn configure_audio_analysis(
    command: &mut Command,
    tracks: &[AudioTrackInfo],
    settings: &AppSettings,
    chain: &str,
) -> Result<String, String> {
    let (selection, label) = select_audio_source(tracks, settings)?;
    let graph = match selection {
        AudioSelection::Mix(count) => {
            format!("{}{},{chain}[aout]", mix_inputs(count), amix_filter(count))
        }
        AudioSelection::Track(index) => format!("[0:a:{index}]{chain}[aout]"),
    };
    command
        .arg("-filter_complex")
        .arg(graph)
        .args(["-map", "[aout]"]);
    Ok(label)
}

fn select_automatic_microphone_track(tracks: &[AudioTrackInfo]) -> (usize, &'static str) {
    let microphone_words = ["mic", "microphone", "voice", "commentary", "headset"];
    if let Some((index, _)) = tracks.iter().enumerate().find(|(_, track)| {
        let title = track.title.to_lowercase();
        microphone_words.iter().any(|word| title.contains(word))
    }) {
        return (index, "detected by name");
    }

    let mono_tracks = tracks
        .iter()
        .enumerate()
        .filter(|(_, track)| track.channels == 1)
        .collect::<Vec<_>>();
    if mono_tracks.len() == 1 {
        return (mono_tracks[0].0, "unique mono track");
    }

    (0, "automatic fallback")
}

fn track_label(track: &AudioTrackInfo, reason: &str) -> String {
    format!("Track {} — {} ({reason})", track.number, track.title)
}

fn probe_duration(app: &AppHandle, settings: &AppSettings, path: &str) -> Option<u64> {
    let ffprobe = resolve_tool(app, &settings.ffprobe_path, "ffprobe");
    let output = hidden_command(&ffprobe)
        .args([
            "-v",
            "error",
            "-show_entries",
            "format=duration",
            "-of",
            "default=noprint_wrappers=1:nokey=1",
        ])
        .arg(path)
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    String::from_utf8_lossy(&output.stdout)
        .trim()
        .parse::<f64>()
        .ok()
        .map(|seconds| (seconds * 1000.0) as u64)
}

fn resolve_tool(app: &AppHandle, configured: &str, name: &str) -> PathBuf {
    if !configured.trim().is_empty() {
        return PathBuf::from(configured);
    }
    let executable = if cfg!(windows) {
        format!("{name}.exe")
    } else {
        name.to_string()
    };
    if let Ok(resource_dir) = app.path().resource_dir() {
        let bundled = resource_dir
            .join("resources")
            .join("sidecars")
            .join(&executable);
        if bundled.exists() {
            return bundled;
        }
    }
    PathBuf::from(executable)
}

fn hidden_command(program: &Path) -> Command {
    let mut command = Command::new(program);
    #[cfg(target_os = "windows")]
    command.creation_flags(0x0800_0000);
    command
}

fn resolve_model(app: &AppHandle, configured: &str) -> Result<PathBuf, String> {
    if !configured.trim().is_empty() {
        let path = PathBuf::from(configured);
        return path.exists().then_some(path).ok_or_else(|| "The configured Whisper model does not exist. Open Settings and choose a .bin model file.".into());
    }
    if let Ok(resource_dir) = app.path().resource_dir() {
        let bundled = resource_dir
            .join("resources")
            .join("models")
            .join("ggml-large-v3-turbo-q5_0.bin");
        if bundled.exists() {
            return Ok(bundled);
        }
    }
    Err("No Whisper model is configured. Open Settings and choose a whisper.cpp GGML model; release installers can bundle one automatically.".into())
}

fn export_path_for(recording: &Recording, extension: &str) -> Result<PathBuf, String> {
    let source = Path::new(&recording.path);
    let directory = source
        .parent()
        .unwrap_or_else(|| Path::new("."))
        .join("Leonardo Exports");
    fs::create_dir_all(&directory)
        .map_err(|error| format!("Could not create export folder: {error}"))?;
    let stem = source
        .file_stem()
        .and_then(|value| value.to_str())
        .unwrap_or("recording");
    Ok(directory.join(format!("{stem}.{extension}")))
}

fn open_path(path: &Path, seek_ms: Option<u64>) -> Result<(), String> {
    #[cfg(target_os = "windows")]
    {
        let _ = seek_ms;
        Command::new("cmd")
            .args(["/C", "start", ""])
            .arg(path)
            .spawn()
            .map_err(|error| format!("Could not open recording: {error}"))?;
    }
    #[cfg(target_os = "macos")]
    {
        let _ = seek_ms;
        Command::new("open")
            .arg(path)
            .spawn()
            .map_err(|error| format!("Could not open recording: {error}"))?;
    }
    #[cfg(target_os = "linux")]
    {
        let _ = seek_ms;
        Command::new("xdg-open")
            .arg(path)
            .spawn()
            .map_err(|error| format!("Could not open recording: {error}"))?;
    }
    Ok(())
}

fn save_library(app: &AppHandle, library: &LibrarySnapshot) -> Result<(), String> {
    let path = library_path(app)?;
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .map_err(|error| format!("Could not create app data folder: {error}"))?;
    }
    let json = serde_json::to_string_pretty(library)
        .map_err(|error| format!("Could not serialize library: {error}"))?;
    fs::write(path, json).map_err(|error| format!("Could not save library: {error}"))
}

fn read_library(app: &AppHandle) -> LibrarySnapshot {
    let Ok(path) = library_path(app) else {
        return LibrarySnapshot::default();
    };
    fs::read_to_string(path)
        .ok()
        .and_then(|json| serde_json::from_str(&json).ok())
        .unwrap_or_default()
}

fn library_path(app: &AppHandle) -> Result<PathBuf, String> {
    app.path()
        .app_data_dir()
        .map(|path| path.join("library.json"))
        .map_err(|error| format!("Could not locate app data: {error}"))
}

fn recording_id(path: &str) -> String {
    let mut hasher = DefaultHasher::new();
    path.to_lowercase().hash(&mut hasher);
    format!("{:016x}", hasher.finish())
}

fn system_time_ms(time: SystemTime) -> Option<u64> {
    time.duration_since(UNIX_EPOCH)
        .ok()
        .map(|duration| duration.as_millis() as u64)
}

fn srt_time(ms: u64) -> String {
    let hours = ms / 3_600_000;
    let minutes = (ms % 3_600_000) / 60_000;
    let seconds = (ms % 60_000) / 1_000;
    format!("{hours:02}:{minutes:02}:{seconds:02},{:03}", ms % 1_000)
}

fn file_url(path: &str) -> String {
    let normalized = path.replace('\\', "/").replace(' ', "%20");
    if normalized.starts_with('/') {
        format!("file://{normalized}")
    } else {
        format!("file:///{normalized}")
    }
}

fn xml_escape(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&apos;")
}

fn stderr_text(bytes: &[u8]) -> String {
    let value = String::from_utf8_lossy(bytes).trim().to_string();
    if value.chars().count() > 800 {
        format!("{}…", value.chars().take(800).collect::<String>())
    } else {
        value
    }
}

fn tool_start_error(name: &str, path: &Path, error: std::io::Error) -> String {
    format!(
        "Could not start {name} at '{}': {error}. Open Settings to select the executable.",
        path.display()
    )
}

fn lock_error<T>(error: std::sync::PoisonError<T>) -> String {
    format!("The library is temporarily unavailable: {error}")
}

pub fn run() {
    tauri::Builder::default()
        .setup(|app| {
            let mut library = read_library(app.handle());
            if remove_invalid_index_entries(&mut library) {
                let _ = save_library(app.handle(), &library);
            }
            app.manage(LibraryState(Mutex::new(library)));
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            load_library,
            choose_and_scan_folder,
            rescan_media_folders,
            save_settings,
            transcribe_recording,
            recording_thumbnail,
            recording_signals,
            recording_highlights,
            open_recording,
            export_srt,
            export_resolve_markers
        ])
        .run(tauri::generate_context!())
        .expect("error while running Leonardo");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_srt_segments() {
        let srt = "1\n00:00:01,200 --> 00:00:03,400\nHello there.\n\n2\n00:01:00,000 --> 00:01:02,500\nA second line.\n";
        let segments = parse_srt(srt).unwrap();
        assert_eq!(segments.len(), 2);
        assert_eq!(segments[0].start_ms, 1_200);
        assert_eq!(segments[1].end_ms, 62_500);
    }

    #[test]
    fn exports_srt_timestamps() {
        assert_eq!(srt_time(3_723_045), "01:02:03,045");
    }

    #[test]
    fn ignores_hidden_and_windows_system_paths() {
        assert!(is_ignored_path(Path::new(
            "/captures/$RECYCLE.BIN/$IIMS3LD.mkv"
        )));
        assert!(is_ignored_path(Path::new(
            "/captures/System Volume Information/index.mp4"
        )));
        assert!(is_ignored_path(Path::new("/captures/.trash/video.mkv")));
        assert!(!is_ignored_path(Path::new(
            "/captures/OBS/2026-07-19-session.mkv"
        )));
    }

    #[test]
    fn builds_bounded_chapters() {
        let segments = (0..40)
            .map(|index| TranscriptSegment {
                start_ms: index * 60_000,
                end_ms: index * 60_000 + 8_000,
                text: format!("We start mission {index} and fight the boss with this build"),
            })
            .collect::<Vec<_>>();
        let chapters = build_chapters(&segments);
        assert!(!chapters.is_empty());
        assert!(chapters.len() <= 8);
    }

    #[test]
    fn detects_named_microphone_track() {
        let tracks = vec![
            AudioTrackInfo {
                number: 1,
                stream_index: 1,
                title: "Desktop Audio".into(),
                codec: "aac".into(),
                channels: 2,
            },
            AudioTrackInfo {
                number: 2,
                stream_index: 2,
                title: "Mic/Aux".into(),
                codec: "aac".into(),
                channels: 2,
            },
        ];
        assert_eq!(
            select_automatic_microphone_track(&tracks),
            (1, "detected by name")
        );
    }

    #[test]
    fn detects_unique_mono_track_without_names() {
        let tracks = vec![
            AudioTrackInfo {
                number: 1,
                stream_index: 1,
                title: "Audio track 1".into(),
                codec: "aac".into(),
                channels: 2,
            },
            AudioTrackInfo {
                number: 2,
                stream_index: 2,
                title: "Audio track 2".into(),
                codec: "aac".into(),
                channels: 1,
            },
        ];
        assert_eq!(
            select_automatic_microphone_track(&tracks),
            (1, "unique mono track")
        );
    }
}
