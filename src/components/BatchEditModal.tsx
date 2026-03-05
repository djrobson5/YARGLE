import React, { useState, useEffect } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen, UnlistenFn } from "@tauri-apps/api/event";
import type { SongMetadata } from "../types";

interface BatchEditModalProps {
  paths: string[];
  onClose: (edited: boolean) => void;
}

interface SongFieldPreview {
  path: string;
  filename: string;
  current_value: string;
  metadata: SongMetadata;
}

interface FieldProgress {
  current: number;
  total: number;
}

const BATCH_FIELDS: { value: string; label: string; type: "text" | "select" }[] = [
  { value: "author", label: "Author", type: "text" },
  { value: "genre", label: "Genre", type: "select" },
  { value: "sub_genre", label: "Sub-genre", type: "text" },
  { value: "vocal_gender", label: "Vocal Gender", type: "select" },
  { value: "game_origin", label: "Game Origin", type: "text" },
  { value: "rating", label: "Rating", type: "select" },
  { value: "year_released", label: "Year Released", type: "text" },
];

const GENRES = [
  "alternative", "blues", "classic", "classicrock", "country", "emo",
  "fusion", "glam", "grunge", "hiphoprap", "indierock", "inspirational",
  "jazz", "jrock", "latin", "metal", "new_wave", "novelty", "numetal",
  "other", "pop", "poprock", "prog", "punk", "rb", "reggaeska",
  "rock", "southernrock", "urban", "world",
];

const VOCAL_GENDERS = ["male", "female"];

const RATINGS = [
  { value: "1", label: "Family Friendly" },
  { value: "2", label: "Supervision Recommended" },
  { value: "3", label: "Mature Content" },
];

export function BatchEditModal({ paths, onClose }: BatchEditModalProps) {
  const [state, setState] = useState<"pick" | "scanning" | "review" | "applying" | "done">("pick");
  const [field, setField] = useState("author");
  const [newValue, setNewValue] = useState("");
  const [progress, setProgress] = useState<FieldProgress | null>(null);
  const [previews, setPreviews] = useState<SongFieldPreview[]>([]);
  const [selected, setSelected] = useState<Set<string>>(new Set());
  const [error, setError] = useState<string | null>(null);
  const [didEdit, setDidEdit] = useState(false);

  // Apply state
  const [applyProgress, setApplyProgress] = useState({ current: 0, total: 0 });
  const [applyErrors, setApplyErrors] = useState<string[]>([]);
  const [applySuccess, setApplySuccess] = useState(0);

  useEffect(() => {
    let unlisten: UnlistenFn | null = null;
    listen<FieldProgress>("batch-field-progress", (event) => {
      setProgress(event.payload);
    }).then((fn) => {
      unlisten = fn;
    });
    return () => {
      if (unlisten) unlisten();
    };
  }, []);

  const fieldDef = BATCH_FIELDS.find((f) => f.value === field)!;

  const handleScan = async () => {
    setState("scanning");
    setError(null);
    setProgress(null);
    try {
      const result = await invoke<SongFieldPreview[]>("batch_get_field", {
        paths,
        field,
      });
      setPreviews(result);
      // Auto-select files whose current value differs from newValue
      const toSelect = new Set(
        result
          .filter((r) => r.current_value !== newValue)
          .map((r) => r.path)
      );
      setSelected(toSelect);
      setState("review");
    } catch (e) {
      setError(String(e));
      setState("review");
    }
  };

  const toggleSelect = (path: string) => {
    const next = new Set(selected);
    if (next.has(path)) next.delete(path);
    else next.add(path);
    setSelected(next);
  };

  const toggleAll = () => {
    const changeable = previews.filter((p) => p.current_value !== newValue);
    if (selected.size === changeable.length) {
      setSelected(new Set());
    } else {
      setSelected(new Set(changeable.map((p) => p.path)));
    }
  };

  const handleApply = async () => {
    const toApply = previews.filter((p) => selected.has(p.path));
    setState("applying");
    setApplyProgress({ current: 0, total: toApply.length });
    setApplyErrors([]);
    setApplySuccess(0);

    let successCount = 0;
    const errors: string[] = [];

    for (let i = 0; i < toApply.length; i++) {
      const item = toApply[i];
      const patched = { ...item.metadata };

      // Patch the single field
      if (field === "rating") {
        (patched as any)[field] = newValue ? parseInt(newValue, 10) : null;
      } else if (field === "year_released") {
        (patched as any)[field] = newValue ? parseInt(newValue, 10) : null;
      } else {
        (patched as any)[field] = newValue;
      }

      try {
        await invoke("save_song", {
          path: item.path,
          displayName: null,
          description: null,
          metadata: patched,
          thumbnailBase64: null,
        });
        successCount++;
      } catch (e) {
        errors.push(`${item.filename}: ${e}`);
      }

      setApplyProgress({ current: i + 1, total: toApply.length });
      setApplySuccess(successCount);
      setApplyErrors([...errors]);
    }

    if (successCount > 0) setDidEdit(true);
    setState("done");
  };

  const progressPct = progress
    ? Math.round((progress.current / progress.total) * 100)
    : 0;
  const applyPct = applyProgress.total
    ? Math.round((applyProgress.current / applyProgress.total) * 100)
    : 0;

  const matchCount = previews.filter((p) => p.current_value === newValue).length;
  const changeCount = previews.filter((p) => p.current_value !== newValue).length;

  const renderValueInput = () => {
    if (field === "genre") {
      return (
        <select
          className="batch-edit-select"
          value={newValue}
          onChange={(e) => setNewValue(e.target.value)}
        >
          <option value="">--</option>
          {GENRES.map((g) => (
            <option key={g} value={g}>{g}</option>
          ))}
        </select>
      );
    }
    if (field === "vocal_gender") {
      return (
        <select
          className="batch-edit-select"
          value={newValue}
          onChange={(e) => setNewValue(e.target.value)}
        >
          <option value="">--</option>
          {VOCAL_GENDERS.map((g) => (
            <option key={g} value={g}>{g}</option>
          ))}
        </select>
      );
    }
    if (field === "rating") {
      return (
        <select
          className="batch-edit-select"
          value={newValue}
          onChange={(e) => setNewValue(e.target.value)}
        >
          <option value="">--</option>
          {RATINGS.map((r) => (
            <option key={r.value} value={r.value}>{r.label}</option>
          ))}
        </select>
      );
    }
    return (
      <input
        type={field === "year_released" ? "number" : "text"}
        className="batch-edit-input"
        placeholder={`New ${fieldDef.label} value...`}
        value={newValue}
        onChange={(e) => setNewValue(e.target.value)}
      />
    );
  };

  return (
    <div className="art-search-overlay" onClick={() => onClose(didEdit)}>
      <div
        className="art-search-panel batch-edit-panel"
        onClick={(e) => e.stopPropagation()}
      >
        <div className="art-search-header">
          <h3>Batch Edit Metadata</h3>
          <button
            className="art-search-close"
            onClick={() => onClose(didEdit)}
          >
            &times;
          </button>
        </div>

        <div className="rename-body">
          {state === "pick" && (
            <>
              <p className="mogg-decrypt-desc">
                Edit a single metadata field across all {paths.length} loaded song
                {paths.length !== 1 ? "s" : ""}. Pick the field and new value,
                then scan to preview changes.
              </p>
              <div className="batch-edit-field-picker">
                <div className="field">
                  <label>Field</label>
                  <select
                    value={field}
                    onChange={(e) => {
                      setField(e.target.value);
                      setNewValue("");
                    }}
                  >
                    {BATCH_FIELDS.map((f) => (
                      <option key={f.value} value={f.value}>
                        {f.label}
                      </option>
                    ))}
                  </select>
                </div>
                <div className="field">
                  <label>New Value</label>
                  {renderValueInput()}
                </div>
              </div>
              <button
                className="mogg-decrypt-start"
                onClick={handleScan}
                disabled={!newValue && field !== "game_origin" && field !== "sub_genre" && field !== "author"}
              >
                Scan Files
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
                    Reading metadata... ({progress.current} / {progress.total})
                  </div>
                </>
              ) : (
                <div className="art-search-loading">Starting scan...</div>
              )}
            </div>
          )}

          {state === "review" && error && (
            <div className="art-search-error">{error}</div>
          )}

          {state === "review" && !error && (
            <>
              <div className="batch-edit-summary">
                <span className="batch-edit-field-label">
                  Setting <strong>{fieldDef.label}</strong> to{" "}
                  <strong>"{newValue || "(empty)"}"</strong>
                </span>
                {matchCount > 0 && (
                  <span className="batch-edit-skip-note">
                    {matchCount} already match (skipped)
                  </span>
                )}
              </div>

              {changeCount === 0 ? (
                <div className="duplicate-no-results">
                  <p>All files already have this value.</p>
                  <button
                    className="mogg-decrypt-start"
                    onClick={() => onClose(false)}
                  >
                    Done
                  </button>
                </div>
              ) : (
                <>
                  <div className="batch-edit-select-all">
                    <label>
                      <input
                        type="checkbox"
                        checked={selected.size === changeCount}
                        onChange={toggleAll}
                      />
                      Select all ({changeCount})
                    </label>
                  </div>
                  <div className="rename-list">
                    {previews.map((p) => {
                      const isMatch = p.current_value === newValue;
                      return (
                        <label
                          key={p.path}
                          className={`rename-row ${isMatch ? "rename-row-skip" : ""}`}
                        >
                          {isMatch ? (
                            <span className="rename-skip-icon" />
                          ) : (
                            <input
                              type="checkbox"
                              checked={selected.has(p.path)}
                              onChange={() => toggleSelect(p.path)}
                            />
                          )}
                          <div className="rename-row-info">
                            <div className="rename-current">{p.filename}</div>
                            {isMatch ? (
                              <div className="rename-skip-reason">
                                Already "{p.current_value}"
                              </div>
                            ) : (
                              <div className="rename-arrow-row">
                                <span className="batch-edit-old-value">
                                  {p.current_value || "(empty)"}
                                </span>
                                <span className="rename-arrow">&rarr;</span>
                                <span className="rename-new">
                                  {newValue || "(empty)"}
                                </span>
                              </div>
                            )}
                          </div>
                        </label>
                      );
                    })}
                  </div>

                  <div className="duplicate-actions">
                    <button
                      className="duplicate-delete-btn"
                      disabled={selected.size === 0}
                      onClick={handleApply}
                    >
                      Apply to Selected ({selected.size})
                    </button>
                  </div>
                </>
              )}
            </>
          )}

          {state === "applying" && (
            <div className="mogg-decrypt-progress">
              <div className="mogg-decrypt-bar-outer">
                <div
                  className="mogg-decrypt-bar-inner"
                  style={{ width: `${applyPct}%` }}
                />
              </div>
              <div className="mogg-decrypt-status">
                Applying changes... ({applyProgress.current} / {applyProgress.total})
              </div>
            </div>
          )}

          {state === "done" && (
            <div className="rename-done">
              <div className="mogg-decrypt-summary">
                {applySuccess > 0 && (
                  <div className="mogg-result-row mogg-result-success">
                    {applySuccess} file{applySuccess !== 1 ? "s" : ""} updated
                  </div>
                )}
                {matchCount > 0 && (
                  <div className="mogg-result-row mogg-result-skip">
                    {matchCount} already matched (skipped)
                  </div>
                )}
                {applyErrors.length > 0 && (
                  <div className="mogg-result-row mogg-result-error">
                    {applyErrors.length} error{applyErrors.length !== 1 ? "s" : ""}
                  </div>
                )}
              </div>

              {applyErrors.length > 0 && (
                <div className="mogg-decrypt-errors">
                  {applyErrors.map((err, i) => (
                    <div key={i} className="mogg-error-item">{err}</div>
                  ))}
                </div>
              )}

              <button
                className="mogg-decrypt-start"
                onClick={() => onClose(didEdit)}
              >
                Done
              </button>
            </div>
          )}
        </div>
      </div>
    </div>
  );
}
