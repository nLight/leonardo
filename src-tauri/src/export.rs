//! Turning a plan into something another program can open.
//!
//! Two outputs, for two different moments. An FCPXML timeline is what a finished plan is handed
//! off in — it opens in Resolve with every cut in place and the original media still attached. An
//! FFmpeg proxy render is what answers "does this cut actually work" in a minute, without leaving
//! the app.
//!
//! Both are built by pure functions here and executed by the caller, because the interesting
//! failures — an offset that drifts, a filter graph that will not parse — are all in the text.

use serde::{Deserialize, Serialize};

use crate::plan::Cut;

/// Height of the proxy render. Tall enough to judge framing and read a HUD, small enough that an
/// hour of footage encodes while someone makes coffee.
pub const PREVIEW_HEIGHT: u32 = 540;

/// The timebase a timeline is written in.
///
/// FCPXML expresses every time as a rational number of seconds, and Resolve is happiest when
/// those rationals land on frame boundaries. Carrying the source frame rate here means both the
/// asset description and every cut point are stated in the same timebase the footage actually
/// has, rather than in a hardcoded 30 that a 60 fps capture would have to be conformed out of.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VideoFormat {
    pub width: u32,
    pub height: u32,
    /// Frames per second as the numerator and denominator FFprobe reports, so 59.94 stays
    /// 60000/1001 instead of becoming a rounded 60 that drifts a second every half hour.
    pub frame_num: u64,
    pub frame_den: u64,
}

impl Default for VideoFormat {
    fn default() -> Self {
        Self {
            width: 1920,
            height: 1080,
            frame_num: 30,
            frame_den: 1,
        }
    }
}

impl VideoFormat {
    /// Nearest frame to a wall-clock position.
    pub fn frames(&self, ms: u64) -> u64 {
        let divisor = 1_000 * self.frame_den.max(1);
        (ms * self.frame_num + divisor / 2) / divisor
    }

    /// A frame count as an FCPXML time.
    pub fn time(&self, frames: u64) -> String {
        if frames == 0 {
            return "0s".into();
        }
        format!("{}/{}s", frames * self.frame_den.max(1), self.frame_num)
    }

    pub fn frame_duration(&self) -> String {
        format!("{}/{}s", self.frame_den.max(1), self.frame_num)
    }
}

/// Read a format out of FFprobe's flat key-value output, falling back to 1080p30 for anything
/// missing or nonsensical. A wrong guess conforms badly in Resolve; no guess at all does not open.
pub fn parse_video_format(probe_output: &str) -> VideoFormat {
    let mut format = VideoFormat::default();
    for line in probe_output.lines() {
        let Some((key, value)) = line.trim().split_once('=') else {
            continue;
        };
        match key {
            // A zero is FFprobe saying it does not know, not a one-pixel picture.
            "width" => format.width = positive(value).unwrap_or(format.width),
            "height" => format.height = positive(value).unwrap_or(format.height),
            "r_frame_rate" => {
                if let Some((num, den)) = value.split_once('/') {
                    if let (Ok(num), Ok(den)) = (num.parse::<u64>(), den.parse::<u64>()) {
                        if num > 0 && den > 0 {
                            format.frame_num = num;
                            format.frame_den = den;
                        }
                    }
                }
            }
            _ => {}
        }
    }
    format
}

/// The source file a timeline points at.
pub struct Asset<'a> {
    pub file_name: &'a str,
    pub project_name: &'a str,
    /// A `file://` URL, already escaped.
    pub source_url: &'a str,
    pub duration_ms: u64,
}

/// A named point on the timeline.
pub struct Marker {
    pub start_ms: u64,
    pub title: String,
    pub note: String,
}

/// Write a timeline containing every cut, in order, against one source asset.
///
/// Offsets are accumulated in frames rather than milliseconds. Converting each cut's position
/// independently lets rounding open one-frame holes between clips; accumulating the same integers
/// the durations are written from cannot.
///
/// Markers are attached to the first clip. A selects export is the degenerate case of this — one
/// cut spanning the whole file, with the chapters hanging off it.
pub fn fcpxml_document(
    asset: &Asset,
    format: &VideoFormat,
    cuts: &[Cut],
    markers: &[Marker],
) -> String {
    let asset_frames = format.frames(asset.duration_ms).max(1);
    let mut offset_frames = 0;
    let mut clips = String::new();
    for (index, cut) in cuts.iter().enumerate() {
        let start_frames = format.frames(cut.start_ms);
        let duration_frames = format
            .frames(cut.end_ms)
            .saturating_sub(start_frames)
            .max(1);
        let body = if index == 0 {
            markers
                .iter()
                .map(|marker| {
                    format!(
                        "<marker start=\"{}\" value=\"{}\" note=\"{}\"/>",
                        format.time(format.frames(marker.start_ms)),
                        xml_escape(&marker.title),
                        xml_escape(&marker.note)
                    )
                })
                .collect::<String>()
        } else {
            String::new()
        };
        clips.push_str(&format!(
            "\n        <asset-clip name=\"{}\" ref=\"r2\" offset=\"{}\" start=\"{}\" duration=\"{}\" audioRole=\"dialogue\">{body}</asset-clip>",
            xml_escape(asset.file_name),
            format.time(offset_frames),
            format.time(start_frames),
            format.time(duration_frames),
        ));
        offset_frames += duration_frames;
    }

    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE fcpxml>
<fcpxml version="1.10">
  <resources>
    <format id="r1" frameDuration="{frame_duration}" width="{width}" height="{height}" colorSpace="1-1-1 (Rec. 709)"/>
    <asset id="r2" name="{file_name}" start="0s" duration="{asset_duration}" hasVideo="1" hasAudio="1" format="r1">
      <media-rep kind="original-media" src="{source_url}"/>
    </asset>
  </resources>
  <library><event name="Leonardo Selects"><project name="{project_name}">
    <sequence format="r1" duration="{timeline_duration}" tcStart="0s" tcFormat="NDF" audioLayout="stereo" audioRate="48k">
      <spine>{clips}
      </spine>
    </sequence>
  </project></event></library>
</fcpxml>
"#,
        frame_duration = format.frame_duration(),
        width = format.width,
        height = format.height,
        file_name = xml_escape(asset.file_name),
        asset_duration = format.time(asset_frames),
        source_url = xml_escape(asset.source_url),
        project_name = xml_escape(asset.project_name),
        timeline_duration = format.time(offset_frames.max(1)),
    )
}

/// Build the filter graph that renders a plan as one continuous proxy file.
///
/// A labelled output can only be consumed once in a filter graph, so the source is split as many
/// ways as there are cuts before anything is trimmed. Scaling happens once, before the split,
/// where it is cheapest.
///
/// `video` and `audio` are graph labels without their brackets — a stream specifier like `0:a:1`,
/// or the output of a chain the caller prepended, such as a mix of every track.
pub fn preview_filter_graph(cuts: &[Cut], height: u32, video: &str, audio: &str) -> String {
    if cuts.is_empty() {
        return String::new();
    }
    let count = cuts.len();
    let mut graph = format!(
        "[{video}]scale=-2:{height}:flags=fast_bilinear,setsar=1,{};\n[{audio}]{};",
        fan_out("split", count, "pv"),
        fan_out("asplit", count, "pa")
    );
    for (index, cut) in cuts.iter().enumerate() {
        graph.push_str(&format!(
            "\n[pv{index}]trim=start={start}:end={end},setpts=PTS-STARTPTS[cv{index}];\
             \n[pa{index}]atrim=start={start}:end={end},asetpts=PTS-STARTPTS[ca{index}];",
            start = seconds(cut.start_ms),
            end = seconds(cut.end_ms),
        ));
    }
    graph.push('\n');
    for index in 0..count {
        graph.push_str(&format!("[cv{index}][ca{index}]"));
    }
    graph.push_str(&format!("concat=n={count}:v=1:a=1[vout][aout]"));
    graph
}

/// `split=3[pv0][pv1][pv2]`. A single-cut plan still gets a `split=1`: a chain has to contain at
/// least one filter, so dropping it would leave `[0:a:0][pa0];`, which does not parse.
fn fan_out(filter: &str, count: usize, prefix: &str) -> String {
    let labels = (0..count)
        .map(|index| format!("[{prefix}{index}]"))
        .collect::<String>();
    format!("{filter}={count}{labels}")
}

fn positive(value: &str) -> Option<u32> {
    value.parse::<u32>().ok().filter(|parsed| *parsed > 0)
}

fn seconds(ms: u64) -> String {
    format!("{:.3}", ms as f64 / 1_000.0)
}

fn xml_escape(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&apos;")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cut(id: &str, start_ms: u64, end_ms: u64) -> Cut {
        Cut {
            id: id.into(),
            start_ms,
            end_ms,
            kind: "narration".into(),
            label: "A line of commentary".into(),
        }
    }

    fn asset() -> Asset<'static> {
        Asset {
            file_name: "session & take 2.mkv",
            project_name: "Rough cut",
            source_url: "file:///D:/Captures/session%20&%20take%202.mkv",
            duration_ms: 60_000,
        }
    }

    fn ntsc() -> VideoFormat {
        VideoFormat {
            width: 1920,
            height: 1080,
            frame_num: 60_000,
            frame_den: 1_001,
        }
    }

    #[test]
    fn lays_cuts_end_to_end_without_a_gap() {
        let cuts = vec![cut("s0000", 0, 4_200), cut("s0001", 19_000, 22_000)];
        let xml = fcpxml_document(&asset(), &VideoFormat::default(), &cuts, &[]);
        // 4.2 s at 30 fps is 126 frames, so the second clip starts there and takes its own
        // in-point from the source rather than from the timeline.
        assert!(xml.contains(r#"offset="0s" start="0s" duration="126/30s""#));
        assert!(xml.contains(r#"offset="126/30s" start="570/30s" duration="90/30s""#));
        assert!(xml.contains(r#"<sequence format="r1" duration="216/30s""#));
    }

    #[test]
    fn accumulates_offsets_in_frames_so_rounding_cannot_open_a_hole() {
        // Cut boundaries deliberately off the frame grid.
        let cuts = (0..40)
            .map(|index| cut("s", index * 1_017, index * 1_017 + 1_017))
            .collect::<Vec<_>>();
        let format = VideoFormat::default();
        let xml = fcpxml_document(&asset(), &format, &cuts, &[]);
        let mut expected = 0;
        for cut in &cuts {
            assert!(xml.contains(&format!("offset=\"{}\"", format.time(expected))));
            expected += format
                .frames(cut.end_ms)
                .saturating_sub(format.frames(cut.start_ms));
        }
    }

    #[test]
    fn keeps_a_broadcast_frame_rate_exact() {
        let format = ntsc();
        assert_eq!(format.frame_duration(), "1001/60000s");
        // One second is 59.94 frames, which rounds to 60, and 60 frames is 60060/60000 s.
        assert_eq!(format.frames(1_000), 60);
        assert_eq!(format.time(60), "60060/60000s");
        let xml = fcpxml_document(&asset(), &format, &[cut("s0000", 0, 1_000)], &[]);
        assert!(xml.contains(r#"frameDuration="1001/60000s""#));
    }

    #[test]
    fn never_writes_a_zero_length_clip() {
        // Two boundaries that round to the same frame.
        let xml = fcpxml_document(
            &asset(),
            &VideoFormat::default(),
            &[cut("s0000", 1_000, 1_005)],
            &[],
        );
        assert!(xml.contains(r#"duration="1/30s""#));
    }

    #[test]
    fn hangs_markers_off_the_first_clip_only() {
        let markers = vec![Marker {
            start_ms: 12_000,
            title: "Boss fight".into(),
            note: "Second phase".into(),
        }];
        let xml = fcpxml_document(
            &asset(),
            &VideoFormat::default(),
            &[cut("s0000", 0, 20_000), cut("s0001", 30_000, 40_000)],
            &markers,
        );
        assert_eq!(xml.matches("<marker").count(), 1);
        assert!(xml.contains(r#"<marker start="360/30s" value="Boss fight" note="Second phase"/>"#));
    }

    #[test]
    fn escapes_a_filename_that_would_break_the_document() {
        let xml = fcpxml_document(
            &asset(),
            &VideoFormat::default(),
            &[cut("s0000", 0, 1_000)],
            &[],
        );
        assert!(xml.contains("session &amp; take 2.mkv"));
        assert!(!xml.contains("session & take"));
    }

    #[test]
    fn reads_a_format_from_ffprobe() {
        let format = parse_video_format("width=2560\nheight=1440\nr_frame_rate=60000/1001\n");
        assert_eq!(
            format,
            VideoFormat {
                width: 2560,
                height: 1440,
                frame_num: 60_000,
                frame_den: 1_001
            }
        );
    }

    #[test]
    fn falls_back_when_ffprobe_says_something_useless() {
        // A stream with no timing information reports 0/0, which is not a frame rate.
        let format = parse_video_format("width=0\nr_frame_rate=0/0\n");
        assert_eq!(format, VideoFormat::default());
    }

    #[test]
    fn splits_the_source_once_per_cut() {
        let cuts = vec![
            cut("s0000", 0, 4_200),
            cut("s0001", 19_000, 22_000),
            cut("s0002", 25_000, 30_000),
        ];
        let graph = preview_filter_graph(&cuts, PREVIEW_HEIGHT, "0:v", "0:a:1");
        assert!(graph.contains("split=3[pv0][pv1][pv2]"));
        assert!(graph.contains("asplit=3[pa0][pa1][pa2]"));
        assert!(graph.contains("[pv1]trim=start=19.000:end=22.000,setpts=PTS-STARTPTS[cv1];"));
        assert!(graph.ends_with("[cv0][ca0][cv1][ca1][cv2][ca2]concat=n=3:v=1:a=1[vout][aout]"));
    }

    #[test]
    fn keeps_a_split_even_for_a_single_cut() {
        // Every chain needs at least one filter in it. `[mix][pa0];` does not parse.
        let graph = preview_filter_graph(&[cut("s0000", 0, 4_200)], PREVIEW_HEIGHT, "0:v", "mix");
        assert!(graph.contains("setsar=1,split=1[pv0];"));
        assert!(graph.contains("[mix]asplit=1[pa0];"));
        assert!(graph.ends_with("[cv0][ca0]concat=n=1:v=1:a=1[vout][aout]"));
    }

    #[test]
    fn starts_every_chain_with_a_filter_rather_than_a_comma() {
        for count in 1..4 {
            let cuts = (0..count)
                .map(|index| cut("s", index * 2_000, index * 2_000 + 1_000))
                .collect::<Vec<_>>();
            let graph = preview_filter_graph(&cuts, PREVIEW_HEIGHT, "0:v:0", "0:a:0");
            assert!(
                !graph.contains("],"),
                "a label followed by a comma is an empty filter: {graph}"
            );
        }
    }

    #[test]
    fn renders_nothing_for_an_empty_plan() {
        assert!(preview_filter_graph(&[], PREVIEW_HEIGHT, "0:v", "0:a").is_empty());
    }
}
