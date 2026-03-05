import React, { useMemo } from "react";
import { FixedSizeList } from "react-window";
import type { SongSummary } from "../types";

interface FileListProps {
  songs: SongSummary[];
  selectedPath: string | null;
  filter: string;
  onSelect: (path: string) => void;
  modifiedPaths: Set<string>;
}

export function FileList({ songs, selectedPath, filter, onSelect, modifiedPaths }: FileListProps) {
  const filtered = useMemo(() => {
    if (!filter) return songs;
    const lower = filter.toLowerCase();
    return songs.filter(
      (s) =>
        s.display_name.toLowerCase().includes(lower) ||
        s.description.toLowerCase().includes(lower)
    );
  }, [songs, filter]);

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

    return (
      <div
        style={style}
        className={`file-list-item ${isSelected ? "selected" : ""} ${isModified ? "modified" : ""}`}
        onClick={() => onSelect(song.path)}
      >
        <div className="song-name">
          {isModified && <span className="modified-dot" title="Unsaved changes" />}
          {song.is_folder && <span className="folder-badge" title="Unpacked song folder">F</span>}
          {song.display_name || "(unnamed)"}
        </div>
        <div className="song-artist">{song.description}</div>
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
