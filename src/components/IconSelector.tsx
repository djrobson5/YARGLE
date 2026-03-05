import React, { useMemo, useRef, useState, useEffect } from "react";
import sourcesData from "../data/sources.json";

interface SourceEntry {
  ids: string[];
  names: { "en-US": string };
  icon: string;
  type: string;
}

interface SourceInfo {
  name: string;
  icon: string;
  type: string;
  primaryId: string;
}

const TYPE_LABELS: Record<string, string> = {
  rb: "Rock Band",
  gh: "Guitar Hero",
  game: "Games",
  custom: "Community Packs",
  charter: "Charters",
};

const TYPE_ORDER = ["rb", "gh", "game", "custom", "charter"];

function buildLookup(sources: SourceEntry[]): Map<string, SourceInfo> {
  const map = new Map<string, SourceInfo>();
  for (const s of sources) {
    const info: SourceInfo = {
      name: s.names["en-US"],
      icon: s.icon,
      type: s.type,
      primaryId: s.ids[0],
    };
    for (const id of s.ids) {
      map.set(id, info);
    }
  }
  return map;
}

// Deduplicated list for the grid (one entry per unique source, not per alias)
function buildGridEntries(sources: SourceEntry[]): SourceInfo[] {
  return sources
    .filter((s) => s.ids[0] !== "$DEFAULT$")
    .map((s) => ({
      name: s.names["en-US"],
      icon: s.icon,
      type: s.type,
      primaryId: s.ids[0],
    }));
}

interface IconSelectorProps {
  value: string;
  onChange: (val: string) => void;
}

export function IconSelector({ value, onChange }: IconSelectorProps) {
  const [open, setOpen] = useState(false);
  const [search, setSearch] = useState("");
  const containerRef = useRef<HTMLDivElement>(null);

  const lookup = useMemo(
    () => buildLookup(sourcesData.sources as SourceEntry[]),
    []
  );
  const gridEntries = useMemo(
    () => buildGridEntries(sourcesData.sources as SourceEntry[]),
    []
  );

  const matched = lookup.get(value);

  // Close on click outside
  useEffect(() => {
    if (!open) return;
    const handler = (e: MouseEvent) => {
      if (
        containerRef.current &&
        !containerRef.current.contains(e.target as Node)
      ) {
        setOpen(false);
        setSearch("");
      }
    };
    document.addEventListener("mousedown", handler);
    return () => document.removeEventListener("mousedown", handler);
  }, [open]);

  const filtered = useMemo(() => {
    if (!search) return gridEntries;
    const q = search.toLowerCase();
    return gridEntries.filter(
      (e) =>
        e.name.toLowerCase().includes(q) || e.primaryId.toLowerCase().includes(q)
    );
  }, [gridEntries, search]);

  const grouped = useMemo(() => {
    const groups: Record<string, SourceInfo[]> = {};
    for (const e of filtered) {
      (groups[e.type] ??= []).push(e);
    }
    return groups;
  }, [filtered]);

  const handleSelect = (id: string) => {
    onChange(id);
    setOpen(false);
    setSearch("");
  };

  return (
    <div className="icon-selector" ref={containerRef}>
      <label>Game Origin</label>
      <div className="icon-selector-trigger" onClick={() => setOpen(!open)}>
        {matched ? (
          <>
            <img
              className="icon-selector-preview"
              src={`/icons/${matched.icon}.png`}
              alt={matched.name}
            />
            <span className="icon-selector-name">{matched.name}</span>
          </>
        ) : value ? (
          <>
            <img
              className="icon-selector-preview"
              src="/icons/custom.png"
              alt="custom"
            />
            <span className="icon-selector-name">{value}</span>
          </>
        ) : (
          <span className="icon-selector-name icon-selector-placeholder">
            Select source...
          </span>
        )}
        <span className="icon-selector-arrow">&#9662;</span>
      </div>

      {open && (
        <div className="icon-dropdown">
          <input
            className="icon-search"
            type="text"
            placeholder="Search or type custom ID..."
            value={search}
            onChange={(e) => setSearch(e.target.value)}
            onKeyDown={(e) => {
              if (e.key === "Enter" && search) {
                handleSelect(search);
              }
            }}
            autoFocus
          />
          <div className="icon-grid-scroll">
            {TYPE_ORDER.map((type) => {
              const items = grouped[type];
              if (!items || items.length === 0) return null;
              return (
                <div key={type} className="icon-group">
                  <div className="icon-group-label">
                    {TYPE_LABELS[type] || type}
                  </div>
                  <div className="icon-grid">
                    {items.map((entry) => (
                      <div
                        key={entry.primaryId}
                        className={`icon-grid-item ${value === entry.primaryId ? "active" : ""}`}
                        onClick={() => handleSelect(entry.primaryId)}
                        title={entry.name}
                      >
                        <img
                          src={`/icons/${entry.icon}.png`}
                          alt={entry.name}
                        />
                        <span>{entry.name}</span>
                      </div>
                    ))}
                  </div>
                </div>
              );
            })}
            {filtered.length === 0 && search && (
              <div className="icon-no-results">
                No match. Press Enter to use "{search}" as custom value.
              </div>
            )}
          </div>
        </div>
      )}
    </div>
  );
}
