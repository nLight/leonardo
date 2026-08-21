//! Candidate clips derived from a signal track.
//!
//! This is the layer that turns raw detector output into things an editor — or a planner — can
//! actually choose between: a bounded list of scored clips, the dead air worth cutting out of a
//! narration take, and the stretches no B-roll should ever be pulled from.
//!
//! Every candidate carries an id. Later layers refer to clips by id and never by timestamp, so a
//! plan can always be checked against the evidence it claims to rest on.

use serde::{Deserialize, Serialize};

use crate::signals::{merge_ranges, SceneSample, SignalTrack, TimeRange, SILENT_LUFS};

pub const HIGHLIGHTS_VERSION: u32 = 1;

/// A decoded frame this different from the one before it is treated as a hard cut. Keyframe-only
/// decoding spaces frames a couple of seconds apart, which inflates scores, so the bar sits well
/// above the 0.3 that suits a full-rate decode.
const SCENE_CUT_THRESHOLD: f32 = 0.45;

/// Momentary loudness is mapped onto 0..1 between these bounds. Game commentary sits around
/// -25 LUFS, so -12 is a shout and -45 is room tone.
const QUIET_LUFS: f32 = -45.0;
const LOUD_LUFS: f32 = -12.0;

/// How the two evidence streams combine. Loudness leads because a reaction is audible before it
/// is visible, and because keyframe-grid scene scores are the noisier of the two.
const LOUDNESS_WEIGHT: f32 = 0.65;
const SCENE_WEIGHT: f32 = 0.35;

const MIN_CLIP_MS: u64 = 3_000;
const TARGET_CLIP_MS: u64 = 8_000;
const MAX_CLIP_MS: u64 = 15_000;

/// A clip boundary is pulled onto a real scene cut if one is this close, so a proposal lands on a
/// picture change instead of mid-motion.
const SNAP_WINDOW_MS: u64 = 2_000;

/// Below this mean excitement a window is not worth proposing.
const MIN_CLIP_SCORE: f32 = 0.18;

/// Roughly one candidate per two minutes of footage, within these bounds.
const CLIP_INTERVAL_MS: u64 = 120_000;
const MIN_CLIPS: usize = 3;
const MAX_CLIPS: usize = 40;

/// Shorter gaps than this are breath, not dead air.
const MIN_DEAD_AIR_MS: u64 = 700;

/// Gaps between unusable stretches this small are closed, so one loading screen does not arrive
/// as five fragments.
const UNUSABLE_MERGE_GAP_MS: u64 = 1_000;

/// A stretch of footage worth proposing, with the evidence behind the proposal.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Candidate {
    /// Stable within a signal track, and the only handle later layers are allowed to use.
    pub id: String,
    pub start_ms: u64,
    pub end_ms: u64,
    /// Mean excitement across the clip, 0..1.
    pub score: f32,
    pub peak_lufs: f32,
    pub scene_cuts: usize,
    /// Why this clip was proposed, in the words a person would use.
    pub reason: String,
}

/// Everything the candidate layer knows about one recording.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Highlights {
    pub version: u32,
    pub duration_ms: u64,
    pub audio_source: String,
    pub clips: Vec<Candidate>,
    /// Silence long enough to be worth cutting out of a narration take.
    pub dead_air: Vec<TimeRange>,
    /// Black and frozen stretches — loading screens, menus, an abandoned capture. No candidate
    /// starts inside one and no B-roll should be pulled from one.
    pub unusable: Vec<TimeRange>,
}

pub fn build_highlights(track: &SignalTrack) -> Highlights {
    let unusable = merge_ranges(
        track
            .blacks
            .iter()
            .chain(&track.freezes)
            .copied()
            .collect::<Vec<_>>(),
        UNUSABLE_MERGE_GAP_MS,
    );
    Highlights {
        version: HIGHLIGHTS_VERSION,
        duration_ms: track.duration_ms,
        audio_source: track.audio_source.clone(),
        clips: pick_clips(track, &unusable),
        dead_air: track
            .silences
            .iter()
            .copied()
            .filter(|range| range.duration_ms() >= MIN_DEAD_AIR_MS)
            .collect(),
        unusable,
    }
}

/// Blend loudness and scene-change density into a single curve on the loudness grid, then flatten
/// whatever falls inside an unusable stretch. A loading screen can be loud; it is never a clip.
fn excitement_curve(track: &SignalTrack, unusable: &[TimeRange]) -> Vec<f32> {
    let step_ms = track.loudness_step_ms.max(1);
    let buckets = track.loudness.len();
    let scene = scene_density(&track.scene_scores, buckets, step_ms);
    let mut curve = track
        .loudness
        .iter()
        .zip(scene)
        .map(|(lufs, density)| normalize_loudness(*lufs) * LOUDNESS_WEIGHT + density * SCENE_WEIGHT)
        .collect::<Vec<_>>();
    for (index, value) in curve.iter_mut().enumerate() {
        let at_ms = index as u64 * step_ms;
        if unusable.iter().any(|range| range.contains(at_ms)) {
            *value = 0.0;
        }
    }
    // Two seconds of smoothing: enough to join a shout to the cut that caused it, short enough to
    // keep a three-second moment distinguishable from the minute around it.
    smooth(&curve, (2_000 / step_ms).max(1) as usize)
}

/// How busy the picture is around each bucket, 0..1.
fn scene_density(samples: &[SceneSample], buckets: usize, step_ms: u64) -> Vec<f32> {
    let mut density = vec![0.0; buckets];
    for sample in samples.iter().filter(|s| s.score >= SCENE_CUT_THRESHOLD) {
        let index = (sample.at_ms / step_ms) as usize;
        if let Some(bucket) = density.get_mut(index) {
            *bucket = 1.0;
        }
    }
    smooth(&density, (3_000 / step_ms.max(1)).max(1) as usize)
        .into_iter()
        .map(|value| (value * 3.0).min(1.0))
        .collect()
}

fn normalize_loudness(lufs: f32) -> f32 {
    if lufs <= SILENT_LUFS {
        return 0.0;
    }
    ((lufs - QUIET_LUFS) / (LOUD_LUFS - QUIET_LUFS)).clamp(0.0, 1.0)
}

fn smooth(values: &[f32], radius: usize) -> Vec<f32> {
    if radius == 0 || values.is_empty() {
        return values.to_vec();
    }
    (0..values.len())
        .map(|index| {
            let start = index.saturating_sub(radius);
            let end = (index + radius + 1).min(values.len());
            values[start..end].iter().sum::<f32>() / (end - start) as f32
        })
        .collect()
}

/// Take the best non-overlapping windows off the excitement curve, strongest first.
///
/// Greedy selection rather than a global optimum: an editor scanning the list wants the loudest
/// moment first, and a clip that loses to a stronger neighbour was never going to survive the cut
/// anyway.
fn pick_clips(track: &SignalTrack, unusable: &[TimeRange]) -> Vec<Candidate> {
    let step_ms = track.loudness_step_ms.max(1);
    let curve = excitement_curve(track, unusable);
    let window = (TARGET_CLIP_MS / step_ms).max(1) as usize;
    if curve.len() < window {
        return Vec::new();
    }

    let limit = ((track.duration_ms / CLIP_INTERVAL_MS) as usize).clamp(MIN_CLIPS, MAX_CLIPS);
    let cuts = scene_cut_times(&track.scene_scores);
    let mut taken: Vec<(usize, usize)> = Vec::new();
    let mut clips = Vec::new();

    while clips.len() < limit {
        let Some((start, score)) = (0..=curve.len() - window)
            .filter(|start| {
                taken
                    .iter()
                    .all(|(from, to)| start + window <= *from || start >= to)
            })
            .map(|start| {
                let mean = curve[start..start + window].iter().sum::<f32>() / window as f32;
                (start, mean)
            })
            .max_by(|left, right| left.1.total_cmp(&right.1))
        else {
            break;
        };
        if score < MIN_CLIP_SCORE {
            break;
        }
        taken.push((start, start + window));
        clips.push(describe_clip(
            snap_to_cuts(
                TimeRange {
                    start_ms: start as u64 * step_ms,
                    end_ms: (start + window) as u64 * step_ms,
                },
                &cuts,
                track.duration_ms,
            ),
            score,
            track,
            &cuts,
        ));
    }

    clips.sort_by_key(|clip| clip.start_ms);
    for (index, clip) in clips.iter_mut().enumerate() {
        clip.id = format!("c{index:04}");
    }
    clips
}

fn scene_cut_times(samples: &[SceneSample]) -> Vec<u64> {
    samples
        .iter()
        .filter(|sample| sample.score >= SCENE_CUT_THRESHOLD)
        .map(|sample| sample.at_ms)
        .collect()
}

/// Pull both edges onto nearby scene cuts, then keep the result inside the allowed clip length.
fn snap_to_cuts(range: TimeRange, cuts: &[u64], duration_ms: u64) -> TimeRange {
    let start_ms = nearest_cut(cuts, range.start_ms).unwrap_or(range.start_ms);
    let end_ms = nearest_cut(cuts, range.end_ms).unwrap_or(range.end_ms);
    let start_ms = start_ms.min(duration_ms.saturating_sub(MIN_CLIP_MS));
    let end_ms = end_ms
        .max(start_ms + MIN_CLIP_MS)
        .min(start_ms + MAX_CLIP_MS)
        .min(duration_ms);
    TimeRange { start_ms, end_ms }
}

fn nearest_cut(cuts: &[u64], at_ms: u64) -> Option<u64> {
    cuts.iter()
        .copied()
        .filter(|cut| cut.abs_diff(at_ms) <= SNAP_WINDOW_MS)
        .min_by_key(|cut| cut.abs_diff(at_ms))
}

fn describe_clip(range: TimeRange, score: f32, track: &SignalTrack, cuts: &[u64]) -> Candidate {
    let step_ms = track.loudness_step_ms.max(1);
    let peak_lufs = track
        .loudness
        .iter()
        .enumerate()
        .filter(|(index, _)| range.contains(*index as u64 * step_ms))
        .map(|(_, lufs)| *lufs)
        .fold(SILENT_LUFS, f32::max);
    let scene_cuts = cuts.iter().filter(|cut| range.contains(**cut)).count();
    Candidate {
        // Replaced once the whole list is ordered by time.
        id: String::new(),
        start_ms: range.start_ms,
        end_ms: range.end_ms,
        score: (score * 100.0).round() / 100.0,
        peak_lufs: (peak_lufs * 10.0).round() / 10.0,
        scene_cuts,
        reason: match (peak_lufs > -20.0, scene_cuts >= 2) {
            (true, true) => "loud moment over fast cuts".into(),
            (true, false) => "loud moment".into(),
            (false, true) => "fast cuts".into(),
            (false, false) => "steady activity".into(),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::signals::{LOUDNESS_STEP_MS, SIGNAL_TRACK_VERSION};

    /// A minute of quiet commentary with one loud burst at 30 s and a frozen, equally loud
    /// stretch at 10 s that must never be proposed.
    fn sample_track() -> SignalTrack {
        let buckets = (60_000 / LOUDNESS_STEP_MS) as usize + 1;
        let loudness = (0..buckets)
            .map(|index| {
                let at_ms = index as u64 * LOUDNESS_STEP_MS;
                if (30_000..36_000).contains(&at_ms) || (10_000..16_000).contains(&at_ms) {
                    -14.0
                } else {
                    -38.0
                }
            })
            .collect();
        SignalTrack {
            version: SIGNAL_TRACK_VERSION,
            duration_ms: 60_000,
            scene_scores: vec![
                SceneSample {
                    at_ms: 29_500,
                    score: 0.9,
                },
                SceneSample {
                    at_ms: 33_000,
                    score: 0.7,
                },
                SceneSample {
                    at_ms: 38_000,
                    score: 0.8,
                },
            ],
            loudness_step_ms: LOUDNESS_STEP_MS,
            loudness,
            silences: vec![
                TimeRange {
                    start_ms: 2_000,
                    end_ms: 2_400,
                },
                TimeRange {
                    start_ms: 20_000,
                    end_ms: 24_000,
                },
            ],
            blacks: vec![TimeRange {
                start_ms: 9_500,
                end_ms: 10_000,
            }],
            freezes: vec![TimeRange {
                start_ms: 10_000,
                end_ms: 16_000,
            }],
            audio_source: "Track 2 — Mic/Aux (detected by name)".into(),
        }
    }

    #[test]
    fn proposes_the_loud_moment() {
        let highlights = build_highlights(&sample_track());
        let clip = highlights
            .clips
            .iter()
            .max_by(|left, right| left.score.total_cmp(&right.score))
            .expect("a clip");
        assert!(clip.start_ms <= 30_000 && clip.end_ms >= 33_000);
        assert!(clip.peak_lufs > -20.0);
        assert_eq!(clip.reason, "loud moment over fast cuts");
    }

    #[test]
    fn never_proposes_a_frozen_stretch() {
        let highlights = build_highlights(&sample_track());
        assert!(highlights
            .clips
            .iter()
            .all(|clip| clip.start_ms >= 16_000 || clip.end_ms <= 9_500));
    }

    #[test]
    fn snaps_a_clip_edge_onto_a_nearby_scene_cut() {
        let highlights = build_highlights(&sample_track());
        let clip = highlights
            .clips
            .iter()
            .find(|clip| clip.start_ms >= 27_000 && clip.start_ms <= 32_000)
            .expect("the loud clip");
        assert_eq!(clip.start_ms, 29_500);
    }

    #[test]
    fn keeps_every_clip_within_the_allowed_length() {
        for clip in build_highlights(&sample_track()).clips {
            let length = clip.end_ms - clip.start_ms;
            assert!((MIN_CLIP_MS..=MAX_CLIP_MS).contains(&length), "{length}");
        }
    }

    #[test]
    fn numbers_clips_in_playback_order() {
        let clips = build_highlights(&sample_track()).clips;
        assert!(clips
            .windows(2)
            .all(|pair| pair[0].start_ms < pair[1].start_ms));
        assert_eq!(clips[0].id, "c0000");
    }

    #[test]
    fn reports_only_silence_worth_cutting() {
        let highlights = build_highlights(&sample_track());
        assert_eq!(
            highlights.dead_air,
            vec![TimeRange {
                start_ms: 20_000,
                end_ms: 24_000
            }]
        );
    }

    #[test]
    fn fuses_a_black_fade_into_the_freeze_that_follows_it() {
        let highlights = build_highlights(&sample_track());
        assert_eq!(
            highlights.unusable,
            vec![TimeRange {
                start_ms: 9_500,
                end_ms: 16_000
            }]
        );
    }

    #[test]
    fn proposes_nothing_for_a_recording_with_no_activity() {
        let mut track = sample_track();
        track.loudness = vec![SILENT_LUFS; track.loudness.len()];
        track.scene_scores.clear();
        assert!(build_highlights(&track).clips.is_empty());
    }
}
