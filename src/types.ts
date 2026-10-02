export interface SongSummary {
  path: string;
  display_name: string;
  description: string;
  title_name: string;
  has_thumbnail: boolean;
  is_folder: boolean;
  album_name: string;
  author: string;
  game_origin: string;
  // File modified time (Unix seconds); drives the "Recently added" sort.
  added_at: number;
  // Artist and title from song.ini / songs.dta (empty if unreadable).
  artist: string;
  song_title: string;
}

// Library list sort order: "name" = alphabetical (default), "recent" = newest
// file first (by added_at).
export type SortMode = "name" | "recent";

export interface SongMetadata {
  shortname: string;
  name: string;
  artist: string;
  album_name: string;
  album_track_number: number | null;
  genre: string;
  sub_genre: string;
  vocal_gender: string;
  year_released: number | null;
  song_length: number | null;
  rating: number | null;
  song_id: number | null;
  game_origin: string;
  preview_start: number | null;
  preview_end: number | null;
  rank_drum: number | null;
  rank_guitar: number | null;
  rank_bass: number | null;
  rank_vocals: number | null;
  rank_keys: number | null;
  rank_band: number | null;
  rank_real_guitar: number | null;
  rank_real_bass: number | null;
  rank_real_keys: number | null;
  author: string;
}

export interface ValidationIssue {
  level: "Error" | "Warning" | "Info";
  field: string;
  message: string;
}

export interface SongDetails {
  path: string;
  display_name: string;
  description: string;
  title_name: string;
  thumbnail_base64: string;
  metadata: SongMetadata;
  raw_dta: string;
  dta_file_size: number;
  validation_issues: ValidationIssue[];
  // True for unpacked song folders (song.ini native 0-6 tiers), false for
  // CON/STFS packages (Rock Band rank scale). Drives difficulty interpretation.
  is_folder: boolean;
}

export interface SongValidationResult {
  path: string;
  display_name: string;
  // Used by "Fix it" to pre-search the chart browser.
  artist: string;
  title: string;
  issues: ValidationIssue[];
  // Artist / title guessed from the file name when the metadata lacks
  // either; empty when there's nothing to suggest.
  suggested_artist: string;
  suggested_title: string;
}

// Issues with this field mean the song can't play (missing chart or audio);
// the UI offers "Fix it" (re-download and replace) for them.
export const BROKEN_FIELD = "broken";

// A broken song being replaced through the chart browser.
export interface FixTarget {
  path: string;
  name: string;
  artist: string;
  title: string;
}

export interface BatchValidateResult {
  total_songs: number;
  songs_with_errors: number;
  songs_with_warnings: number;
  songs_clean: number;
  parse_failures: number;
  results: SongValidationResult[];
}

export interface ChartOverview {
  duration_ms: number;
  total_measures: number;
  ticks_per_quarter: number;
  instruments: InstrumentSummary[];
}

export interface InstrumentSummary {
  name: string;
  track_name: string;
  note_counts: DifficultyNoteCounts;
  density: number[];
}

export interface DifficultyNoteCounts {
  easy: number;
  medium: number;
  hard: number;
  expert: number;
}

export interface InstrumentNotes {
  instrument: string;
  difficulty: string;
  ticks_per_quarter: number;
  tempo_changes: TempoEvent[];
  time_signatures: TimeSigEvent[];
  notes: ChartNote[];
  overdrive_phrases: OverdrivePhrase[];
  duration_ticks: number;
}

export interface OverdrivePhrase {
  start_tick: number;
  end_tick: number;
}

export interface ChartNote {
  tick: number;
  duration: number;
  lane: number;
  is_hopo: boolean;
}

export interface TempoEvent {
  tick: number;
  bpm: number;
}

export interface TimeSigEvent {
  tick: number;
  numerator: number;
  denominator: number;
}

// --- RhythmVerse browser (mirrors src-tauri/src/rhythmverse.rs) ---

export interface RvSongFile {
  file_id: string;
  song_id: number | null;
  title: string;
  artist: string;
  album: string;
  genre: string;
  subgenre: string;
  year: number | null;
  decade: string;
  song_length_sec: number | null;
  album_art_url: string;
  charter: string;
  gameformat: string;
  gamesource: string;
  size_bytes: number | null;
  downloads: number | null;
  uploader: string;
  uploaded: string;
  file_name: string;
  detail_url: string;
  download_url: string;
  // Non-empty when hosted off-site (Google Drive, Mediafire, …).
  external_url: string;
  // True when rv_download can fetch the off-site link itself (Drive file or
  // folder, Mediafire, Dropbox, shorteners); false = "Open ↗" only.
  external_auto: boolean;
  // Per-instrument difficulty tier; >=1 = charted, 0/-1/null = not present.
  diff_guitar: number | null;
  diff_bass: number | null;
  diff_drums: number | null;
  diff_vocals: number | null;
  diff_keys: number | null;
}

// Result of `delete_files`. Deletes go to the Recycle Bin; paths the bin refused
// (some USB / network drives) are listed in not_trashable, untouched, so the UI
// can ask before retrying them with `permanent: true`.
export interface DeleteResult {
  failures: string[];
  not_trashable: string[];
}

export interface RvBrowseResult {
  songs: RvSongFile[];
  total_available: number;
  total_filtered: number;
  returned: number;
  page: number;
}

// Offline RhythmVerse catalog (catalog.rs). `ready` = a full build has
// finished at least once, so browsing can use the local copy.
export interface CatalogStatus {
  ready: boolean;
  syncing: boolean;
  mode: "" | "full" | "delta";
  pages_done: number;
  pages_total: number;
  rows: number;
  last_sync: string;
  error: string | null;
}

export interface CatalogQuery {
  text?: string;
  charter?: string;
  genre?: string;
  gameformat?: string;
  instruments?: string[];
  yearMin?: number | null;
  yearMax?: number | null;
  lengthMin?: number | null;
  lengthMax?: number | null;
  addedWithinDays?: number | null;
  hideOwned?: boolean;
  sortBy?: string;
  sortOrder?: "ASC" | "DESC";
  page?: number;
  records?: number;
  random?: boolean;
}

export interface FacetCount {
  value: string;
  count: number;
}

export interface CatalogFacets {
  genres: FacetCount[];
  formats: FacetCount[];
}

export interface CatalogSuggestion {
  kind: "artist" | "title";
  value: string;
  count: number;
}

export interface RvDownloadResult {
  file_id: string;
  extracted_to: string;
  entries: number;
}

export interface RvDownloadRecord {
  file_id: string;
  downloaded_at: string;
  // RhythmVerse's upload_date for the version held locally (the update baseline).
  // Empty when unknown (pre-tracking records / editor links) — treated as
  // "don't flag updates" and backfilled on the next browse.
  rv_upload_date: string;
  // A "Got it" mark with no on-disk path; only these can be undone.
  manual: boolean;
}

export interface UpdateInfo {
  version: string;
  url: string;
  notes: string;
  // Direct download URL for the release's yargle.exe (null if the release has
  // no exe asset — then only "View release" is available).
  download_url: string | null;
}
