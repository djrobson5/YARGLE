import React, { useMemo } from "react";
import { FixedSizeList } from "react-window";
import type { SongSummary } from "../types";
import sourcesData from "../data/sources.json";

interface SourceEntry {
  ids: string[];
  names: { "en-US": string };
  icon: string;
  type: string;
}

const sourceLookup = new Map<string, { icon: string; name: string }>();
for (const s of (sourcesData.sources as SourceEntry[])) {
  for (const id of s.ids) {
    sourceLookup.set(id, { icon: s.icon, name: s.names["en-US"] });
  }
}

function getOriginIcon(gameOrigin: string): { icon: string; name: string } | null {
  if (!gameOrigin) return null;
  return sourceLookup.get(gameOrigin) || { icon: "custom", name: gameOrigin };
}

interface FileListProps {
  songs: SongSummary[];
  selectedPath: string | null;
  filter: string;
  gameOriginFilter: string | null;
  onSelect: (path: string) => void;
  modifiedPaths: Set<string>;
}

export function FileList({ songs, selectedPath, filter, gameOriginFilter, onSelect, modifiedPaths }: FileListProps) {
  const filtered = useMemo(() => {
    let result = songs;

    if (gameOriginFilter) {
      result = result.filter((s) => {
        const origin = s.game_origin || "";
        // Normalize: treat empty/ugc_plus as c3customs (matches useSongFiles behavior)
        const normalized = (!origin || origin === "ugc_plus") ? "c3customs" : origin;
        return normalized === gameOriginFilter;
      });
    }

    if (filter) {
      const lower = filter.toLowerCase();
      result = result.filter(
        (s) =>
          s.display_name.toLowerCase().includes(lower) ||
          s.description.toLowerCase().includes(lower) ||
          s.album_name.toLowerCase().includes(lower) ||
          s.author.toLowerCase().includes(lower)
      );
    }

    return result;
  }, [songs, filter, gameOriginFilter]);

  if (songs.length === 0) {
    return (
      <div className="file-list-empty">
        <p>No songs loaded</p>
        <p className="hint">Click "Open Folder" to browse CON files and song folders</p>
      </div>
    );
  }

  const Row = ({ index, style }: { index: number; style: React.CSSProperties }) => {
    const song = filtered[index];
    const isSelected = song.path === selectedPath;
    const isModified = modifiedPaths.has(song.path);
    const originInfo = getOriginIcon(song.game_origin);

    return (
      <div
        style={style}
        className={`file-list-item ${isSelected ? "selected" : ""} ${isModified ? "modified" : ""}`}
        onClick={() => onSelect(song.path)}
      >
        {originInfo && (
          <img
            className="file-list-origin-icon"
            src={`/icons/${originInfo.icon}.png`}
            alt={originInfo.name}
            title={originInfo.name}
          />
        )}
        <div className="file-list-item-text">
          <div className="song-name">
            {isModified && <span className="modified-dot" title="Unsaved changes" />}
            {song.is_folder && <span className="folder-badge" title="Unpacked song folder">F</span>}
            {song.display_name || "(unnamed)"}
          </div>
          <div className="song-artist">{song.description}</div>
        </div>
      </div>
    );
  };

  return (
    <FixedSizeList
      height={window.innerHeight - 60}
      width="100%"
      itemCount={filtered.length}
      itemSize={56}
      className="file-list"
    >
      {Row}
    </FixedSizeList>
  );
}
