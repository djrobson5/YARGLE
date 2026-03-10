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
}

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
}

export interface SongValidationResult {
  path: string;
  display_name: string;
  issues: ValidationIssue[];
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
