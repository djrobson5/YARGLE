import React, { useState, useEffect } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen, UnlistenFn } from "@tauri-apps/api/event";

interface DuplicateModalProps {
  paths: string[];
  onClose: (deleted: boolean) => void;
}

interface DuplicateEntry {
  path: string;
  display_name: string;
  description: string;
  file_size: number;
  has_drums: boolean;
  has_guitar: boolean;
  has_bass: boolean;
  has_vocals: boolean;
  has_keys: boolean;
}

interface DuplicateGroup {
  shortname: string;
  display_name: string;
  entries: DuplicateEntry[];
}

interface ScanProgress {
  current: number;
  total: number;
  phase: string;
}

function formatSize(bytes: number): string {
  if (bytes < 1024) return `${bytes} B`;
  if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(1)} KB`;
  return `${(bytes / (1024 * 1024)).toFixed(1)} MB`;
}

function fileName(path: string): string {
  const parts = path.replace(/\\/g, "/").split("/");
  return parts[parts.length - 1] || path;
}

export function DuplicateModal({ paths, onClose }: DuplicateModalProps) {
  const [state, setState] = useState<"ready" | "scanning" | "results">("ready");
  const [progress, setProgress] = useState<ScanProgress | null>(null);
  const [groups, setGroups] = useState<DuplicateGroup[]>([]);
  const [selected, setSelected] = useState<Set<string>>(new Set());
  const [error, setError] = useState<string | null>(null);
  const [deleting, setDeleting] = useState(false);
  const [confirmDelete, setConfirmDelete] = useState(false);
  const [didDelete, setDidDelete] = useState(false);
  const [deleteErrors, setDeleteErrors] = useState<string[]>([]);

  useEffect(() => {
    let unlisten: UnlistenFn | null = null;
    listen<ScanProgress>("duplicate-scan-progress", (event) => {
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
      const result = await invoke<DuplicateGroup[]>("find_duplicates", { paths });
      setGroups(result);
      setState("results");
    } catch (e) {
      setError(String(e));
      setState("results");
    }
  };

  const toggleSelect = (path: string, group: DuplicateGroup) => {
    const next = new Set(selected);
    if (next.has(path)) {
      next.delete(path);
    } else {
      // Don't allow selecting ALL entries in a group
      const groupPaths = group.entries.map((e) => e.path);
      const wouldBeSelected = groupPaths.filter((p) => p === path || next.has(p));
      if (wouldBeSelected.length >= group.entries.length) {
        return; // Would select all — block it
      }
      next.add(path);
    }
    setSelected(next);
  };

  const handleDelete = async () => {
    setDeleting(true);
    setConfirmDelete(false);
    try {
      const toDelete = Array.from(selected);
      const failures = await invoke<string[]>("delete_files", { paths: toDelete });
      setDeleteErrors(failures);

      // Remove deleted entries from groups
      const deletedSet = new Set(
        toDelete.filter((p) => !failures.some((f) => f.startsWith(p)))
      );
      if (deletedSet.size > 0) setDidDelete(true);

      const updated = groups
        .map((g) => ({
          ...g,
          entries: g.entries.filter((e) => !deletedSet.has(e.path)),
        }))
        .filter((g) => g.entries.length > 1);

      setGroups(updated);
      setSelected(new Set());
    } catch (e) {
      setDeleteErrors([String(e)]);
    }
    setDeleting(false);
  };

  const progressPct = progress
    ? Math.round((progress.current / progress.total) * 100)
    : 0;

  return (
    <div className="art-search-overlay" onClick={() => onClose(didDelete)}>
      <div
        className="art-search-panel duplicate-panel"
        onClick={(e) => e.stopPropagation()}
      >
        <div className="art-search-header">
          <h3>Find Duplicates</h3>
          <button
            className="art-search-close"
            onClick={() => onClose(didDelete)}
          >
            &times;
          </button>
        </div>

        <div className="duplicate-body">
          {state === "ready" && (
            <>
              <p className="mogg-decrypt-desc">
                Scan {paths.length} CON file{paths.length !== 1 ? "s" : ""} for
                duplicate songs. Matches by display name, then verifies using
                the DTA shortname.
              </p>
              <button className="mogg-decrypt-start" onClick={handleScan}>
                Scan for Duplicates
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
                    {progress.phase} ({progress.current} / {progress.total})
                  </div>
                </>
              ) : (
                <div className="art-search-loading">Starting scan...</div>
              )}
            </div>
          )}

          {state === "results" && error && (
            <div className="art-search-error">{error}</div>
          )}

          {state === "results" && !error && groups.length === 0 && (
            <div className="duplicate-no-results">
              <p>No duplicates found.</p>
              <button
                className="mogg-decrypt-start"
                onClick={() => onClose(didDelete)}
              >
                Done
              </button>
            </div>
          )}

          {state === "results" && !error && groups.length > 0 && (
            <>
              <div className="duplicate-groups">
                {groups.map((group) => (
                  <div key={group.shortname} className="duplicate-group-card">
                    <div className="duplicate-group-header">
                      {group.display_name}
                      <span className="duplicate-group-count">
                        {group.entries.length} copies
                      </span>
                    </div>
                    <div className="rename-list" style={{ maxHeight: "none", marginBottom: 0 }}>
                      {group.entries.map((entry) => (
                        <label key={entry.path} className="rename-row">
                          <input
                            type="checkbox"
                            checked={selected.has(entry.path)}
                            onChange={() => toggleSelect(entry.path, group)}
                          />
                          <div className="rename-row-info">
                            <div className="rename-current">
                              {fileName(entry.path)}
                            </div>
                            <div className="duplicate-entry-meta">
                              {formatSize(entry.file_size)}
                              {entry.description && ` \u2014 ${entry.description}`}
                            </div>
                            <div className="duplicate-instruments">
                              <span className={`inst-badge ${entry.has_drums ? "inst-active" : "inst-inactive"}`}>D</span>
                              <span className={`inst-badge ${entry.has_guitar ? "inst-active" : "inst-inactive"}`}>G</span>
                              <span className={`inst-badge ${entry.has_bass ? "inst-active" : "inst-inactive"}`}>B</span>
                              <span className={`inst-badge ${entry.has_vocals ? "inst-active" : "inst-inactive"}`}>V</span>
                              <span className={`inst-badge ${entry.has_keys ? "inst-active" : "inst-inactive"}`}>K</span>
                            </div>
                          </div>
                        </label>
                      ))}
                    </div>
                  </div>
                ))}
              </div>

              {deleteErrors.length > 0 && (
                <div className="mogg-decrypt-errors">
                  {deleteErrors.map((err, i) => (
                    <div key={i} className="mogg-error-item">
                      {err}
                    </div>
                  ))}
                </div>
              )}

              <div className="duplicate-actions">
                {confirmDelete ? (
                  <div className="score-sync-confirm">
                    <p>
                      Delete {selected.size} file
                      {selected.size !== 1 ? "s" : ""}? This cannot be undone.
                    </p>
                    <div className="score-sync-confirm-btns">
                      <button
                        className="score-sync-btn score-sync-btn-confirm"
                        onClick={handleDelete}
                        disabled={deleting}
                      >
                        {deleting ? "Deleting..." : "Confirm Delete"}
                      </button>
                      <button
                        className="score-sync-btn score-sync-btn-cancel"
                        onClick={() => setConfirmDelete(false)}
                        disabled={deleting}
                      >
                        Cancel
                      </button>
                    </div>
                  </div>
                ) : (
                  <button
                    className="duplicate-delete-btn"
                    disabled={selected.size === 0}
                    onClick={() => setConfirmDelete(true)}
                  >
                    Delete Selected ({selected.size})
                  </button>
                )}
              </div>
            </>
          )}
        </div>
      </div>
    </div>
  );
}
