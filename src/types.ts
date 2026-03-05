export interface SongSummary {
  path: string;
  display_name: string;
  description: string;
  title_name: string;
  has_thumbnail: boolean;
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

export interface SongDetails {
  path: string;
  display_name: string;
  description: string;
  title_name: string;
  thumbnail_base64: string;
  metadata: SongMetadata;
  raw_dta: string;
  dta_file_size: number;
}
