import type { EditPlan, Highlights, LibrarySnapshot } from "./types";

export const demoSnapshot: LibrarySnapshot = {
  settings: {
    mediaFolders: ["D:\\Captures\\YouTube"],
    ffmpegPath: "",
    ffprobePath: "",
    whisperPath: "",
    modelPath: "",
    language: "auto",
    audioMode: "auto",
    microphoneTrack: 3,
  },
  recordings: [
    {
      id: "demo-elden-ring",
      path: "D:\\Captures\\YouTube\\elden-ring-2026-07-14.mkv",
      fileName: "elden-ring-2026-07-14.mkv",
      displayTitle: "Elden Ring — Messmer attempt and Scadutree exploration",
      extension: "mkv",
      sizeBytes: 18_742_804_480,
      modifiedAt: Date.now() - 86400000,
      durationMs: 5_186_000,
      status: "ready",
      progress: 100,
      summary: "Explores the Shadow Keep, tests a new bleed build, and spends the final section learning Messmer’s second phase. The cleanest boss attempt starts around 01:12:40.",
      language: "en",
      audioTracks: [
        { number: 1, streamIndex: 1, title: "Desktop Audio", codec: "aac", channels: 2 },
        { number: 2, streamIndex: 2, title: "Discord", codec: "aac", channels: 2 },
        { number: 3, streamIndex: 3, title: "Mic/Aux", codec: "aac", channels: 1 },
      ],
      audioSource: "Track 3 — Mic/Aux (detected by name)",
      error: null,
      chapters: [
        { startMs: 0, title: "Build check", description: "Reviews the bleed setup and changes talismans before entering Shadow Keep." },
        { startMs: 742000, title: "Shadow Keep exploration", description: "Finds a shortcut and discusses where the missing map fragment might be." },
        { startMs: 3215000, title: "First Messmer attempts", description: "Learns the grab timing and reaches phase two." },
        { startMs: 4360000, title: "Best attempt", description: "A long attempt with a near-finish and strong live commentary." },
      ],
      transcript: [
        { startMs: 0, endMs: 4800, text: "Okay, today we're going back into Shadow Keep with the bleed build." },
        { startMs: 742000, endMs: 749000, text: "That has to be the shortcut. I completely missed this lift yesterday." },
        { startMs: 3215000, endMs: 3223000, text: "All right, first real Messmer attempt. Let's see how bad this is." },
        { startMs: 4360000, endMs: 4370000, text: "This is the run. We have all the flasks and I finally understand that combo." },
        { startMs: 5120000, endMs: 5131000, text: "So close. I think this is where the episode ends, but next time he is absolutely done." },
      ],
    },
    {
      id: "demo-helldivers",
      path: "D:\\Captures\\YouTube\\obs_2026-07-16_22-14-08.mp4",
      fileName: "obs_2026-07-16_22-14-08.mp4",
      displayTitle: "Helldivers 2 — Super Helldive with the new squad",
      extension: "mp4",
      sizeBytes: 11_093_442_112,
      modifiedAt: Date.now() - 172800000,
      durationMs: 3_824_000,
      status: "ready",
      progress: 100,
      summary: "Three-match session with a new squad. The second mission has the best teamwork and funniest commentary; the last match ends in a failed extraction.",
      language: "en",
      audioTracks: [
        { number: 1, streamIndex: 1, title: "Game", codec: "aac", channels: 2 },
        { number: 2, streamIndex: 2, title: "Microphone", codec: "aac", channels: 1 },
      ],
      audioSource: "Track 2 — Microphone (detected by name)",
      error: null,
      chapters: [
        { startMs: 0, title: "Loadout and introductions", description: "Meets the squad and compares stratagems." },
        { startMs: 1240000, title: "Best mission", description: "Strong teamwork during the nursery objective." },
        { startMs: 3010000, title: "Failed extraction", description: "Chaotic finale after the reinforcement budget runs out." },
      ],
      transcript: [
        { startMs: 0, endMs: 6200, text: "I've never played with these guys before, so this could be incredible or a disaster." },
        { startMs: 1240000, endMs: 1249000, text: "That was actually coordinated. Nobody panic, we're becoming a real team." },
        { startMs: 3010000, endMs: 3018000, text: "No reinforcements, two minutes left, and somehow the pelican is on fire." },
      ],
    },
    {
      id: "demo-new",
      path: "D:\\Captures\\YouTube\\2026-07-18_09-42-11.mkv",
      fileName: "2026-07-18_09-42-11.mkv",
      displayTitle: "2026-07-18_09-42-11",
      extension: "mkv",
      sizeBytes: 6_308_012_032,
      modifiedAt: Date.now() - 7200000,
      durationMs: 2_106_000,
      status: "new",
      progress: 0,
      summary: "",
      language: null,
      audioTracks: [],
      audioSource: null,
      error: null,
      chapters: [],
      transcript: [],
    },
  ],
};

/// Sample scan output, so the preview shows what a finished analysis looks like.
export const demoHighlights: Record<string, Highlights> = {
  "demo-elden-ring": {
    version: 1,
    durationMs: 5_186_000,
    audioSource: "Track 3 — Mic/Aux (detected by name)",
    clips: [
      { id: "c0000", startMs: 214_000, endMs: 222_500, score: 0.41, peakLufs: -19.4, sceneCuts: 2, reason: "loud moment over fast cuts" },
      { id: "c0001", startMs: 968_500, endMs: 977_000, score: 0.33, peakLufs: -24.1, sceneCuts: 3, reason: "fast cuts" },
      { id: "c0002", startMs: 2_461_000, endMs: 2_472_500, score: 0.58, peakLufs: -14.8, sceneCuts: 4, reason: "loud moment over fast cuts" },
      { id: "c0003", startMs: 3_390_000, endMs: 3_398_000, score: 0.27, peakLufs: -26.6, sceneCuts: 1, reason: "steady activity" },
      { id: "c0004", startMs: 4_360_000, endMs: 4_371_000, score: 0.62, peakLufs: -13.2, sceneCuts: 5, reason: "loud moment over fast cuts" },
      { id: "c0005", startMs: 4_874_000, endMs: 4_881_500, score: 0.44, peakLufs: -18.9, sceneCuts: 2, reason: "loud moment over fast cuts" },
    ],
    deadAir: [
      { startMs: 132_000, endMs: 139_400 },
      { startMs: 1_204_000, endMs: 1_211_800 },
      { startMs: 2_890_000, endMs: 2_903_500 },
      { startMs: 3_902_000, endMs: 3_915_000 },
    ],
    unusable: [
      { startMs: 0, endMs: 24_000 },
      { startMs: 1_640_000, endMs: 1_702_000 },
      { startMs: 3_010_000, endMs: 3_061_000 },
      { startMs: 4_980_000, endMs: 5_016_000 },
    ],
  },
};


/// A sample rough cut for the same recording, so the preview shows a finished plan. Cuts and
/// removals partition the source exactly, the way a real plan does.
export const demoPlans: Record<string, EditPlan> = {
  "demo-elden-ring": {
    version: 1,
    recordingId: "demo-elden-ring",
    sourceDurationMs: 5_186_000,
    timelineDurationMs: 4_987_700,
    cuts: [
      { id: "s0000", startMs: 0, endMs: 131_750, kind: "narration", label: "Right, so this is the third session on…" },
      { id: "s0001", startMs: 139_650, endMs: 620_400, kind: "narration", label: "The bleed build is finally doing what…" },
      { id: "s0002", startMs: 623_100, endMs: 1_203_750, kind: "narration", label: "Shadow Keep is genuinely the best area…" },
      { id: "s0003", startMs: 1_212_050, endMs: 1_640_000, kind: "narration", label: "So the shortcut behind the gaol drops…" },
      { id: "s0004", startMs: 1_702_000, endMs: 2_104_000, kind: "narration", label: "And that is the run that nearly worked." },
      { id: "s0005", startMs: 2_106_900, endMs: 2_889_750, kind: "narration", label: "This is where it starts going wrong." },
      { id: "s0006", startMs: 2_903_750, endMs: 3_010_000, kind: "narration", label: "Second phase, and the grab timing is…" },
      { id: "s0007", startMs: 3_061_000, endMs: 3_901_750, kind: "narration", label: "I keep rolling into the follow-up." },
      { id: "s0008", startMs: 3_915_250, endMs: 4_980_000, kind: "narration", label: "That attempt is the one to keep." },
      { id: "s0009", startMs: 5_016_000, endMs: 5_186_000, kind: "narration", label: "Anyway, that is the session." },
    ],
    removed: [
      { startMs: 131_750, endMs: 139_650, reason: "dead air" },
      { startMs: 620_400, endMs: 623_100, reason: "filler" },
      { startMs: 1_203_750, endMs: 1_212_050, reason: "dead air" },
      { startMs: 1_640_000, endMs: 1_702_000, reason: "repeated take" },
      { startMs: 2_104_000, endMs: 2_106_900, reason: "filler" },
      { startMs: 2_889_750, endMs: 2_903_750, reason: "dead air" },
      { startMs: 3_010_000, endMs: 3_061_000, reason: "repeated take" },
      { startMs: 3_901_750, endMs: 3_915_250, reason: "dead air" },
      { startMs: 4_980_000, endMs: 5_016_000, reason: "no speech" },
    ],
  },
};
