# Automatic edit assembly

Leonardo already turns a capture folder into transcripts, chapters, and previews. This
document describes how the same library becomes an assistant that proposes a cut: dead air
removed from a narration take, gameplay B-roll matched to what the creator is saying, and a
timeline that opens in DaVinci Resolve.

## The problem with a purely LLM-driven pass

The obvious design is a coarse-to-fine sweep: sample one frame per minute, ask a model where
it wants a closer look, resample that region at one frame per second, repeat. It is cheap and
it sounds principled, but it only works when the coarse signal correlates with the fine one.

For "what kind of footage is this" — menu, loading screen, cutscene, gameplay, talking head —
the correlation holds. The property changes slowly, so a frame per minute sees it.

For "where is something interesting happening" the correlation fails. A clutch, an explosion,
a funny death lasts one to three seconds and simply is not present in a one-frame-per-minute
grid. The model cannot ask to drill into a region whose evidence it never saw, so the sweep
systematically misses exactly the moments the edit exists for.

Cost is not the binding constraint either. An hour sampled once per minute is about 60 frames,
roughly 60k tokens, and tiling frames into contact sheets (`-vf tile=4x4`) cuts that by an
order of magnitude. Even a dense one-frame-per-second sweep of an hour lands in the low
hundreds of thousands of tokens. Recall is the constraint, and recall is bought with cheap
local detectors, not with tokens.

## Inverted pipeline

Run the dense pass locally with FFmpeg filters that already ship with the app. Let the model
see only candidates, and let it work on text.

```text
Layer 1  Signal pass          FFmpeg detectors over the whole file, cached, no model
         scene cuts, loudness curve, silence, black, freeze, keyframe density
Layer 2  Candidate segments   signals merged into scored 3-15s clips and dead-air ranges
Layer 3  Candidate tagging    1-3 frames per candidate, contact-sheeted, into tags/embeddings
Layer 4  Planner              text-only: transcript + candidate index + brief -> edit plan
Layer 5  Export               multi-clip FCPXML, FFmpeg proxy render
```

Layers 1 and 2 are deterministic and run on every recording. Layer 3 runs per candidate, not
per second. Layer 4 sees a few thousand tokens of text for an hour of footage.

### The timecode contract

**The planner never invents a timecode. It only selects IDs from a list it was given.**

Every millisecond in an exported timeline originates in Layer 1 or 2. A model asked to emit
`00:14:32.500` will happily emit a plausible number for a moment that does not exist, and the
failure is invisible until someone scrubs the timeline. Selecting `c0412` from a supplied
index either resolves or is rejected on the spot.

## Layer 1 signals

Everything here is a filter in the bundled FFmpeg binary.

| Signal | Filter | Used for |
| --- | --- | --- |
| Scene cuts | `select='gt(scene,T)'` + `metadata=print` | segment boundaries |
| Loudness | `ebur128=metadata=1` | reactions, explosions, laughter |
| Silence | `silencedetect=noise=-35dB:d=0.6` | dead air in a narration take |
| Black | `blackdetect` | transitions, menu fades |
| Freeze | `freezedetect` | loading screens, AFK, paused game |

The video pass decodes with `-skip_frame nokey`. Only I-frames are decoded, which is roughly
an order of magnitude faster than a full decode, and in OBS captures the encoder already
places I-frames more densely where the picture changes hard. The keyframe grid is therefore
both cheap and mildly correlated with activity — the coarse pass the original sketch wanted,
for the price of a decode rather than the price of tokens. The cost is temporal resolution of
about two seconds, which is the right granularity for proposing a candidate and the wrong one
for choosing a frame-accurate cut point. Layer 2 refines boundaries; Layer 1 only proposes.

Signal tracks are content-addressed by recording id and file mtime and stored under the app
data folder, following the same versioned-filename pattern already used for thumbnails, so a
schema change invalidates old files instead of silently misreading them.

## Two streams, not one

A narration video is two separate problems that share a library.

**A-roll — the talking head.** Cut from text, not from pictures. The transcript already
carries timestamps, which is enough to drop silences, cut fillers, and keep the last of
several takes of the same sentence. No vision needed, and this is where most of the visible
value is.

**B-roll — the gameplay.** This is retrieval, not annotation. The task is not "label forty
hours of captures" but "find six seconds that fit the line *the boss fight annoyed me*".
Annotation is linear in library size; retrieval is linear in script length. For a library that
grows to terabytes only the second one survives.

## Vision options for Layer 3

1. **Hosted model on frames.** Best semantic understanding, but it contradicts the local-first,
   no-account promise in the README. It must be explicit opt-in with a key in Settings, and
   local-only stays the default.
2. **Local CLIP/SigLIP through ONNX Runtime** (the `ort` crate, no Python runtime). Frame
   embeddings, semantic search from a text query, zero tokens, scales across the whole library.
   The best value for B-roll retrieval.
3. **Local VLM** through a llama.cpp sidecar, reusing the pattern already established for
   whisper.cpp. Fully local captions at the cost of another binary and VRAM.

The intended default is a hybrid: local embeddings for search and tags, a hosted model for
text-only planning. What leaves the machine is then a transcript and a tag index, never frames.

## Export

**FFmpeg** is for preview, not for delivery. `-ss` before `-i` with `-c copy` only cuts on GOP
boundaries, accurate cuts require re-encoding, the `concat` demuxer needs identical stream
parameters across pieces, and mixed frame rates drift the audio. The practical use is a single
low-bitrate proxy built with `filter_complex` `trim`/`atrim`/`concat` so the creator can watch
the proposed cut in a minute.

**FCPXML** is the delivery format and the app already emits a minimal one. Growing it means a
spine with several `asset-clip` elements in sequence plus `lane="1"` for B-roll above the
narration. Resolve is strict about `format`/`frameDuration` (mixed source frame rates need a
format per asset and `conform-rate`), about `file://` URLs containing spaces, and about
`tcFormat`.

Fallbacks worth keeping in mind: a CMX3600 EDL is primitive but opens everywhere, and a
generated Resolve Lua script driving the scripting API sidesteps XML strictness entirely
because Lua ships inside Resolve. OpenTimelineIO is the right interchange format but its
reference implementation is Python, which this project deliberately does not require.

## Order of work

Each step is useful on its own.

1. **Signal pass.** FFmpeg detectors, parsers, versioned cache, one Tauri command.
2. **Candidates and highlights.** Signals merged into scored clips and dead-air ranges, shown
   in the UI. Real editing value with no model involved.
3. **Deterministic edit plan.** Transcript plus silence into an ordered list of cuts. Fillers
   and dead air gone.
4. **Exports.** Multi-clip FCPXML and the proxy render, so a plan can actually be watched and
   opened in Resolve.
5. **Planner.** An opt-in provider in Settings, structured output, the timecode contract
   enforced by construction — the model returns candidate IDs and the app resolves them.
6. **Vision tags and B-roll matching.** Candidate frames into tags and embeddings, lines
   matched to clips.
7. **Human brief.** A free-text brief ("highlight the stupid deaths, cut on impact, fit this
   script") folded into the planner prompt. Automatic and guided editing are the same layer
   with an empty or non-empty brief.

## Operational notes

- The signal pass is I/O heavy. Compute once, version the cache, never recompute silently.
- `library.json` is a single pretty-printed file. Per-second signals for a whole library do not
  belong in it; signal tracks are separate per-recording files and the library keeps a pointer.
- Long passes belong on the blocking worker pool, next to transcription, with progress
  reported the same way.
