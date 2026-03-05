import React, { useState, useEffect } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen, UnlistenFn } from "@tauri-apps/api/event";

interface RenameModalProps {
  paths: string[];
  onClose: (renamed: boolean) => void;
}

interface RenamePreview {
  path: string;
  current_name: string;
  new_name: string;
  status: string;
}

interface RenameResult {
  old_path: string;
  new_path: string;
  success: boolean;
  error: string;
}

interface PreviewProgress {
  current: number;
  total: number;
}

function fileName(path: string): string {
  const parts = path.replace(/\\/g, "/").split("/");
  return parts[parts.length - 1] || path;
}

function dirName(path: string): string {
  const normalized = path.replace(/\\/g, "/");
  const lastSlash = normalized.lastIndexOf("/");
  return lastSlash >= 0 ? path.substring(0, lastSlash) : ".";
}

export function RenameModal({ paths, onClose }: RenameModalProps) {
  const [state, setState] = useState<
    "ready" | "scanning" | "results" | "done"
  >("ready");
  const [progress, setProgress] = useState<PreviewProgress | null>(null);
  const [previews, setPreviews] = useState<RenamePreview[]>([]);
  const [selected, setSelected] = useState<Set<string>>(new Set());
  const [error, setError] = useState<string | null>(null);
  const [renaming, setRenaming] = useState(false);
  const [didRename, setDidRename] = useState(false);
  const [renameResults, setRenameResults] = useState<RenameResult[]>([]);

  useEffect(() => {
    let unlisten: UnlistenFn | null = null;
    listen<PreviewProgress>("rename-preview-progress", (event) => {
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
      const result = await invoke<RenamePreview[]>("preview_renames", {
        paths,
      });
      setPreviews(result);
      // Auto-select all renameable entries
      const renameable = new Set(
        result.filter((r) => r.status === "rename").map((r) => r.path)
      );
      setSelected(renameable);
      setState("results");
    } catch (e) {
      setError(String(e));
      setState("results");
    }
  };

  const toggleSelect = (path: string) => {
    const next = new Set(selected);
    if (next.has(path)) {
      next.delete(path);
    } else {
      next.add(path);
    }
    setSelected(next);
  };

  const handleRename = async () => {
    setRenaming(true);
    try {
      const renames = previews
        .filter((p) => selected.has(p.path))
        .map((p) => ({
          old_path: p.path,
          new_path: dirName(p.path) + "/" + p.new_name,
        }));
      const results = await invoke<RenameResult[]>("batch_rename", { renames });
      setRenameResults(results);
      const anySuccess = results.some((r) => r.success);
      if (anySuccess) setDidRename(true);
      setState("done");
    } catch (e) {
      setError(String(e));
    }
    setRenaming(false);
  };

  const progressPct = progress
    ? Math.round((progress.current / progress.total) * 100)
    : 0;

  const renameCount = previews.filter((p) => p.status === "rename").length;
  const skipSameCount = previews.filter(
    (p) => p.status === "skip_same"
  ).length;
  const skipNoMetaCount = previews.filter(
    (p) => p.status === "skip_no_metadata"
  ).length;

  return (
    <div className="art-search-overlay" onClick={() => onClose(didRename)}>
      <div
        className="art-search-panel rename-panel"
        onClick={(e) => e.stopPropagation()}
      >
        <div className="art-search-header">
          <h3>Batch Rename Files</h3>
          <button
            className="art-search-close"
            onClick={() => onClose(didRename)}
          >
            &times;
          </button>
        </div>

        <div className="rename-body">
          {state === "ready" && (
            <>
              <p className="mogg-decrypt-desc">
                Rename {paths.length} CON file{paths.length !== 1 ? "s" : ""}{" "}
                based on their DTA metadata. Files will be renamed to{" "}
                <strong>Artist - Title_rb3con</strong> format.
              </p>
              <button className="mogg-decrypt-start" onClick={handleScan}>
                Scan Metadata
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

          {state === "results" && error && (
            <div className="art-search-error">{error}</div>
          )}

          {state === "results" && !error && (
            <>
              {renameCount === 0 ? (
                <div className="duplicate-no-results">
                  <p>No files need renaming.</p>
                  {skipSameCount > 0 && (
                    <p className="rename-skip-detail">
                      {skipSameCount} already named correctly
                    </p>
                  )}
                  {skipNoMetaCount > 0 && (
                    <p className="rename-skip-detail">
                      {skipNoMetaCount} missing metadata
                    </p>
                  )}
                  <button
                    className="mogg-decrypt-start"
                    onClick={() => onClose(false)}
                  >
                    Done
                  </button>
                </div>
              ) : (
                <>
                  <div className="rename-list">
                    {previews.map((p) => (
                      <label
                        key={p.path}
                        className={`rename-row ${p.status !== "rename" ? "rename-row-skip" : ""}`}
                      >
                        {p.status === "rename" ? (
                          <input
                            type="checkbox"
                            checked={selected.has(p.path)}
                            onChange={() => toggleSelect(p.path)}
                          />
                        ) : (
                          <span className="rename-skip-icon" />
                        )}
                        <div className="rename-row-info">
                          <div className="rename-current">{p.current_name}</div>
                          {p.status === "rename" ? (
                            <div className="rename-arrow-row">
                              <span className="rename-arrow">&rarr;</span>
                              <span className="rename-new">{p.new_name}</span>
                            </div>
                          ) : (
                            <div className="rename-skip-reason">
                              {p.status === "skip_same"
                                ? "Already named correctly"
                                : "No metadata found"}
                            </div>
                          )}
                        </div>
                      </label>
                    ))}
                  </div>

                  <div className="duplicate-actions">
                    <button
                      className="duplicate-delete-btn"
                      disabled={selected.size === 0 || renaming}
                      onClick={handleRename}
                    >
                      {renaming
                        ? "Renaming..."
                        : `Rename Selected (${selected.size})`}
                    </button>
                  </div>
                </>
              )}
            </>
          )}

          {state === "done" && (
            <div className="rename-done">
              <div className="mogg-decrypt-summary">
                {renameResults.filter((r) => r.success).length > 0 && (
                  <div className="mogg-result-row mogg-result-success">
                    {renameResults.filter((r) => r.success).length} file
                    {renameResults.filter((r) => r.success).length !== 1
                      ? "s"
                      : ""}{" "}
                    renamed
                  </div>
                )}
                {renameResults.filter((r) => !r.success).length > 0 && (
                  <div className="mogg-result-row mogg-result-error">
                    {renameResults.filter((r) => !r.success).length} error
                    {renameResults.filter((r) => !r.success).length !== 1
                      ? "s"
                      : ""}
                  </div>
                )}
              </div>

              {renameResults.filter((r) => !r.success).length > 0 && (
                <div className="mogg-decrypt-errors">
                  {renameResults
                    .filter((r) => !r.success)
                    .map((r, i) => (
                      <div key={i} className="mogg-error-item">
                        {fileName(r.old_path)}: {r.error}
                      </div>
                    ))}
                </div>
              )}

              <button
                className="mogg-decrypt-start"
                onClick={() => onClose(didRename)}
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
