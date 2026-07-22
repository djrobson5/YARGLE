import React, { useMemo } from "react";
import type { SongSummary } from "../types";
import sourcesData from "../data/sources.json";

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
  gameOriginFilter: string | null;
  onGameOriginFilter: (origin: string | null) => void;
  multiSelectedCount: number;
  onClearMultiSelect: () => void;
  onSelectAllVisible: () => void;
  filteredCount: number;
}

export function SearchBar({ value, onChange, onOpenFolder, onOpenOptions, onDecryptMoggs, onFindDuplicates, onBatchRename, onBatchEdit, onOrganize, onValidate, onBrowseRhythmVerse, songCount, songs, gameOriginFilter, onGameOriginFilter, multiSelectedCount, onClearMultiSelect, onSelectAllVisible, filteredCount }: SearchBarProps) {
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
        <button className="open-folder-btn" onClick={onOpenFolder} title="Open Folder">
          Open Folder
        </button>
        {onBrowseRhythmVerse && (
          <button
            className="open-folder-btn rv-open-btn"
            onClick={onBrowseRhythmVerse}
            title="Browse and download songs from RhythmVerse"
          >
            Browse RhythmVerse
          </button>
        )}
        <input
          type="text"
          placeholder="Filter"
          value={value}
          onChange={(e) => onChange(e.target.value)}
          className="search-input"
        />
        {songCount > 0 && <span className="song-count">{songCount} songs</span>}
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
              <svg width="14" height="14" viewBox="0 0 16 16" fill="currentColor">
                <path d="M1 3.5h9v1H1zm0 4h9v1H1zm0 4h6v1H1z" />
                <path d="M12.5 8.5l2.5 2.5-2.5 2.5M11 11h4" fill="none" stroke="currentColor" strokeWidth="1.2" strokeLinecap="round" strokeLinejoin="round"/>
              </svg>
              Rename
            </button>
          )}
          {onBatchEdit && (
            <button className="toolbar-btn" onClick={onBatchEdit} title="Batch Edit Metadata">
              <svg width="14" height="14" viewBox="0 0 16 16" fill="currentColor">
                <path d="M12.146.146a.5.5 0 0 1 .708 0l3 3a.5.5 0 0 1 0 .708l-10 10a.5.5 0 0 1-.168.11l-5 2a.5.5 0 0 1-.65-.65l2-5a.5.5 0 0 1 .11-.168l10-10zM11.207 2.5 13.5 4.793 14.793 3.5 12.5 1.207 11.207 2.5zm1.586 3L10.5 3.207 4 9.707V10h.5a.5.5 0 0 1 .5.5v.5h.5a.5.5 0 0 1 .5.5v.5h.293l6.5-6.5z"/>
              </svg>
              Batch Edit
            </button>
          )}
          {onOrganize && (
            <button className="toolbar-btn" onClick={onOrganize} title="Auto-Organize into Artist/Album folders">
              <svg width="14" height="14" viewBox="0 0 16 16" fill="currentColor">
                <path d="M1 2.5A1.5 1.5 0 0 1 2.5 1h3.879a1.5 1.5 0 0 1 1.06.44l1.122 1.12A.5.5 0 0 0 8.914 3H13.5A1.5 1.5 0 0 1 15 4.5v2a.5.5 0 0 1-1 0v-2a.5.5 0 0 0-.5-.5H8.914a1.5 1.5 0 0 1-1.06-.44L6.732 2.44A.5.5 0 0 0 6.379 2H2.5a.5.5 0 0 0-.5.5v10a.5.5 0 0 0 .5.5h3a.5.5 0 0 1 0 1h-3A1.5 1.5 0 0 1 1 12.5v-10z"/>
                <path d="M11 8.5a.5.5 0 0 1 .5-.5h4a.5.5 0 0 1 .354.854l-2 2a.5.5 0 0 1-.708-.708L14.293 9H11.5a.5.5 0 0 1-.5-.5z" fill="none" stroke="currentColor" strokeWidth="0.3"/>
                <path d="M9 11.5a.5.5 0 0 1 .5-.5h4a.5.5 0 0 1 .354.854l-2 2a.5.5 0 0 1-.708-.708L12.293 12H9.5a.5.5 0 0 1-.5-.5z" fill="none" stroke="currentColor" strokeWidth="0.3"/>
                <rect x="9" y="8" width="6" height="1" rx="0.3"/>
                <rect x="9" y="11" width="6" height="1" rx="0.3"/>
                <rect x="9" y="14" width="4" height="1" rx="0.3"/>
              </svg>
              Organize
            </button>
          )}
          {onValidate && (
            <button className="toolbar-btn" onClick={onValidate} title="Validate Song Metadata">
              <svg width="14" height="14" viewBox="0 0 16 16" fill="currentColor">
                <path d="M8 1a7 7 0 1 0 0 14A7 7 0 0 0 8 1zm0 1.2A5.8 5.8 0 1 1 8 12.8 5.8 5.8 0 0 1 8 2.2z"/>
                <path d="M7 7.5a1 1 0 1 1 2 0v3a1 1 0 1 1-2 0zM8 4.5a1 1 0 1 1 0 2 1 1 0 0 1 0-2z"/>
              </svg>
              Validate
            </button>
          )}
          {onFindDuplicates && (
            <button className="toolbar-btn" onClick={onFindDuplicates} title="Find Duplicates">
              <svg width="14" height="14" viewBox="0 0 16 16" fill="currentColor">
                <rect x="1" y="1" width="9" height="11" rx="1.5" fill="none" stroke="currentColor" strokeWidth="1.2"/>
                <rect x="5" y="4" width="9" height="11" rx="1.5" fill="none" stroke="currentColor" strokeWidth="1.2"/>
              </svg>
              Duplicates
            </button>
          )}
          {onDecryptMoggs && (
            <button className="toolbar-btn" onClick={onDecryptMoggs} title="Decrypt MOGGs">
              <svg width="14" height="14" viewBox="0 0 16 16" fill="currentColor">
                <rect x="0.5" y="6" width="1.4" height="4" rx="0.5"/>
                <rect x="2.8" y="4" width="1.4" height="8" rx="0.5"/>
                <rect x="5.1" y="5" width="1.4" height="6" rx="0.5"/>
                <rect x="7.4" y="3" width="1.4" height="10" rx="0.5"/>
                <path d="M12.8 7.5V6a1.2 1.2 0 0 1 2.4 0" strokeWidth="1" stroke="currentColor" fill="none" strokeLinecap="round"/>
                <rect x="11.5" y="8.5" width="3.8" height="3.2" rx="0.7"/>
              </svg>
              Decrypt
            </button>
          )}
          {onOpenOptions && (
            <button className="toolbar-btn" onClick={onOpenOptions} title="Options">
              <svg width="14" height="14" viewBox="0 0 16 16" fill="currentColor">
                <path d="M8 4.754a3.246 3.246 0 1 0 0 6.492 3.246 3.246 0 0 0 0-6.492zM5.754 8a2.246 2.246 0 1 1 4.492 0 2.246 2.246 0 0 1-4.492 0z"/>
                <path d="M9.796 1.343c-.527-1.79-3.065-1.79-3.592 0l-.094.319a.873.873 0 0 1-1.255.52l-.292-.16c-1.64-.892-3.433.902-2.54 2.541l.159.292a.873.873 0 0 1-.52 1.255l-.319.094c-1.79.527-1.79 3.065 0 3.592l.319.094a.873.873 0 0 1 .52 1.255l-.16.292c-.892 1.64.901 3.434 2.541 2.54l.292-.159a.873.873 0 0 1 1.255.52l.094.319c.527 1.79 3.065 1.79 3.592 0l.094-.319a.873.873 0 0 1 1.255-.52l.292.16c1.64.893 3.434-.902 2.54-2.541l-.159-.292a.873.873 0 0 1 .52-1.255l.319-.094c1.79-.527 1.79-3.065 0-3.592l-.319-.094a.873.873 0 0 1-.52-1.255l.16-.292c.893-1.64-.902-3.433-2.541-2.54l-.292.159a.873.873 0 0 1-1.255-.52l-.094-.319zm-2.633.283c.246-.835 1.428-.835 1.674 0l.094.319a1.873 1.873 0 0 0 2.693 1.115l.291-.16c.764-.415 1.6.42 1.184 1.185l-.159.292a1.873 1.873 0 0 0 1.116 2.692l.318.094c.835.246.835 1.428 0 1.674l-.319.094a1.873 1.873 0 0 0-1.115 2.693l.16.291c.415.764-.42 1.6-1.185 1.184l-.291-.159a1.873 1.873 0 0 0-2.693 1.116l-.094.318c-.246.835-1.428.835-1.674 0l-.094-.319a1.873 1.873 0 0 0-2.692-1.115l-.292.16c-.764.415-1.6-.42-1.184-1.185l.159-.291A1.873 1.873 0 0 0 1.945 8.93l-.319-.094c-.835-.246-.835-1.428 0-1.674l.319-.094A1.873 1.873 0 0 0 3.06 4.377l-.16-.292c-.415-.764.42-1.6 1.185-1.184l.292.159a1.873 1.873 0 0 0 2.692-1.116l.094-.318z"/>
              </svg>
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
