import React, { useMemo } from "react";
import type { SongSummary, SortMode } from "../types";
import sourcesData from "../data/sources.json";
import {
  Copy,
  FolderOpen,
  FolderTree,
  Globe,
  LockOpen,
  PenLine,
  Search,
  Settings,
  ShieldCheck,
  TextCursorInput,
} from "lucide-react";

interface SourceEntry {
  ids: string[];
  names: { "en-US": string };
  icon: string;
  type: string;
}

const sourceLookup = new Map<string, { icon: string; name: string; primaryId: string }>();
for (const s of (sourcesData.sources as SourceEntry[])) {
  const info = { icon: s.icon, name: s.names["en-US"], primaryId: s.ids[0] };
  for (const id of s.ids) {
    sourceLookup.set(id, info);
  }
}

interface SearchBarProps {
  value: string;
  onChange: (value: string) => void;
  onOpenFolder: () => void;
  onOpenOptions?: () => void;
  onDecryptMoggs?: () => void;
  onFindDuplicates?: () => void;
  onBatchRename?: () => void;
  onBatchEdit?: () => void;
  onOrganize?: () => void;
  onValidate?: () => void;
  onBrowseRhythmVerse?: () => void;
  songCount: number;
  songs: SongSummary[];
  sortBy: SortMode;
  onSortChange: (mode: SortMode) => void;
  gameOriginFilter: string | null;
  onGameOriginFilter: (origin: string | null) => void;
  multiSelectedCount: number;
  onClearMultiSelect: () => void;
  onSelectAllVisible: () => void;
  filteredCount: number;
}

export function SearchBar({ value, onChange, onOpenFolder, onOpenOptions, onDecryptMoggs, onFindDuplicates, onBatchRename, onBatchEdit, onOrganize, onValidate, onBrowseRhythmVerse, songCount, songs, sortBy, onSortChange, gameOriginFilter, onGameOriginFilter, multiSelectedCount, onClearMultiSelect, onSelectAllVisible, filteredCount }: SearchBarProps) {
  const hasTools = songCount > 0;

  // Build list of unique game origins present in the loaded songs, sorted by count descending
  const originButtons = useMemo(() => {
    if (songs.length === 0) return [];
    const counts = new Map<string, number>();
    for (const s of songs) {
      const origin = s.game_origin || "";
      const normalized = (!origin || origin === "ugc_plus") ? "c3customs" : origin;
      counts.set(normalized, (counts.get(normalized) || 0) + 1);
    }
    // Convert to array with icon info, sorted by count descending
    const entries: { id: string; icon: string; name: string; count: number }[] = [];
    for (const [id, count] of counts) {
      const info = sourceLookup.get(id);
      entries.push({
        id,
        icon: info ? info.icon : "custom",
        name: info ? info.name : id,
        count,
      });
    }
    entries.sort((a, b) => b.count - a.count);
    return entries;
  }, [songs]);

  return (
    <div className="search-bar-wrap">
      <div className="search-bar">
        <button
          className="open-folder-btn icon-only"
          onClick={onOpenFolder}
          title="Open Folder"
          aria-label="Open Folder"
        >
          <FolderOpen size={17} />
        </button>
        {onBrowseRhythmVerse && (
          <button
            className="open-folder-btn rv-open-btn icon-only"
            onClick={onBrowseRhythmVerse}
            title="Browse and download songs from RhythmVerse"
            aria-label="Browse RhythmVerse"
          >
            <Globe size={17} />
          </button>
        )}
        <label className="search-field">
          <Search size={15} />
          <input
            type="text"
            placeholder={
              songCount > 0
                ? `Filter ${songCount.toLocaleString()} song${songCount !== 1 ? "s" : ""}`
                : "Filter songs"
            }
            value={value}
            onChange={(e) => onChange(e.target.value)}
            className="search-input"
            aria-label="Filter songs"
          />
        </label>
        {songCount > 0 && (
          <select
            className="sort-select"
            value={sortBy}
            onChange={(e) => onSortChange(e.target.value as SortMode)}
            title="Sort the song list"
          >
            <option value="name">Name (A–Z)</option>
            <option value="recent">Recently added</option>
          </select>
        )}
      </div>
      {hasTools && (
        <div className="multi-select-bar">
          {multiSelectedCount > 0 ? (
            <span>{multiSelectedCount} song{multiSelectedCount !== 1 ? "s" : ""} selected</span>
          ) : (
            <span className="multi-select-hint">Use checkboxes to select songs</span>
          )}
          <div className="multi-select-actions">
            {multiSelectedCount < filteredCount && (
              <button className="select-all-btn" onClick={onSelectAllVisible}>
                Select All{filteredCount < songCount ? ` (${filteredCount})` : ""}
              </button>
            )}
            {multiSelectedCount > 0 && (
              <button className="clear-selection-btn" onClick={onClearMultiSelect}>Clear</button>
            )}
          </div>
        </div>
      )}
      {hasTools && (
        <div className="toolbar-row">
          {onBatchRename && (
            <button className="toolbar-btn" onClick={onBatchRename} title="Batch Rename Files">
              <TextCursorInput size={14} />
              Rename
            </button>
          )}
          {onBatchEdit && (
            <button className="toolbar-btn" onClick={onBatchEdit} title="Batch Edit Metadata">
              <PenLine size={14} />
              Batch Edit
            </button>
          )}
          {onOrganize && (
            <button className="toolbar-btn" onClick={onOrganize} title="Auto-Organize into Artist/Album folders">
              <FolderTree size={14} />
              Organize
            </button>
          )}
          {onValidate && (
            <button className="toolbar-btn" onClick={onValidate} title="Validate Song Metadata">
              <ShieldCheck size={14} />
              Validate
            </button>
          )}
          {onFindDuplicates && (
            <button className="toolbar-btn" onClick={onFindDuplicates} title="Find Duplicates">
              <Copy size={14} />
              Duplicates
            </button>
          )}
          {onDecryptMoggs && (
            <button className="toolbar-btn" onClick={onDecryptMoggs} title="Decrypt MOGGs">
              <LockOpen size={14} />
              Decrypt
            </button>
          )}
          {onOpenOptions && (
            <button className="toolbar-btn" onClick={onOpenOptions} title="Options">
              <Settings size={14} />
              Options
            </button>
          )}
        </div>
      )}
      {originButtons.length > 1 && (
        <div className="origin-filter-bar">
          <button
            className={`origin-filter-btn ${gameOriginFilter === null ? "active" : ""}`}
            onClick={() => onGameOriginFilter(null)}
            title="Show all songs"
          >
            All
          </button>
          {originButtons.map((o) => (
            <button
              key={o.id}
              className={`origin-filter-btn ${gameOriginFilter === o.id ? "active" : ""}`}
              onClick={() => onGameOriginFilter(gameOriginFilter === o.id ? null : o.id)}
              title={`${o.name} (${o.count})`}
            >
              <img src={`/icons/${o.icon}.png`} alt={o.name} />
              <span className="origin-filter-count">{o.count}</span>
            </button>
          ))}
        </div>
      )}
    </div>
  );
}
