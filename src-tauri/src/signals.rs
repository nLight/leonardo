//! Deterministic activity signals extracted from a recording with FFmpeg filters.
//!
//! Nothing in this module knows about models or prompts. It produces the raw evidence an edit
//! plan is later built from: where the picture changes, how loud the commentary is, and which
//! stretches are dead air, black, or a frozen loading screen. Every timestamp that eventually
//! reaches an exported timeline originates here, which is what keeps a proposed cut verifiable.
//!
//! The filter graphs live next to their parsers on purpose: the graph decides the exact log
//! shape the parser has to survive, and they only stay in sync if they are edited together.

use serde::{Deserialize, Serialize};

/// Bumped whenever the shape or meaning of a stored track changes. It is part of the cache
/// file name, so a bump abandons stale files instead of misreading them.
pub const SIGNAL_TRACK_VERSION: u32 = 1;

/// Momentary loudness arrives every 100 ms. Half-second buckets keep an hour of audio at a few
/// thousand numbers while still resolving a single shout or explosion.
pub const LOUDNESS_STEP_MS: u64 = 500;

/// The floor `ebur128` reports for digital silence.
pub const SILENT_LUFS: f32 = -120.0;

/// Written by the filter graphs, relative to the working directory the pass runs in. Keeping
/// them relative avoids escaping a Windows path inside a filter argument, where both the drive
/// colon and the backslashes are syntax.
pub const SCENE_FILE: &str = "scene.txt";
pub const LOUDNESS_FILE: &str = "loudness.txt";

/// A closed interval of the source recording.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TimeRange {
    pub start_ms: u64,
    pub end_ms: u64,
}

impl TimeRange {
    pub fn duration_ms(&self) -> u64 {
        self.end_ms.saturating_sub(self.start_ms)
    }

    pub fn contains(&self, at_ms: u64) -> bool {
        at_ms >= self.start_ms && at_ms < self.end_ms
    }
}

/// How different one decoded frame is from the one before it, on FFmpeg's 0..1 scale.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SceneSample {
    pub at_ms: u64,
    pub score: f32,
}

/// Everything one analysis pass learned about a recording.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SignalTrack {
    pub version: u32,
    pub duration_ms: u64,
    /// One entry per decoded keyframe, carrying its scene-change score. Thresholding happens in
    /// Rust rather than in the filter so a different cut sensitivity does not cost another pass.
    pub scene_scores: Vec<SceneSample>,
    pub loudness_step_ms: u64,
    /// Peak momentary loudness per bucket, in LUFS.
    pub loudness: Vec<f32>,
    pub silences: Vec<TimeRange>,
    pub blacks: Vec<TimeRange>,
    pub freezes: Vec<TimeRange>,
    /// Which audio the loudness and silence numbers describe, in the wording the library
    /// already uses for transcription.
    pub audio_source: String,
}

/// Sort, then fuse anything overlapping or separated by less than `gap_ms`.
pub fn merge_ranges(mut ranges: Vec<TimeRange>, gap_ms: u64) -> Vec<TimeRange> {
    ranges.sort_by_key(|range| range.start_ms);
    let mut merged: Vec<TimeRange> = Vec::with_capacity(ranges.len());
    for range in ranges {
        match merged.last_mut() {
            Some(last) if range.start_ms <= last.end_ms + gap_ms => {
                last.end_ms = last.end_ms.max(range.end_ms);
            }
            _ => merged.push(range),
        }
    }
    merged
}

/// Everything in `0..duration_ms` that the given ranges do not cover. The input is merged first,
/// so callers may hand over an unsorted, overlapping list.
pub fn invert_ranges(ranges: &[TimeRange], duration_ms: u64) -> Vec<TimeRange> {
    let mut gaps = Vec::new();
    let mut cursor = 0;
    for range in merge_ranges(ranges.to_vec(), 0) {
        if range.start_ms > cursor {
            gaps.push(TimeRange {
                start_ms: cursor,
                end_ms: range.start_ms.min(duration_ms),
            });
        }
        cursor = cursor.max(range.end_ms);
        if cursor >= duration_ms {
            return gaps;
        }
    }
    if cursor < duration_ms {
        gaps.push(TimeRange {
            start_ms: cursor,
            end_ms: duration_ms,
        });
    }
    gaps
}

/// Video analysis, decoding keyframes only.
///
/// `-skip_frame nokey` makes this roughly an order of magnitude cheaper than a full decode, and
/// an OBS encoder already places keyframes more densely where the picture changes hard, so the
/// grid is mildly biased towards activity for free. The cost is a temporal resolution of about
/// two seconds: good enough to propose a candidate, not good enough to choose a cut point.
pub fn video_filter_graph() -> String {
    format!(
        "scale=192:-2,\
         blackdetect=d=0.5:pic_th=0.98,\
         freezedetect=n=-45dB:d=3,\
         select='gte(scene,0)',\
         metadata=print:file={SCENE_FILE}"
    )
}

/// Audio analysis over whichever track the library is configured to listen to.
///
/// `framelog=verbose` demotes `ebur128`'s own per-frame chatter below the info level, leaving
/// `silencedetect` as the only thing on stderr while the loudness curve goes to its own file.
pub fn audio_filter_chain() -> String {
    format!(
        "silencedetect=noise=-35dB:d=0.6,\
         ebur128=metadata=1:framelog=verbose,\
         ametadata=mode=print:key=lavfi.r128.M:file={LOUDNESS_FILE}"
    )
}

/// Assemble a track from the four outputs of the two passes.
pub fn build_signal_track(
    scene_output: &str,
    video_log: &str,
    loudness_output: &str,
    audio_log: &str,
    probed_duration_ms: Option<u64>,
    audio_source: String,
) -> SignalTrack {
    let scene_scores = parse_metadata_samples(scene_output, "lavfi.scene_score")
        .into_iter()
        .map(|(at_ms, score)| SceneSample { at_ms, score })
        .collect::<Vec<_>>();
    let loudness_samples = parse_metadata_samples(loudness_output, "lavfi.r128.M");

    let duration_ms = probed_duration_ms.unwrap_or_default().max(
        scene_scores
            .last()
            .map(|sample| sample.at_ms)
            .max(loudness_samples.last().map(|(at_ms, _)| *at_ms))
            .unwrap_or_default(),
    );

    SignalTrack {
        version: SIGNAL_TRACK_VERSION,
        duration_ms,
        scene_scores,
        loudness_step_ms: LOUDNESS_STEP_MS,
        loudness: downsample_loudness(&loudness_samples, LOUDNESS_STEP_MS, duration_ms),
        silences: parse_log_ranges(audio_log, "silence_start:", "silence_end:", duration_ms),
        blacks: parse_log_ranges(video_log, "black_start:", "black_end:", duration_ms),
        freezes: parse_log_ranges(video_log, "freeze_start:", "freeze_end:", duration_ms),
        audio_source,
    }
}

/// Read the two-line records the `metadata` and `ametadata` filters print:
///
/// ```text
/// frame:12   pts:12012   pts_time:0.5005
/// lavfi.scene_score=0.031250
/// ```
///
/// The timestamp lives on the frame header and the value on the line after it, so the header is
/// remembered until a matching key shows up.
pub fn parse_metadata_samples(output: &str, key: &str) -> Vec<(u64, f32)> {
    let prefix = format!("{key}=");
    let mut samples = Vec::new();
    let mut at_ms = None;
    for line in output.lines() {
        let line = line.trim();
        if let Some(timestamp) = parse_pts_time_ms(line) {
            at_ms = Some(timestamp);
        } else if let Some(value) = line.strip_prefix(&prefix) {
            if let (Some(timestamp), Ok(value)) = (at_ms, value.trim().parse::<f32>()) {
                if value.is_finite() {
                    samples.push((timestamp, value));
                }
            }
        }
    }
    samples.sort_by_key(|(at_ms, _)| *at_ms);
    samples
}

/// Pair up the start and end lines a detector logs.
///
/// `blackdetect` reports both on one line, `silencedetect` and `freezedetect` on separate ones,
/// and all three are handled by looking at each label independently. A range still open when the
/// log runs out is closed at `duration_ms`, which is what a recording that fades to black or
/// ends mid-silence produces.
pub fn parse_log_ranges(
    log: &str,
    start_label: &str,
    end_label: &str,
    duration_ms: u64,
) -> Vec<TimeRange> {
    let mut ranges = Vec::new();
    let mut open = None;
    for line in log.lines() {
        if let Some(start_ms) = parse_labelled_seconds(line, start_label) {
            open = Some(start_ms);
        }
        if let Some(end_ms) = parse_labelled_seconds(line, end_label) {
            if let Some(start_ms) = open.take() {
                push_range(&mut ranges, start_ms, end_ms);
            }
        }
    }
    if let Some(start_ms) = open {
        push_range(&mut ranges, start_ms, duration_ms);
    }
    ranges
}

fn push_range(ranges: &mut Vec<TimeRange>, start_ms: u64, end_ms: u64) {
    if end_ms > start_ms {
        ranges.push(TimeRange { start_ms, end_ms });
    }
}

/// Collapse the 100 ms loudness stream into fixed buckets, keeping the peak of each one. Peaks
/// are what a highlight is made of; an average would flatten the one shout in a quiet minute.
pub fn downsample_loudness(samples: &[(u64, f32)], step_ms: u64, duration_ms: u64) -> Vec<f32> {
    if step_ms == 0 {
        return Vec::new();
    }
    let span_ms = duration_ms.max(samples.last().map(|(at_ms, _)| *at_ms).unwrap_or_default());
    let buckets = (span_ms / step_ms) as usize + 1;
    let mut loudness = vec![SILENT_LUFS; buckets];
    for (at_ms, value) in samples {
        let index = (at_ms / step_ms) as usize;
        if let Some(bucket) = loudness.get_mut(index) {
            if value > bucket {
                *bucket = *value;
            }
        }
    }
    // One decimal is finer than the ear and keeps a long recording's cache small.
    loudness
        .into_iter()
        .map(|value| (value * 10.0).round() / 10.0)
        .collect()
}

/// Pull `pts_time:0.5005` out of a frame header line.
fn parse_pts_time_ms(line: &str) -> Option<u64> {
    line.split_whitespace()
        .find_map(|token| token.strip_prefix("pts_time:"))
        .and_then(|value| value.parse::<f64>().ok())
        .map(seconds_to_ms)
}

/// Pull the number that follows a detector label, whether or not a space separates them.
fn parse_labelled_seconds(line: &str, label: &str) -> Option<u64> {
    let start = line.find(label)? + label.len();
    line[start..]
        .trim_start()
        .split_whitespace()
        .next()?
        .trim_end_matches('|')
        .parse::<f64>()
        .ok()
        .map(seconds_to_ms)
}

fn seconds_to_ms(seconds: f64) -> u64 {
    if seconds.is_finite() && seconds > 0.0 {
        (seconds * 1_000.0).round() as u64
    } else {
        0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Captured from FFmpeg 6.1 running the graphs above over a synthetic clip: ten seconds of
    // moving test pattern, five seconds of black, then ten seconds of a static chart. Keeping the
    // real shapes here — the interleaved freeze metadata, the one-line black report, the run of
    // decimals — is what makes these tests worth anything.
    const SCENE_OUTPUT: &str = "\
frame:0    pts:0       pts_time:0
lavfi.scene_score=0.000000
frame:1    pts:15360   pts_time:1
lavfi.scene_score=0.047219
frame:10   pts:153600  pts_time:10
lavfi.freezedetect.freeze_start=10
lavfi.scene_score=1.000000
frame:11   pts:168960  pts_time:11
lavfi.scene_score=0.000000
";

    const VIDEO_LOG: &str = "\
[freezedetect @ 0x55c6e76a1440] lavfi.freezedetect.freeze_start: 10
[blackdetect @ 0x7f8ef4001a00] black_start:10 black_end:15 black_duration:5
[freezedetect @ 0x55c6e76a1440] lavfi.freezedetect.freeze_duration: 5
[freezedetect @ 0x55c6e76a1440] lavfi.freezedetect.freeze_end: 15
[freezedetect @ 0x55c6e76a1440] lavfi.freezedetect.freeze_start: 15
";

    const LOUDNESS_OUTPUT: &str = "\
frame:0    pts:0       pts_time:0
lavfi.r128.M=-120.691
frame:5    pts:4800    pts_time:0.1
lavfi.r128.M=-31.400
frame:20   pts:19200   pts_time:0.4
lavfi.r128.M=-18.200
frame:30   pts:28800   pts_time:0.6
lavfi.r128.M=-24.900
";

    const AUDIO_LOG: &str = "\
[silencedetect @ 0x55ed01486d40] silence_start: 7.99996
[silencedetect @ 0x55ed01486d40] silence_end: 14.016 | silence_duration: 6.01604
";

    #[test]
    fn reads_scene_scores_from_metadata_records() {
        let samples = parse_metadata_samples(SCENE_OUTPUT, "lavfi.scene_score");
        assert_eq!(samples.len(), 4);
        assert_eq!(samples[1].0, 1_000);
        assert!((samples[1].1 - 0.047219).abs() < 1e-6);
    }

    #[test]
    fn skips_metadata_other_filters_left_on_the_same_frame() {
        // `freezedetect` annotates frames, and `metadata=print` dumps every key it finds, so the
        // scene record for a frozen frame carries a foreign line between its header and its score.
        let samples = parse_metadata_samples(SCENE_OUTPUT, "lavfi.scene_score");
        assert_eq!(samples[2], (10_000, 1.0));
        assert!(parse_metadata_samples(SCENE_OUTPUT, "lavfi.r128.M").is_empty());
    }

    #[test]
    fn pairs_detector_ranges_reported_on_one_line() {
        let blacks = parse_log_ranges(VIDEO_LOG, "black_start:", "black_end:", 25_000);
        assert_eq!(
            blacks,
            vec![TimeRange {
                start_ms: 10_000,
                end_ms: 15_000
            }]
        );
    }

    #[test]
    fn pairs_detector_ranges_reported_across_lines() {
        let silences = parse_log_ranges(AUDIO_LOG, "silence_start:", "silence_end:", 25_000);
        assert_eq!(
            silences,
            vec![TimeRange {
                start_ms: 8_000,
                end_ms: 14_016
            }]
        );
    }

    #[test]
    fn closes_a_range_left_open_at_the_end_of_the_recording() {
        // A recording that ends on a static screen leaves the last freeze unterminated.
        let freezes = parse_log_ranges(VIDEO_LOG, "freeze_start:", "freeze_end:", 25_000);
        assert_eq!(
            freezes,
            vec![
                TimeRange {
                    start_ms: 10_000,
                    end_ms: 15_000
                },
                TimeRange {
                    start_ms: 15_000,
                    end_ms: 25_000
                }
            ]
        );
    }

    #[test]
    fn keeps_the_peak_of_every_loudness_bucket() {
        let samples = parse_metadata_samples(LOUDNESS_OUTPUT, "lavfi.r128.M");
        let loudness = downsample_loudness(&samples, 500, 1_000);
        assert_eq!(loudness.len(), 3);
        assert!((loudness[0] + 18.2).abs() < 0.05);
        assert!((loudness[1] + 24.9).abs() < 0.05);
        assert_eq!(loudness[2], SILENT_LUFS);
    }

    #[test]
    fn builds_a_track_from_both_passes() {
        let track = build_signal_track(
            SCENE_OUTPUT,
            VIDEO_LOG,
            LOUDNESS_OUTPUT,
            AUDIO_LOG,
            Some(25_000),
            "Track 2 — Mic/Aux (detected by name)".into(),
        );
        assert_eq!(track.version, SIGNAL_TRACK_VERSION);
        assert_eq!(track.duration_ms, 25_000);
        assert_eq!(track.scene_scores.len(), 4);
        assert_eq!(track.silences.len(), 1);
        assert_eq!(track.blacks.len(), 1);
        assert_eq!(track.freezes.len(), 2);
        assert_eq!(track.loudness.len(), 51);
    }

    #[test]
    fn falls_back_to_the_last_sample_when_the_duration_is_unknown() {
        let track = build_signal_track(SCENE_OUTPUT, "", LOUDNESS_OUTPUT, "", None, String::new());
        assert_eq!(track.duration_ms, 11_000);
    }

    #[test]
    fn quotes_the_scene_expression_so_its_comma_stays_inside_one_filter() {
        let graph = video_filter_graph();
        assert!(graph.contains("select='gte(scene,0)'"));
        assert!(graph.ends_with(&format!("metadata=print:file={SCENE_FILE}")));
    }
}
