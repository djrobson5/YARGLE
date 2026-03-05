import React, { useState, useEffect, useMemo } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen, UnlistenFn } from "@tauri-apps/api/event";
import { IconSelector } from "./IconSelector";
import sourcesData from "../data/sources.json";
import type { BatchValidateResult, SongValidationResult, SongDetails, ValidationIssue } from "../types";

const SOURCES_OPTIONS = (sourcesData.sources as { ids: string[]; names: { "en-US": string } }[])
  .filter((s) => s.ids[0] !== "$DEFAULT$")
  .map((s) => ({ value: s.ids[0], label: s.names["en-US"] }));

interface ValidatorModalProps {
  paths: string[];
  onClose: () => void;
}

interface ValidateProgress {
  current: number;
  total: number;
}

type FilterLevel = "all" | "Error" | "Warning" | "Info";

const GENRES = [
  "alternative", "blues", "classic", "classicrock", "country", "emo",
  "fusion", "glam", "grunge", "hiphoprap", "indierock", "inspirational",
  "jazz", "jrock", "latin", "metal", "new_wave", "novelty", "numetal",
  "other", "pop", "poprock", "prog", "punk", "rb", "reggaeska",
  "rock", "southernrock", "urban", "world",
];

const RATINGS = [
  { value: 1, label: "Family Friendly" },
  { value: 2, label: "Supervision Recommended" },
  { value: 3, label: "Mature Content" },
];

const FIXABLE_FIELDS = new Set([
  "game_origin", "genre", "author", "album_name", "year_released", "rating", "shortname",
]);

function isFixable(issue: ValidationIssue): boolean {
  return FIXABLE_FIELDS.has(issue.field);
}

function fileName(path: string): string {
  const parts = path.replace(/\\/g, "/").split("/");
  return parts[parts.length - 1] || path;
}

function FixInput({
  field,
  value,
  onChange,
  compact = false,
}: {
  field: string;
  value: string;
  onChange: (val: string) => void;
  compact?: boolean;
}) {
  if (field === "game_origin" && !compact) {
    return (
      <div className="validator-fix-icon-selector">
        <IconSelector value={value} onChange={onChange} />
      </div>
    );
  }
  if (field === "game_origin" && compact) {
    return (
      <select value={value} onChange={(e) => onChange(e.target.value)} autoFocus>
        <option value="">-- Select source --</option>
        {SOURCES_OPTIONS.map((s) => (
          <option key={s.value} value={s.value}>{s.label}</option>
        ))}
      </select>
    );
  }
  if (field === "genre") {
    return (
      <select value={value} onChange={(e) => onChange(e.target.value)} autoFocus>
        <option value="">--</option>
        {GENRES.map((g) => (
          <option key={g} value={g}>{g}</option>
        ))}
      </select>
    );
  }
  if (field === "rating") {
    return (
      <select value={value} onChange={(e) => onChange(e.target.value)} autoFocus>
        <option value="">--</option>
        {RATINGS.map((r) => (
          <option key={r.value} value={String(r.value)}>{r.label}</option>
        ))}
      </select>
    );
  }
  if (field === "year_released") {
    return (
      <input
        type="number"
        value={value}
        onChange={(e) => onChange(e.target.value)}
        placeholder="e.g. 2024"
        autoFocus
      />
    );
  }
  // text fields: author, album_name, shortname
  return (
    <input
      type="text"
      value={value}
      onChange={(e) => onChange(e.target.value)}
      placeholder={field.replace(/_/g, " ")}
      autoFocus
    />
  );
}

function coerceValue(field: string, raw: string): string | number | null {
  if (field === "year_released" || field === "rating") {
    if (raw === "") return null;
    const n = parseInt(raw, 10);
    return isNaN(n) ? null : n;
  }
  return raw;
}

export function ValidatorModal({ paths, onClose }: ValidatorModalProps) {
  const [state, setState] = useState<"ready" | "scanning" | "results">("ready");
  const [progress, setProgress] = useState<ValidateProgress | null>(null);
  const [result, setResult] = useState<BatchValidateResult | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [filter, setFilter] = useState<FilterLevel>("all");
  const [collapsed, setCollapsed] = useState<Set<string>>(new Set());

  // Per-issue fix state
  const [fixingIssue, setFixingIssue] = useState<{ path: string; field: string } | null>(null);
  const [fixValue, setFixValue] = useState("");
  const [fixSaving, setFixSaving] = useState(false);

  // Batch fix state
  const [batchFixField, setBatchFixField] = useState<string | null>(null);
  const [batchFixValue, setBatchFixValue] = useState("");
  const [batchFixProgress, setBatchFixProgress] = useState<{ current: number; total: number } | null>(null);

  useEffect(() => {
    let unlisten: UnlistenFn | null = null;
    listen<ValidateProgress>("batch-validate-progress", (event) => {
      setProgress(event.payload);
    }).then((fn) => {
      unlisten = fn;
    });
    return () => {
      if (unlisten) unlisten();
    };
  }, []);

  const handleScan = async () => {
    setState("scanning");
    setError(null);
    try {
      const res = await invoke<BatchValidateResult>("batch_validate", { paths });
      setResult(res);
      setState("results");
    } catch (e) {
      setError(String(e));
      setState("results");
    }
  };

  const toggleCollapse = (path: string) => {
    const next = new Set(collapsed);
    if (next.has(path)) next.delete(path);
    else next.add(path);
    setCollapsed(next);
  };

  // Apply a single fix
  const applyFix = async (path: string, field: string, rawValue: string) => {
    setFixSaving(true);
    try {
      const details = await invoke<SongDetails>("get_song_details", { path });
      const patched = { ...details.metadata, [field]: coerceValue(field, rawValue) };
      await invoke("save_song", { path, metadata: patched });
      // Remove fixed issue from local state
      removeIssue(path, field);
      setFixingIssue(null);
      setFixValue("");
    } catch (e) {
      setError(`Fix failed: ${e}`);
    } finally {
      setFixSaving(false);
    }
  };

  // Apply batch fix to all affected songs
  const applyBatchFix = async (field: string, rawValue: string) => {
    if (!result) return;
    const affected = result.results.filter((s) =>
      s.issues.some((i) => i.field === field)
    );
    setBatchFixProgress({ current: 0, total: affected.length });
    try {
      for (let idx = 0; idx < affected.length; idx++) {
        const song = affected[idx];
        const details = await invoke<SongDetails>("get_song_details", { path: song.path });
        const patched = { ...details.metadata, [field]: coerceValue(field, rawValue) };
        await invoke("save_song", { path: song.path, metadata: patched });
        removeIssue(song.path, field);
        setBatchFixProgress({ current: idx + 1, total: affected.length });
      }
    } catch (e) {
      setError(`Batch fix failed: ${e}`);
    } finally {
      setBatchFixField(null);
      setBatchFixValue("");
      setBatchFixProgress(null);
    }
  };

  // Remove a specific field's issues from a song, and clean up counts
  const removeIssue = (path: string, field: string) => {
    setResult((prev) => {
      if (!prev) return prev;
      const newResults = prev.results
        .map((song) => {
          if (song.path !== path) return song;
          return {
            ...song,
            issues: song.issues.filter((i) => i.field !== field),
          };
        })
        .filter((song) => song.issues.length > 0);

      // Recompute counts
      let errors = 0, warnings = 0;
      for (const s of newResults) {
        if (s.issues.some((i) => i.level === "Error")) errors++;
        if (s.issues.some((i) => i.level === "Warning")) warnings++;
      }
      return {
        ...prev,
        results: newResults,
        songs_with_errors: errors,
        songs_with_warnings: warnings,
        songs_clean: prev.total_songs - newResults.length - prev.parse_failures,
      };
    });
  };

  const filteredResults: SongValidationResult[] = result
    ? result.results.filter((r) => {
        if (filter === "all") return true;
        return r.issues.some((i) => i.level === filter);
      })
    : [];

  // Find the most common fixable field for batch fix
  const batchFixCandidate = useMemo(() => {
    if (!result || filteredResults.length === 0) return null;
    const fieldCounts: Record<string, number> = {};
    for (const song of filteredResults) {
      const issues = filter === "all" ? song.issues : song.issues.filter((i) => i.level === filter);
      for (const issue of issues) {
        if (isFixable(issue)) {
          fieldCounts[issue.field] = (fieldCounts[issue.field] || 0) + 1;
        }
      }
    }
    let bestField: string | null = null;
    let bestCount = 0;
    for (const [field, count] of Object.entries(fieldCounts)) {
      if (count > 1 && count > bestCount) {
        bestField = field;
        bestCount = count;
      }
    }
    return bestField ? { field: bestField, count: bestCount } : null;
  }, [result, filteredResults, filter]);

  const progressPct = progress
    ? Math.round((progress.current / progress.total) * 100)
    : 0;

  return (
    <div className="art-search-overlay" onClick={onClose}>
      <div
        className="art-search-panel validator-panel"
        onClick={(e) => e.stopPropagation()}
      >
        <div className="art-search-header">
          <h3>Validate Songs</h3>
          <button className="art-search-close" onClick={onClose}>
            &times;
          </button>
        </div>

        <div className="validator-body">
          {state === "ready" && (
            <>
              <p className="mogg-decrypt-desc">
                Check {paths.length} song{paths.length !== 1 ? "s" : ""} for
                missing or invalid metadata. Catches common issues that can cause
                YARG to skip or misidentify songs.
              </p>
              <button className="mogg-decrypt-start" onClick={handleScan}>
                Run Validation
              </button>
            </>
          )}

          {state === "scanning" && (
            <div className="mogg-decrypt-progress">
              {progress ? (
                <>
                  <div className="mogg-decrypt-bar-outer">
                    <div
                      className="mogg-decrypt-bar-inner"
                      style={{ width: `${progressPct}%` }}
                    />
                  </div>
                  <div className="mogg-decrypt-status">
                    Validating ({progress.current} / {progress.total})
                  </div>
                </>
              ) : (
                <div className="art-search-loading">Starting validation...</div>
              )}
            </div>
          )}

          {state === "results" && error && (
            <div className="art-search-error">{error}</div>
          )}

          {state === "results" && !error && result && (
            <>
              <div className="validator-summary">
                {result.songs_with_errors > 0 && (
                  <span className="validator-stat validator-stat-error">
                    {result.songs_with_errors} error{result.songs_with_errors !== 1 ? "s" : ""}
                  </span>
                )}
                {result.songs_with_warnings > 0 && (
                  <span className="validator-stat validator-stat-warning">
                    {result.songs_with_warnings} warning{result.songs_with_warnings !== 1 ? "s" : ""}
                  </span>
                )}
                <span className="validator-stat validator-stat-clean">
                  {result.songs_clean} clean
                </span>
                {result.parse_failures > 0 && (
                  <span className="validator-stat validator-stat-error">
                    {result.parse_failures} failed to parse
                  </span>
                )}
              </div>

              <div className="validator-filters">
                {(["all", "Error", "Warning", "Info"] as FilterLevel[]).map((lvl) => (
                  <button
                    key={lvl}
                    className={`validator-filter-btn ${filter === lvl ? "active" : ""}`}
                    onClick={() => setFilter(lvl)}
                  >
                    {lvl === "all" ? "All" : lvl + "s"}
                  </button>
                ))}
              </div>

              {/* Batch fix bar */}
              {batchFixCandidate && !batchFixField && !batchFixProgress && (
                <div className="validator-batch-bar">
                  <span>
                    {batchFixCandidate.count} songs have <strong>{batchFixCandidate.field.replace(/_/g, " ")}</strong> issues
                  </span>
                  <button
                    className="validator-fix-btn"
                    onClick={() => {
                      setBatchFixField(batchFixCandidate.field);
                      setBatchFixValue("");
                    }}
                  >
                    Fix All
                  </button>
                </div>
              )}

              {/* Batch fix editor */}
              {batchFixField && !batchFixProgress && (
                <div className="validator-batch-bar">
                  <span>Set <strong>{batchFixField.replace(/_/g, " ")}</strong> for all:</span>
                  <div className="validator-fix-inline">
                    <FixInput
                      field={batchFixField}
                      value={batchFixValue}
                      onChange={setBatchFixValue}
                    />
                    <button
                      className="validator-fix-btn validator-fix-apply"
                      disabled={!batchFixValue}
                      onClick={() => applyBatchFix(batchFixField, batchFixValue)}
                    >
                      Apply
                    </button>
                    <button
                      className="validator-fix-btn validator-fix-cancel"
                      onClick={() => { setBatchFixField(null); setBatchFixValue(""); }}
                    >
                      Cancel
                    </button>
                  </div>
                </div>
              )}

              {/* Batch fix progress */}
              {batchFixProgress && (
                <div className="validator-batch-bar validator-fixing">
                  Fixing {batchFixProgress.current} / {batchFixProgress.total}...
                </div>
              )}

              {filteredResults.length === 0 ? (
                <div className="duplicate-no-results">
                  <p>{filter === "all" ? "All songs passed validation!" : `No ${filter.toLowerCase()}-level issues found.`}</p>
                  <button className="mogg-decrypt-start" onClick={onClose}>
                    Done
                  </button>
                </div>
              ) : (
                <div className="validator-results">
                  {filteredResults.map((song) => {
                    const isCollapsed = collapsed.has(song.path);
                    const hasError = song.issues.some((i) => i.level === "Error");
                    const displayIssues = filter === "all"
                      ? song.issues
                      : song.issues.filter((i) => i.level === filter);
                    return (
                      <div key={song.path} className="validator-song-card">
                        <div
                          className={`validator-song-header ${hasError ? "has-error" : ""}`}
                          onClick={() => toggleCollapse(song.path)}
                        >
                          <span className="validator-expand-icon">
                            {isCollapsed ? "\u25B6" : "\u25BC"}
                          </span>
                          <span className="validator-song-name">
                            {song.display_name || fileName(song.path)}
                          </span>
                          <span className="validator-issue-count">
                            {displayIssues.length} issue{displayIssues.length !== 1 ? "s" : ""}
                          </span>
                        </div>
                        {!isCollapsed && (
                          <div className="validator-song-issues">
                            {displayIssues.map((issue, i) => {
                              const isEditing = fixingIssue?.path === song.path && fixingIssue?.field === issue.field;
                              return (
                                <div key={i}>
                                  <div
                                    className={`validation-row validation-${issue.level.toLowerCase()}`}
                                  >
                                    <span className="validation-icon">
                                      {issue.level === "Error"
                                        ? "\u2716"
                                        : issue.level === "Warning"
                                        ? "\u26A0"
                                        : "\u2139"}
                                    </span>
                                    <span className="validation-message">{issue.message}</span>
                                    {isFixable(issue) && !isEditing && (
                                      <button
                                        className="validator-fix-btn"
                                        onClick={(e) => {
                                          e.stopPropagation();
                                          setFixingIssue({ path: song.path, field: issue.field });
                                          setFixValue("");
                                        }}
                                      >
                                        Fix
                                      </button>
                                    )}
                                  </div>
                                  {isEditing && (
                                    <div className="validator-fix-inline">
                                      <FixInput
                                        field={issue.field}
                                        value={fixValue}
                                        onChange={setFixValue}
                                        compact
                                      />
                                      <button
                                        className="validator-fix-btn validator-fix-apply"
                                        disabled={!fixValue || fixSaving}
                                        onClick={() => applyFix(song.path, issue.field, fixValue)}
                                      >
                                        {fixSaving ? "..." : "Apply"}
                                      </button>
                                      <button
                                        className="validator-fix-btn validator-fix-cancel"
                                        onClick={() => { setFixingIssue(null); setFixValue(""); }}
                                      >
                                        Cancel
                                      </button>
                                    </div>
                                  )}
                                </div>
                              );
                            })}
                          </div>
                        )}
                      </div>
                    );
                  })}
                </div>
              )}
            </>
          )}
        </div>
      </div>
    </div>
  );
}
