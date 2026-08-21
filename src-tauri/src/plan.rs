//! A first cut of a narration take, built from the transcript and the silence around it.
//!
//! Nothing here judges what a recording is about. It removes what nobody wants in a finished
//! video — dead air, filler, the three attempts at a sentence before the one that worked — and
//! leaves an ordered list of cuts that a planner can later reorder, drop from, or interleave with
//! B-roll. Each cut carries an id for exactly that reason.
//!
//! Everything removed is reported alongside what survived. A cut list on its own is impossible to
//! trust; a cut list next to "18 s of dead air, two repeated takes" can be checked in seconds.

use serde::{Deserialize, Serialize};

use crate::signals::{invert_ranges, TimeRange};
use crate::TranscriptSegment;

pub const EDIT_PLAN_VERSION: u32 = 1;

/// Kept either side of speech, so a cut never lands on a consonant.
const HANDLE_MS: u64 = 250;

/// A pause shorter than this is breathing, and cutting it makes delivery sound hurried.
const MIN_GAP_MS: u64 = 600;

/// A surviving fragment shorter than this is a flicker, not a shot.
const MIN_CUT_MS: u64 = 400;

/// How far apart two attempts at the same sentence can be and still count as retakes.
const REPEAT_WINDOW_MS: u64 = 30_000;

/// Word overlap above which two segments are the same line twice.
const REPEAT_SIMILARITY: f32 = 0.8;

/// Whole segments made only of these are hesitation, in the languages this app ships with.
/// Individually most of them are real words, which is why only a segment that is *entirely*
/// filler is ever dropped.
const FILLERS: &[&str] = &[
    "um", "uhm", "uh", "erm", "er", "hmm", "hm", "mhm", "mm", "ah", "eh", "oh", "like", "so",
    "well", "эм", "ээ", "эээ", "мм", "ммм", "аа", "ну", "вот", "как", "бы", "типа",
];

/// One stretch of source that survives into the timeline.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Cut {
    /// Stable within a plan, and the handle a planner uses instead of a timestamp.
    pub id: String,
    pub start_ms: u64,
    pub end_ms: u64,
    /// "narration" for a kept stretch of the take.
    pub kind: String,
    /// The opening words, so the list reads like a script rather than a table of numbers.
    pub label: String,
}

/// One stretch that did not survive, and why.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Removal {
    pub start_ms: u64,
    pub end_ms: u64,
    pub reason: String,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EditPlan {
    pub version: u32,
    pub recording_id: String,
    pub source_duration_ms: u64,
    pub timeline_duration_ms: u64,
    pub cuts: Vec<Cut>,
    pub removed: Vec<Removal>,
}

/// Build the first cut.
///
/// `dead_air` comes from the signal pass when one has been run. It is worth having: whisper often
/// spans a long pause inside a single segment, which the transcript alone cannot reveal.
pub fn build_narration_plan(
    recording_id: &str,
    transcript: &[TranscriptSegment],
    duration_ms: u64,
    dead_air: &[TimeRange],
) -> EditPlan {
    let duration_ms = duration_ms.max(
        transcript
            .last()
            .map(|segment| segment.end_ms)
            .unwrap_or_default(),
    );
    let mut removals = Vec::new();
    for (index, segment) in transcript.iter().enumerate() {
        if let Some(reason) = discard_reason(segment, &transcript[index + 1..]) {
            removals.push(Removal {
                start_ms: segment.start_ms,
                end_ms: segment.end_ms,
                reason: reason.into(),
            });
        }
    }

    // Silence is measured between *every* segment, including the ones just dropped. Measuring it
    // between the survivors instead would let one long gap swallow the filler and the retake
    // inside it, and a removal that cannot say what it removed is worth very little.
    let speech = transcript
        .iter()
        .map(|segment| TimeRange {
            start_ms: segment.start_ms,
            end_ms: segment.end_ms,
        })
        .collect::<Vec<_>>();
    let gaps = invert_ranges(&speech, duration_ms)
        .into_iter()
        .chain(dead_air.iter().copied());
    for gap in gaps {
        if let Some(range) = shrink(gap, HANDLE_MS, MIN_GAP_MS) {
            removals.push(Removal {
                start_ms: range.start_ms,
                end_ms: range.end_ms,
                reason: "dead air".into(),
            });
        }
    }

    let removed = merge_removals(removals);
    let kept = invert_ranges(
        &removed
            .iter()
            .map(|removal| TimeRange {
                start_ms: removal.start_ms,
                end_ms: removal.end_ms,
            })
            .collect::<Vec<_>>(),
        duration_ms,
    );
    // Slivers left between two removals are not shots. Reporting them keeps the arithmetic
    // honest: the source is the timeline plus everything named in `removed`.
    let (kept, slivers): (Vec<_>, Vec<_>) = kept
        .into_iter()
        .partition(|range| range.duration_ms() >= MIN_CUT_MS);
    let removed = merge_removals(
        removed
            .into_iter()
            .chain(slivers.into_iter().map(|range| Removal {
                start_ms: range.start_ms,
                end_ms: range.end_ms,
                reason: "too short to keep".into(),
            }))
            .collect(),
    );
    let cuts = kept
        .into_iter()
        .enumerate()
        .map(|(index, range)| Cut {
            id: format!("s{index:04}"),
            start_ms: range.start_ms,
            end_ms: range.end_ms,
            kind: "narration".into(),
            label: label_for(range, transcript),
        })
        .collect::<Vec<_>>();

    EditPlan {
        version: EDIT_PLAN_VERSION,
        recording_id: recording_id.to_string(),
        source_duration_ms: duration_ms,
        timeline_duration_ms: cuts.iter().map(|cut| cut.end_ms - cut.start_ms).sum(),
        cuts,
        removed,
    }
}

/// Why this segment should not reach the timeline, if it should not.
fn discard_reason(
    segment: &TranscriptSegment,
    later: &[TranscriptSegment],
) -> Option<&'static str> {
    if is_annotation(&segment.text) {
        return Some("no speech");
    }
    let words = words_of(&segment.text);
    if words.is_empty() {
        return Some("no speech");
    }
    if words.iter().all(|word| FILLERS.contains(&word.as_str())) {
        return Some("filler");
    }
    // A retake is only a retake if the better attempt comes after it, so the search looks forward
    // and the last version of a line is the one that survives.
    later
        .iter()
        .take_while(|next| next.start_ms.saturating_sub(segment.end_ms) <= REPEAT_WINDOW_MS)
        .any(|next| similarity(&words, &words_of(&next.text)) >= REPEAT_SIMILARITY)
        .then_some("repeated take")
}

/// Whether a segment is the engine describing the audio rather than transcribing it —
/// `[BLANK_AUDIO]`, `(soft music)`, a run of musical notes. Their words are real words, so this
/// has to look at the wrapping rather than at the vocabulary.
fn is_annotation(text: &str) -> bool {
    let text = text.trim();
    text.chars().all(|character| !character.is_alphanumeric())
        || (text.starts_with('[') && text.ends_with(']'))
        || (text.starts_with('(') && text.ends_with(')'))
        || (text.starts_with('*') && text.ends_with('*'))
}

/// Lowercase word tokens, with punctuation stripped.
fn words_of(text: &str) -> Vec<String> {
    text.split(|character: char| !character.is_alphanumeric() && character != '\'')
        .filter(|word| !word.is_empty())
        .map(|word| word.to_lowercase())
        .collect()
}

/// Word overlap, ignoring order and repetition — close enough to catch a restarted sentence
/// without pretending to understand it.
fn similarity(left: &[String], right: &[String]) -> f32 {
    if left.is_empty() || right.is_empty() {
        return 0.0;
    }
    let shared = left.iter().filter(|word| right.contains(word)).count();
    shared as f32 / left.len().max(right.len()) as f32
}

/// Pull a gap in from both sides by a handle, and report it only if enough is left to be worth
/// cutting.
fn shrink(range: TimeRange, handle_ms: u64, minimum_ms: u64) -> Option<TimeRange> {
    let start_ms = range.start_ms + handle_ms;
    let end_ms = range.end_ms.checked_sub(handle_ms)?;
    (end_ms > start_ms && end_ms - start_ms >= minimum_ms).then_some(TimeRange { start_ms, end_ms })
}

/// Order removals and fuse only the touching ones that agree on why they are there. Two adjacent
/// removals with different reasons stay separate, because the list is the explanation.
fn merge_removals(mut removals: Vec<Removal>) -> Vec<Removal> {
    removals.sort_by_key(|removal| removal.start_ms);
    let mut merged: Vec<Removal> = Vec::with_capacity(removals.len());
    for removal in removals {
        match merged.last_mut() {
            Some(last) if removal.start_ms <= last.end_ms && last.reason == removal.reason => {
                last.end_ms = last.end_ms.max(removal.end_ms);
            }
            _ => merged.push(removal),
        }
    }
    merged
}

fn label_for(range: TimeRange, transcript: &[TranscriptSegment]) -> String {
    let Some(segment) = transcript
        .iter()
        .find(|segment| segment.end_ms > range.start_ms && segment.start_ms < range.end_ms)
    else {
        return String::new();
    };
    let words = segment.text.split_whitespace().take(9).collect::<Vec<_>>();
    let label = words.join(" ");
    if segment.text.split_whitespace().count() > words.len() {
        format!("{label}…")
    } else {
        label
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn segment(start_ms: u64, end_ms: u64, text: &str) -> TranscriptSegment {
        TranscriptSegment {
            start_ms,
            end_ms,
            text: text.into(),
        }
    }

    fn take() -> Vec<TranscriptSegment> {
        vec![
            segment(
                1_000,
                4_000,
                "So the thing about this boss is that it never telegraphs.",
            ),
            segment(4_200, 4_600, "Um."),
            // Twelve seconds of nothing between these two.
            segment(
                16_500,
                19_000,
                "I spent about four hours on the second phase.",
            ),
            segment(
                19_200,
                21_800,
                "I spent roughly four hours on the second phase.",
            ),
            segment(22_000, 25_000, "[BLANK_AUDIO]"),
            segment(
                25_500,
                29_000,
                "And that is where the run finally came together.",
            ),
        ]
    }

    #[test]
    fn drops_filler_and_engine_annotations() {
        let plan = build_narration_plan("r", &take(), 30_000, &[]);
        assert!(plan
            .removed
            .iter()
            .any(|removal| removal.reason == "filler" && removal.start_ms == 4_200));
        assert!(plan
            .removed
            .iter()
            .any(|removal| removal.reason == "no speech" && removal.start_ms == 22_000));
    }

    #[test]
    fn keeps_the_last_attempt_at_a_repeated_line() {
        let plan = build_narration_plan("r", &take(), 30_000, &[]);
        let repeat = plan
            .removed
            .iter()
            .find(|removal| removal.reason == "repeated take")
            .expect("the first attempt");
        assert_eq!(repeat.start_ms, 16_500);
        assert!(plan
            .cuts
            .iter()
            .any(|cut| cut.start_ms <= 19_200 && cut.end_ms >= 21_800));
    }

    #[test]
    fn cuts_the_long_pause_but_leaves_a_handle() {
        let plan = build_narration_plan("r", &take(), 30_000, &[]);
        let pause = plan
            .removed
            .iter()
            .find(|removal| removal.start_ms > 4_000 && removal.reason == "dead air")
            .expect("the twelve second pause");
        assert_eq!(pause.start_ms, 4_600 + HANDLE_MS);
        assert_eq!(pause.end_ms, 16_500 - HANDLE_MS);
    }

    #[test]
    fn leaves_a_breath_alone() {
        let transcript = vec![
            segment(0, 2_000, "One clean sentence."),
            segment(2_300, 4_000, "And the next one, right behind it."),
        ];
        let plan = build_narration_plan("r", &transcript, 4_000, &[]);
        assert_eq!(plan.cuts.len(), 1);
        assert_eq!(plan.timeline_duration_ms, 4_000);
    }

    #[test]
    fn cuts_a_pause_whisper_hid_inside_one_segment() {
        let transcript = vec![segment(
            0,
            20_000,
            "A sentence, a long think, and then the rest.",
        )];
        let dead_air = vec![TimeRange {
            start_ms: 5_000,
            end_ms: 14_000,
        }];
        let plan = build_narration_plan("r", &transcript, 20_000, &dead_air);
        assert_eq!(plan.cuts.len(), 2);
        assert_eq!(plan.cuts[0].end_ms, 5_250);
        assert_eq!(plan.cuts[1].start_ms, 13_750);
    }

    #[test]
    fn numbers_cuts_in_order_and_labels_them_from_the_script() {
        let plan = build_narration_plan("r", &take(), 30_000, &[]);
        assert_eq!(plan.cuts[0].id, "s0000");
        assert!(plan.cuts[0].label.starts_with("So the thing about"));
        assert!(plan
            .cuts
            .windows(2)
            .all(|pair| pair[0].end_ms <= pair[1].start_ms));
    }

    #[test]
    fn reports_a_timeline_shorter_than_the_source() {
        let plan = build_narration_plan("r", &take(), 30_000, &[]);
        assert_eq!(plan.source_duration_ms, 30_000);
        assert!(plan.timeline_duration_ms < 20_000);
        assert_eq!(
            plan.timeline_duration_ms,
            plan.cuts
                .iter()
                .map(|cut| cut.end_ms - cut.start_ms)
                .sum::<u64>()
        );
    }

    #[test]
    fn accounts_for_every_millisecond_of_the_source() {
        let plan = build_narration_plan("r", &take(), 30_000, &[]);
        let removed: u64 = plan
            .removed
            .iter()
            .map(|removal| removal.end_ms - removal.start_ms)
            .sum();
        assert_eq!(plan.timeline_duration_ms + removed, plan.source_duration_ms);
        assert!(plan
            .removed
            .windows(2)
            .all(|pair| pair[0].end_ms <= pair[1].start_ms));
    }

    #[test]
    fn survives_a_recording_with_no_transcript() {
        let plan = build_narration_plan("r", &[], 0, &[]);
        assert!(plan.cuts.is_empty());
        assert_eq!(plan.timeline_duration_ms, 0);
    }
}
