import { invoke } from "@tauri-apps/api/core";
import "./styles.css";
import { demoSnapshot } from "./demo";
import type { AppSettings, LibrarySnapshot, Recording, RecordingStatus } from "./types";

const app = document.querySelector<HTMLDivElement>("#app")!;

const icons: Record<string, string> = {
  library: '<svg viewBox="0 0 24 24"><path d="M4 5.5A1.5 1.5 0 0 1 5.5 4h13A1.5 1.5 0 0 1 20 5.5v13a1.5 1.5 0 0 1-1.5 1h-13A1.5 1.5 0 0 1 4 18.5z"/><path d="M8 4v16M12 9h5M12 13h5"/></svg>',
  folder: '<svg viewBox="0 0 24 24"><path d="M3 7.5A1.5 1.5 0 0 1 4.5 6H10l2 2h7.5A1.5 1.5 0 0 1 21 9.5v8a1.5 1.5 0 0 1-1.5 1.5h-15A1.5 1.5 0 0 1 3 17.5z"/></svg>',
  spark: '<svg viewBox="0 0 24 24"><path d="m12 3 1.2 4.1a5.1 5.1 0 0 0 3.5 3.5L21 12l-4.3 1.4a5.1 5.1 0 0 0-3.5 3.5L12 21l-1.2-4.1a5.1 5.1 0 0 0-3.5-3.5L3 12l4.3-1.4a5.1 5.1 0 0 0 3.5-3.5z"/></svg>',
  settings: '<svg viewBox="0 0 24 24"><circle cx="12" cy="12" r="3"/><path d="M19.4 15a1.7 1.7 0 0 0 .3 1.9l.1.1-2.8 2.8-.1-.1a1.7 1.7 0 0 0-1.9-.3 1.7 1.7 0 0 0-1 1.6v.2h-4V21a1.7 1.7 0 0 0-1-1.6 1.7 1.7 0 0 0-1.9.3l-.1.1L4.2 17l.1-.1a1.7 1.7 0 0 0 .3-1.9A1.7 1.7 0 0 0 3 14H3v-4h.1a1.7 1.7 0 0 0 1.6-1 1.7 1.7 0 0 0-.3-1.9L4.2 7 7 4.2l.1.1A1.7 1.7 0 0 0 9 4.6a1.7 1.7 0 0 0 1-1.6V3h4v.1a1.7 1.7 0 0 0 1 1.6 1.7 1.7 0 0 0 1.9-.3l.1-.1L19.8 7l-.1.1a1.7 1.7 0 0 0-.3 1.9 1.7 1.7 0 0 0 1.6 1h.2v4H21a1.7 1.7 0 0 0-1.6 1z"/></svg>',
  search: '<svg viewBox="0 0 24 24"><circle cx="10.8" cy="10.8" r="6.8"/><path d="m16 16 4 4"/></svg>',
  plus: '<svg viewBox="0 0 24 24"><path d="M12 5v14M5 12h14"/></svg>',
  play: '<svg viewBox="0 0 24 24"><path d="m9 6 9 6-9 6z"/></svg>',
  external: '<svg viewBox="0 0 24 24"><path d="M14 4h6v6M20 4l-9 9"/><path d="M18 13v5a2 2 0 0 1-2 2H6a2 2 0 0 1-2-2V8a2 2 0 0 1 2-2h5"/></svg>',
  export: '<svg viewBox="0 0 24 24"><path d="M12 3v12M7 8l5-5 5 5"/><path d="M5 14v5a2 2 0 0 0 2 2h10a2 2 0 0 0 2-2v-5"/></svg>',
  chevron: '<svg viewBox="0 0 24 24"><path d="m9 18 6-6-6-6"/></svg>',
  close: '<svg viewBox="0 0 24 24"><path d="m6 6 12 12M18 6 6 18"/></svg>',
  refresh: '<svg viewBox="0 0 24 24"><path d="M20 6v5h-5"/><path d="M4 18v-5h5"/><path d="M18.2 9A7 7 0 0 0 6.4 6.4L4 9M5.8 15A7 7 0 0 0 17.6 17.6L20 15"/></svg>',
  check: '<svg viewBox="0 0 24 24"><path d="m5 12 4 4L19 6"/></svg>',
  sort: '<svg viewBox="0 0 24 24"><path d="M8 6h12M8 12h8M8 18h4M4 5v14"/></svg>',
};

let snapshot: LibrarySnapshot = {
  recordings: [],
  settings: {
    mediaFolders: [],
    ffmpegPath: "",
    ffprobePath: "",
    whisperPath: "",
    modelPath: "",
    language: "auto",
    audioMode: "auto",
    microphoneTrack: 1,
  },
};
let selectedId = "";
let selected = new Set<string>();
let query = "";
let statusFilter: "all" | RecordingStatus = "all";
let demoMode = false;
let settingsOpen = false;
let busy = false;
let rescanning = false;
let lastSelectedId = "";
let sortBy: "newest" | "oldest" | "name" | "duration" = "newest";
let previewObserver: IntersectionObserver | null = null;
let previewLoading = 0;
const previewCache = new Map<string, string>();
const previewRequested = new Set<string>();
const previewQueue: Recording[] = [];

const isTauri = () => "__TAURI_INTERNALS__" in window;

function escapeHtml(value: string): string {
  return value.replace(/[&<>'"]/g, (char) => ({ "&": "&amp;", "<": "&lt;", ">": "&gt;", "'": "&#39;", '"': "&quot;" })[char]!);
}

function formatTime(ms: number | null): string {
  if (ms === null) return "—";
  const total = Math.max(0, Math.floor(ms / 1000));
  const h = Math.floor(total / 3600);
  const m = Math.floor((total % 3600) / 60);
  const s = total % 60;
  return h ? `${h}:${String(m).padStart(2, "0")}:${String(s).padStart(2, "0")}` : `${m}:${String(s).padStart(2, "0")}`;
}

function formatSize(bytes: number): string {
  if (!bytes) return "0 MB";
  const gb = bytes / 1024 ** 3;
  return gb >= 1 ? `${gb.toFixed(1)} GB` : `${(bytes / 1024 ** 2).toFixed(0)} MB`;
}

function relativeDate(timestamp: number): string {
  const days = Math.round((Date.now() - timestamp) / 86400000);
  if (days <= 0) return "Today";
  if (days === 1) return "Yesterday";
  if (days < 7) return `${days} days ago`;
  return new Date(timestamp).toLocaleDateString(undefined, { month: "short", day: "numeric" });
}

function filteredRecordings(): Recording[] {
  const needle = query.trim().toLowerCase();
  const recordings = snapshot.recordings.filter((recording) => {
    if (statusFilter !== "all" && recording.status !== statusFilter) return false;
    if (!needle) return true;
    const haystack = [
      recording.fileName,
      recording.displayTitle,
      recording.summary,
      ...recording.transcript.map((segment) => segment.text),
    ].join(" ").toLowerCase();
    return haystack.includes(needle);
  });
  return recordings.sort((left, right) => {
    if (sortBy === "oldest") return left.modifiedAt - right.modifiedAt;
    if (sortBy === "name") return left.displayTitle.localeCompare(right.displayTitle, undefined, { numeric: true });
    if (sortBy === "duration") return (right.durationMs ?? -1) - (left.durationMs ?? -1);
    return right.modifiedAt - left.modifiedAt;
  });
}

function previewKey(recording: Recording): string {
  return `${recording.id}:${recording.modifiedAt}`;
}

function statusLabel(recording: Recording): string {
  if (recording.status === "processing") return `Transcribing ${recording.progress}%`;
  return ({ new: "Not transcribed", queued: "Queued", ready: "Ready", error: "Needs attention" } as Record<RecordingStatus, string>)[recording.status];
}

function render(): void {
  const listScrollTop = document.querySelector<HTMLElement>(".recording-list")?.scrollTop;
  const detailScrollTop = document.querySelector<HTMLElement>(".detail-scroll, .empty-detail-body")?.scrollTop;
  const recordings = filteredRecordings();
  const current = recordings.find((recording) => recording.id === selectedId) ?? recordings[0];
  if (current && selectedId !== current.id) selectedId = current.id;
  const ready = snapshot.recordings.filter((item) => item.status === "ready").length;
  const pending = snapshot.recordings.filter((item) => item.status === "new" || item.status === "error").length;
  const errors = snapshot.recordings.filter((item) => item.status === "error").length;
  const selectedVisible = recordings.filter((item) => selected.has(item.id)).length;
  const allVisibleSelected = recordings.length > 0 && selectedVisible === recordings.length;

  app.innerHTML = `
    <div class="shell">
      <aside class="sidebar">
        <div class="brand"><span class="brand-mark">L</span><span>Leonardo</span><span class="local-pill">LOCAL</span></div>
        <nav>
          <button class="nav-item active">${icons.library}<span>Library</span><em>${snapshot.recordings.length}</em></button>
          <button class="nav-item" data-action="add-folder">${icons.folder}<span>Folders</span><em>${snapshot.settings.mediaFolders.length}</em></button>
          <button class="nav-item" data-action="show-ready">${icons.spark}<span>Highlights</span><em>${ready}</em></button>
        </nav>
        <div class="side-section">
          <div class="side-label">WORKSPACE</div>
          ${snapshot.settings.mediaFolders.length ? snapshot.settings.mediaFolders.map((folder) => `<button class="folder-item" title="${escapeHtml(folder)}">${icons.folder}<span>${escapeHtml(folder.split(/[\\/]/).pop() || folder)}</span></button>`).join("") : '<p class="side-empty">No capture folders yet.</p>'}
        </div>
        <div class="sidebar-bottom">
          <div class="gpu-card"><span class="gpu-dot"></span><div><strong>Local processing</strong><small>${pending ? `${pending} recording${pending === 1 ? "" : "s"} waiting` : "Library is up to date"}</small></div></div>
          <button class="nav-item" data-action="settings">${icons.settings}<span>Settings</span></button>
        </div>
      </aside>
      <main class="main">
        <header class="topbar">
          <div class="search-box">${icons.search}<input id="search" placeholder="Search anything you said…" value="${escapeHtml(query)}">${query ? `<button class="search-clear" data-action="clear-search" title="Clear search">${icons.close}</button>` : ""}<kbd>Ctrl K</kbd></div>
          <button class="button secondary rescan-button ${rescanning ? "loading" : ""}" data-action="rescan" ${busy || rescanning || !snapshot.settings.mediaFolders.length ? "disabled" : ""} title="Look for added, changed, or removed recordings">${icons.refresh} ${rescanning ? "Scanning…" : "Rescan"}</button>
          <button class="button secondary" data-action="add-folder">${icons.plus} Add folder</button>
          <button class="button primary" data-action="transcribe-selected" ${busy || (!selected.size && pending === 0) ? "disabled" : ""}>${icons.spark} ${selected.size ? `Transcribe ${selected.size}` : "Transcribe new"}</button>
        </header>
        <section class="content">
          <div class="library-pane">
            <div class="library-heading">
              <div><h1>Recordings</h1><p>${snapshot.recordings.length ? `${snapshot.recordings.length} files · ${formatSize(snapshot.recordings.reduce((sum, item) => sum + item.sizeBytes, 0))}` : "Your searchable video library"}</p></div>
              <div class="filter-tabs">
                <button class="${statusFilter === "all" ? "active" : ""}" data-filter="all">All</button>
                <button class="${statusFilter === "ready" ? "active" : ""}" data-filter="ready">Ready</button>
                <button class="${statusFilter === "new" ? "active" : ""}" data-filter="new">New</button>
                ${errors ? `<button class="${statusFilter === "error" ? "active danger" : ""}" data-filter="error">Errors <span>${errors}</span></button>` : ""}
              </div>
            </div>
            ${snapshot.recordings.length === 0 ? renderEmpty() : `
              <div class="library-controls ${selected.size ? "has-selection" : ""}">
                <div class="selection-summary">
                  ${selected.size ? `<span class="selection-count">${icons.check}<strong>${selected.size}</strong> selected</span><button class="text-button" data-action="select-visible">${allVisibleSelected ? "Deselect visible" : `Select all ${recordings.length} shown`}</button><button class="text-button muted-action" data-action="clear-selection">Clear</button>` : `<span class="selection-hint">Select recordings to transcribe them as a batch</span><button class="text-button" data-action="select-visible">Select all ${recordings.length} shown</button>`}
                </div>
                <label class="sort-control">${icons.sort}<select id="sort-recordings" aria-label="Sort recordings"><option value="newest" ${sortBy === "newest" ? "selected" : ""}>Newest first</option><option value="oldest" ${sortBy === "oldest" ? "selected" : ""}>Oldest first</option><option value="name" ${sortBy === "name" ? "selected" : ""}>Name</option><option value="duration" ${sortBy === "duration" ? "selected" : ""}>Longest first</option></select></label>
              </div>
              <div class="table-head"><label class="check select-all" title="Select all visible recordings"><input type="checkbox" data-select-all ${allVisibleSelected ? "checked" : ""}><span></span></label><span>Recording</span><span>Length</span><span>Added</span><span>Status</span></div>
              <div class="recording-list">
                ${recordings.map((recording) => renderRow(recording, recording.id === current?.id)).join("") || `<div class="no-results"><div>${icons.search}</div><strong>No recordings found</strong><span>Try a different search or clear the active filter.</span><button class="button secondary" data-action="clear-filters">Clear filters</button></div>`}
              </div>
            `}
          </div>
          ${current ? renderDetails(current) : ""}
        </section>
      </main>
      ${settingsOpen ? renderSettings() : ""}
      <div id="toast-root"></div>
    </div>`;
  bindEvents();
  observePreviews();
  restoreScrollPosition(".recording-list", listScrollTop);
  restoreScrollPosition(".detail-scroll, .empty-detail-body", detailScrollTop);
}

function restoreScrollPosition(selector: string, scrollTop: number | undefined): void {
  if (scrollTop === undefined) return;
  const element = document.querySelector<HTMLElement>(selector);
  if (element) element.scrollTop = scrollTop;
}

function renderEmpty(): string {
  return `<div class="empty-state">
    <div class="empty-orbit"><span>${icons.folder}</span></div>
    <h2>Turn your recordings into a searchable library</h2>
    <p>Add your OBS or capture folder. Leonardo finds videos, transcribes commentary locally, and builds timestamped summaries.</p>
    <div class="empty-actions"><button class="button primary" data-action="add-folder">${icons.plus} Choose capture folder</button><button class="button ghost" data-action="load-demo">Preview with sample data</button></div>
    <div class="privacy-note"><span class="gpu-dot"></span> Your media and transcripts stay on this computer</div>
  </div>`;
}

function renderPreview(recording: Recording, large = false): string {
  const key = previewKey(recording);
  const cached = previewCache.get(key);
  return `<div class="${large ? "detail-preview" : "thumb"} ${cached ? "loaded" : "is-placeholder"}" data-preview-key="${escapeHtml(key)}" data-preview-id="${escapeHtml(recording.id)}" data-demo-id="${demoMode ? escapeHtml(recording.id) : ""}">
    ${cached ? `<img src="${cached}" alt="Preview frame from ${escapeHtml(recording.displayTitle)}">` : `<span class="thumb-format">${recording.extension.toUpperCase()}</span><span class="preview-shimmer"></span>`}
    <button data-action="open" data-id="${escapeHtml(recording.id)}" title="Play recording">${icons.play}<span>${large ? "Play original" : ""}</span></button>
  </div>`;
}

function renderRow(recording: Recording, active: boolean): string {
  const checked = selected.has(recording.id);
  return `<article class="recording-row ${active ? "active" : ""}" data-id="${recording.id}">
    <label class="check" title="Select"><input type="checkbox" data-select="${recording.id}" ${checked ? "checked" : ""}><span></span></label>
    <div class="recording-main">${renderPreview(recording)}<div class="recording-copy"><strong>${escapeHtml(recording.displayTitle)}</strong><small>${escapeHtml(recording.fileName)} · ${formatSize(recording.sizeBytes)}</small></div></div>
    <span class="cell muted">${formatTime(recording.durationMs)}</span>
    <span class="cell muted">${relativeDate(recording.modifiedAt)}</span>
    <span class="status ${recording.status}"><i></i>${statusLabel(recording)}</span>
  </article>`;
}

function renderDetails(recording: Recording): string {
  if (recording.status !== "ready") {
    return `<aside class="details empty-detail"><div class="details-top"><span>Recording details</span><button class="icon-button" data-action="open" data-id="${recording.id}" title="Open original">${icons.external}</button></div><div class="empty-detail-body">${renderPreview(recording, true)}<div class="waiting-art">${icons.spark}</div><h2>${recording.status === "processing" ? "Listening to your recording" : "Ready to transcribe"}</h2><p>${recording.status === "error" ? escapeHtml(recording.error || "The last transcription failed.") : "Generate a timestamped transcript and an edit-friendly overview."}</p>${recording.status === "processing" ? `<div class="progress"><span style="width:${recording.progress}%"></span></div>` : `<button class="button primary" data-action="transcribe-one" data-id="${recording.id}" ${busy ? "disabled" : ""}>${icons.spark} Transcribe recording</button>`}</div></aside>`;
  }
  return `<aside class="details">
    <div class="details-top"><span>Recording details</span><button class="icon-button" data-action="open" data-id="${recording.id}" title="Open original">${icons.external}</button></div>
    <div class="detail-scroll">
      ${renderPreview(recording, true)}
      <div class="title-block"><span class="eyebrow">AI TITLE</span><h2>${escapeHtml(recording.displayTitle)}</h2><p>${escapeHtml(recording.fileName)}${recording.audioSource ? ` · ${escapeHtml(recording.audioSource)}` : ""}</p><div class="metadata-chips"><span>${formatTime(recording.durationMs)}</span><span>${formatSize(recording.sizeBytes)}</span>${recording.audioTracks.length ? `<span>${recording.audioTracks.length} audio track${recording.audioTracks.length === 1 ? "" : "s"}</span>` : ""}</div></div>
      <section class="summary-card"><div class="section-title">${icons.spark}<span>Quick take</span></div><p>${escapeHtml(recording.summary)}</p></section>
      <section class="chapters"><div class="section-header"><span>Key moments</span><em>${recording.chapters.length}</em></div>
        ${recording.chapters.map((chapter, index) => `<button class="chapter" data-action="seek" data-id="${recording.id}" data-ms="${chapter.startMs}"><span class="chapter-time">${formatTime(chapter.startMs)}</span><span class="chapter-line"></span><span class="chapter-index">${index + 1}</span><span class="chapter-copy"><strong>${escapeHtml(chapter.title)}</strong><small>${escapeHtml(chapter.description)}</small></span>${icons.chevron}</button>`).join("")}
      </section>
      <section class="transcript"><div class="section-header"><span>Transcript</span><em>${recording.language?.toUpperCase() || "AUTO"}</em></div>
        ${recording.transcript.map((segment) => `<button class="transcript-line" data-action="seek" data-id="${recording.id}" data-ms="${segment.startMs}"><time>${formatTime(segment.startMs)}</time><span>${highlightQuery(segment.text)}</span></button>`).join("")}
      </section>
    </div>
    <div class="detail-actions"><button class="button secondary" data-action="export-srt" data-id="${recording.id}">${icons.export} Export SRT</button><button class="button primary compact" data-action="export-resolve" data-id="${recording.id}">Resolve markers ${icons.chevron}</button></div>
  </aside>`;
}

function highlightQuery(text: string): string {
  const safe = escapeHtml(text);
  if (!query.trim()) return safe;
  const escapedNeedle = query.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");
  return safe.replace(new RegExp(`(${escapedNeedle})`, "ig"), "<mark>$1</mark>");
}

function renderSettings(): string {
  const settings = snapshot.settings;
  return `<div class="modal-backdrop" data-action="close-settings"><section class="modal" role="dialog" aria-modal="true" onclick="event.stopPropagation()">
    <div class="modal-header"><div><span class="eyebrow">LOCAL AI</span><h2>Transcription engine</h2></div><button class="icon-button" data-action="close-settings">${icons.close}</button></div>
    <p class="modal-intro">Release builds bundle these tools. Paths are exposed here so developer builds can use local binaries without Python.</p>
    <form id="settings-form">
      <label>Whisper executable<input name="whisperPath" value="${escapeHtml(settings.whisperPath)}" placeholder="whisper-cli.exe"></label>
      <label>Whisper model<input name="modelPath" value="${escapeHtml(settings.modelPath)}" placeholder="ggml-large-v3-turbo.bin"></label>
      <div class="form-grid"><label>FFmpeg<input name="ffmpegPath" value="${escapeHtml(settings.ffmpegPath)}" placeholder="ffmpeg.exe"></label><label>FFprobe<input name="ffprobePath" value="${escapeHtml(settings.ffprobePath)}" placeholder="ffprobe.exe"></label></div>
      <label>Spoken language<select name="language"><option value="auto" ${settings.language === "auto" ? "selected" : ""}>Detect automatically</option><option value="en" ${settings.language === "en" ? "selected" : ""}>English</option><option value="de" ${settings.language === "de" ? "selected" : ""}>German</option><option value="ru" ${settings.language === "ru" ? "selected" : ""}>Russian</option></select></label>
      <div class="audio-settings">
        <div class="section-title">${icons.library}<span>Multi-track MKV audio</span></div>
        <div class="form-grid"><label>Audio source<select name="audioMode"><option value="auto" ${settings.audioMode === "auto" ? "selected" : ""}>Detect microphone</option><option value="track" ${settings.audioMode === "track" ? "selected" : ""}>Always use track number</option><option value="mix" ${settings.audioMode === "mix" ? "selected" : ""}>Mix every audio track</option></select></label><label>Microphone track number<input name="microphoneTrack" type="number" min="1" step="1" value="${settings.microphoneTrack}"></label></div>
        <p>Automatic mode prefers tracks named Mic, Microphone, Voice, Commentary, or Headset, then a unique mono track. Track numbers are counted among audio tracks starting at 1. Mix mode is best for clean stems; some OBS layouts include a combined Track 1 that would duplicate audio if mixed again.</p>
      </div>
      <div class="engine-note"><span class="gpu-dot"></span><div><strong>RTX acceleration</strong><small>Use a CUDA build of whisper.cpp. Leonardo detects it from the engine output.</small></div></div>
      <button class="button primary full" type="submit">Save settings</button>
    </form>
  </section></div>`;
}

function bindEvents(): void {
  document.querySelector<HTMLInputElement>("#search")?.addEventListener("input", (event) => {
    query = (event.target as HTMLInputElement).value;
    render();
    const input = document.querySelector<HTMLInputElement>("#search");
    input?.focus();
    input?.setSelectionRange(query.length, query.length);
  });
  document.querySelectorAll<HTMLElement>("[data-filter]").forEach((element) => element.addEventListener("click", () => {
    statusFilter = element.dataset.filter as typeof statusFilter;
    render();
  }));
  document.querySelectorAll<HTMLElement>(".recording-row").forEach((row) => row.addEventListener("click", (event) => {
    if ((event.target as HTMLElement).closest("input,button,label")) return;
    selectedId = row.dataset.id || "";
    render();
  }));
  document.querySelectorAll<HTMLInputElement>("[data-select]").forEach((checkbox) => checkbox.addEventListener("click", (event) => {
    const id = checkbox.dataset.select!;
    if ((event as MouseEvent).shiftKey && lastSelectedId) {
      const ids = filteredRecordings().map((recording) => recording.id);
      const from = ids.indexOf(lastSelectedId);
      const to = ids.indexOf(id);
      if (from >= 0 && to >= 0) {
        ids.slice(Math.min(from, to), Math.max(from, to) + 1).forEach((rangeId) => checkbox.checked ? selected.add(rangeId) : selected.delete(rangeId));
      }
    } else {
      checkbox.checked ? selected.add(id) : selected.delete(id);
    }
    lastSelectedId = id;
    render();
  }));
  document.querySelector<HTMLInputElement>("[data-select-all]")?.addEventListener("change", (event) => {
    const checked = (event.target as HTMLInputElement).checked;
    filteredRecordings().forEach((recording) => checked ? selected.add(recording.id) : selected.delete(recording.id));
    render();
  });
  const selectAll = document.querySelector<HTMLInputElement>("[data-select-all]");
  if (selectAll) {
    const visible = filteredRecordings();
    const selectedVisible = visible.filter((recording) => selected.has(recording.id)).length;
    selectAll.indeterminate = selectedVisible > 0 && selectedVisible < visible.length;
  }
  document.querySelector<HTMLSelectElement>("#sort-recordings")?.addEventListener("change", (event) => {
    sortBy = (event.target as HTMLSelectElement).value as typeof sortBy;
    render();
  });
  document.querySelectorAll<HTMLElement>("[data-action]").forEach((element) => element.addEventListener("click", () => handleAction(element)));
  document.querySelector<HTMLFormElement>("#settings-form")?.addEventListener("submit", saveSettings);
}

async function handleAction(element: HTMLElement): Promise<void> {
  const action = element.dataset.action;
  if (action === "add-folder") await addFolder();
  if (action === "load-demo") { snapshot = structuredClone(demoSnapshot); selectedId = snapshot.recordings[0].id; demoMode = true; render(); }
  if (action === "show-ready") { statusFilter = "ready"; render(); }
  if (action === "settings") { settingsOpen = true; render(); }
  if (action === "close-settings") { settingsOpen = false; render(); }
  if (action === "clear-search") { query = ""; render(); }
  if (action === "clear-filters") { query = ""; statusFilter = "all"; render(); }
  if (action === "clear-selection") { selected.clear(); lastSelectedId = ""; render(); }
  if (action === "select-visible") {
    const visible = filteredRecordings();
    const allSelected = visible.length > 0 && visible.every((recording) => selected.has(recording.id));
    visible.forEach((recording) => allSelected ? selected.delete(recording.id) : selected.add(recording.id));
    render();
  }
  if (action === "rescan") await rescan();
  if (action === "transcribe-one" && element.dataset.id) await transcribe([element.dataset.id]);
  if (action === "transcribe-selected") {
    const ids = selected.size ? [...selected] : snapshot.recordings.filter((item) => item.status === "new" || item.status === "error").map((item) => item.id);
    await transcribe(ids);
  }
  if (action === "open" && element.dataset.id) await backend("open_recording", { id: element.dataset.id });
  if (action === "seek" && element.dataset.id) await backend("open_recording", { id: element.dataset.id, seekMs: Number(element.dataset.ms || 0) });
  if (action === "export-srt" && element.dataset.id) await exportRecording("export_srt", element.dataset.id, "Subtitle file exported");
  if (action === "export-resolve" && element.dataset.id) await exportRecording("export_resolve_markers", element.dataset.id, "Resolve marker file exported");
}

async function addFolder(): Promise<void> {
  if (demoMode) { snapshot = { recordings: [], settings: snapshot.settings }; demoMode = false; }
  try {
    const next = await backend<LibrarySnapshot>("choose_and_scan_folder");
    if (next) {
      snapshot = next;
      selected.clear();
      selectedId = snapshot.recordings[0]?.id || "";
      render();
    }
  } catch (error) { toast(String(error), true); }
}

async function rescan(): Promise<void> {
  if (rescanning || busy) return;
  if (demoMode) { toast("Add your own capture folder before rescanning."); return; }
  if (!snapshot.settings.mediaFolders.length) { await addFolder(); return; }
  const before = new Set(snapshot.recordings.map((recording) => recording.id));
  rescanning = true;
  render();
  try {
    const next = await backend<LibrarySnapshot>("rescan_media_folders");
    const after = new Set(next.recordings.map((recording) => recording.id));
    const added = next.recordings.filter((recording) => !before.has(recording.id)).length;
    const removed = snapshot.recordings.filter((recording) => !after.has(recording.id)).length;
    snapshot = next;
    selected = new Set([...selected].filter((id) => after.has(id)));
    if (!after.has(selectedId)) selectedId = snapshot.recordings[0]?.id || "";
    toast(added || removed ? `Library updated · ${added} added · ${removed} removed` : "Library is already up to date");
  } catch (error) {
    toast(String(error), true);
  } finally {
    rescanning = false;
    render();
  }
}

function observePreviews(): void {
  previewObserver?.disconnect();
  previewObserver = null;
  if (demoMode || !isTauri()) return;
  previewObserver = new IntersectionObserver((entries) => {
    entries.forEach((entry) => {
      if (!entry.isIntersecting) return;
      const element = entry.target as HTMLElement;
      const id = element.dataset.previewId;
      const key = element.dataset.previewKey;
      if (!id || !key) return;
      const recording = snapshot.recordings.find((item) => item.id === id && previewKey(item) === key);
      if (recording) queuePreview(recording);
      previewObserver?.unobserve(element);
    });
  }, { rootMargin: "80px" });
  document.querySelectorAll<HTMLElement>("[data-preview-key]").forEach((element) => {
    const key = element.dataset.previewKey!;
    const cached = previewCache.get(key);
    if (cached) applyPreview(key, cached);
    else previewObserver?.observe(element);
  });
}

function queuePreview(recording: Recording): void {
  const key = previewKey(recording);
  if (previewCache.has(key) || previewRequested.has(key)) return;
  previewRequested.add(key);
  previewQueue.push(recording);
  pumpPreviewQueue();
}

function pumpPreviewQueue(): void {
  while (previewLoading < 1 && previewQueue.length) {
    const recording = previewQueue.shift()!;
    const key = previewKey(recording);
    if (!previewIsNearViewport(key)) {
      previewRequested.delete(key);
      continue;
    }
    previewLoading += 1;
    backend<string>("recording_thumbnail", { id: recording.id })
      .then((url) => {
        previewCache.set(key, url);
        applyPreview(key, url);
      })
      .catch(() => {
        document.querySelectorAll<HTMLElement>(`[data-preview-key="${CSS.escape(key)}"]`).forEach((element) => element.classList.add("preview-failed"));
      })
      .finally(() => {
        previewLoading -= 1;
        pumpPreviewQueue();
      });
  }
}

function previewIsNearViewport(key: string): boolean {
  return [...document.querySelectorAll<HTMLElement>(`[data-preview-key="${CSS.escape(key)}"]`)].some((element) => {
    const bounds = element.getBoundingClientRect();
    return bounds.bottom >= -120 && bounds.top <= window.innerHeight + 120;
  });
}

function applyPreview(key: string, url: string): void {
  document.querySelectorAll<HTMLElement>(`[data-preview-key="${CSS.escape(key)}"]`).forEach((element) => {
    if (element.querySelector("img")) return;
    const image = document.createElement("img");
    image.src = url;
    image.alt = "Video preview";
    element.querySelector(".thumb-format")?.remove();
    element.querySelector(".preview-shimmer")?.remove();
    element.insertBefore(image, element.querySelector("button"));
    element.classList.remove("is-placeholder", "preview-failed");
    element.classList.add("loaded");
  });
}

async function transcribe(ids: string[]): Promise<void> {
  if (!ids.length || busy) return;
  if (demoMode) { toast("Sample data is read-only. Add your capture folder to transcribe real files."); return; }
  busy = true;
  for (const id of ids) {
    const item = snapshot.recordings.find((recording) => recording.id === id);
    if (!item) continue;
    item.status = "processing";
    item.progress = 8;
    render();
    try {
      const updated = await backend<Recording>("transcribe_recording", { id });
      Object.assign(item, updated);
    } catch (error) {
      item.status = "error";
      item.error = String(error);
      toast(`Could not transcribe ${item.fileName}`, true);
    }
    render();
  }
  selected.clear();
  busy = false;
  render();
}

async function exportRecording(command: string, id: string, message: string): Promise<void> {
  if (demoMode) { toast("Add your own folder before exporting."); return; }
  try {
    const path = await backend<string>(command, { id });
    if (path) toast(`${message}: ${path}`);
  } catch (error) { toast(String(error), true); }
}

async function saveSettings(event: SubmitEvent): Promise<void> {
  event.preventDefault();
  const data = new FormData(event.currentTarget as HTMLFormElement);
  const settings: AppSettings = { ...snapshot.settings,
    whisperPath: String(data.get("whisperPath") || ""), modelPath: String(data.get("modelPath") || ""),
    ffmpegPath: String(data.get("ffmpegPath") || ""), ffprobePath: String(data.get("ffprobePath") || ""), language: String(data.get("language") || "auto"),
    audioMode: String(data.get("audioMode") || "auto") as AppSettings["audioMode"],
    microphoneTrack: Math.max(1, Number(data.get("microphoneTrack") || 1)),
  };
  try {
    snapshot.settings = demoMode ? settings : await backend<AppSettings>("save_settings", { settings });
    settingsOpen = false;
    render();
    toast("Engine settings saved");
  } catch (error) { toast(String(error), true); }
}

async function backend<T>(command: string, args?: Record<string, unknown>): Promise<T> {
  if (!isTauri()) throw new Error("Desktop commands are only available in the Tauri app.");
  return invoke<T>(command, args);
}

function toast(message: string, isError = false): void {
  const root = document.querySelector<HTMLDivElement>("#toast-root");
  if (!root) return;
  const node = document.createElement("div");
  node.className = `toast ${isError ? "error" : ""}`;
  node.textContent = message;
  root.appendChild(node);
  window.setTimeout(() => node.remove(), 4200);
}

document.addEventListener("keydown", (event) => {
  const target = event.target as HTMLElement;
  const isTyping = target.matches("input, textarea, select") || target.isContentEditable;
  if ((event.ctrlKey || event.metaKey) && event.key.toLowerCase() === "k") {
    event.preventDefault();
    document.querySelector<HTMLInputElement>("#search")?.focus();
  }
  if ((event.ctrlKey || event.metaKey) && event.key.toLowerCase() === "a" && !isTyping && !settingsOpen) {
    event.preventDefault();
    filteredRecordings().forEach((recording) => selected.add(recording.id));
    render();
  }
  if (event.key === "F5" && !settingsOpen) {
    event.preventDefault();
    void rescan();
  }
  if (event.key === "Escape" && settingsOpen) { settingsOpen = false; render(); }
  else if (event.key === "Escape" && selected.size) { selected.clear(); lastSelectedId = ""; render(); }
});

async function start(): Promise<void> {
  if (isTauri()) {
    try { snapshot = await backend<LibrarySnapshot>("load_library"); }
    catch (error) { console.error(error); }
  }
  selectedId = snapshot.recordings[0]?.id || "";
  render();
}

void start();
