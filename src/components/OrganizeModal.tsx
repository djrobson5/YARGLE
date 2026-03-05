import React, { useState, useEffect } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen, UnlistenFn } from "@tauri-apps/api/event";

interface OrganizeModalProps {
  paths: string[];
  currentFolder: string;
  onClose: (organized: boolean) => void;
}

interface OrganizePreview {
  path: string;
  filename: string;
  artist: string;
  album: string;
  target_folder: string;
  target_path: string;
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

export function OrganizeModal({
  paths,
  currentFolder,
  onClose,
}: OrganizeModalProps) {
  const [state, setState] = useState<
    "ready" | "scanning" | "results" | "done"
  >("ready");
  const [progress, setProgress] = useState<PreviewProgress | null>(null);
  const [previews, setPreviews] = useState<OrganizePreview[]>([]);
  const [selected, setSelected] = useState<Set<string>>(new Set());
  const [error, setError] = useState<string | null>(null);
  const [organizing, setOrganizing] = useState(false);
  const [didOrganize, setDidOrganize] = useState(false);
  const [organizeResults, setOrganizeResults] = useState<RenameResult[]>([]);

  useEffect(() => {
    let unlisten: UnlistenFn | null = null;
    listen<PreviewProgress>("organize-preview-progress", (event) => {
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
      const result = await invoke<OrganizePreview[]>("preview_organize", {
        paths,
      });
      setPreviews(result);
      const moveable = new Set(
        result.filter((r) => r.status === "move").map((r) => r.path)
      );
      setSelected(moveable);
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

  const handleOrganize = async () => {
    setOrganizing(true);
    try {
      const requests = previews
        .filter((p) => selected.has(p.path))
        .map((p) => ({
          old_path: p.path,
          new_path: p.target_path,
        }));
      const results = await invoke<RenameResult[]>("execute_organize", {
        requests,
      });
      setOrganizeResults(results);
      const anySuccess = results.some((r) => r.success);
      if (anySuccess) setDidOrganize(true);
      setState("done");
    } catch (e) {
      setError(String(e));
    }
    setOrganizing(false);
  };

  const progressPct = progress
    ? Math.round((progress.current / progress.total) * 100)
    : 0;

  const moveCount = previews.filter((p) => p.status === "move").length;
  const skipSameCount = previews.filter(
    (p) => p.status === "skip_same"
  ).length;
  const skipNoMetaCount = previews.filter(
    (p) => p.status === "skip_no_metadata"
  ).length;

  // Group moveable previews by target_folder
  const moveablePreviews = previews.filter((p) => p.status === "move");
  const folderGroups: Map<string, OrganizePreview[]> = new Map();
  for (const p of moveablePreviews) {
    const group = folderGroups.get(p.target_folder) || [];
    group.push(p);
    folderGroups.set(p.target_folder, group);
  }
  const sortedFolders = Array.from(folderGroups.keys()).sort();

  const skippedPreviews = previews.filter((p) => p.status !== "move");

  return (
    <div className="art-search-overlay" onClick={() => onClose(didOrganize)}>
      <div
        className="art-search-panel organize-panel"
        onClick={(e) => e.stopPropagation()}
      >
        <div className="art-search-header">
          <h3>Auto-Organize Files</h3>
          <button
            className="art-search-close"
            onClick={() => onClose(didOrganize)}
          >
            &times;
          </button>
        </div>

        <div className="rename-body">
          {state === "ready" && (
            <>
              <p className="mogg-decrypt-desc">
                Sort {paths.length} CON file{paths.length !== 1 ? "s" : ""} into{" "}
                <strong>Artist / Album</strong> subfolders based on DTA
                metadata.
              </p>
              <div className="organize-base-path">
                Base folder: <code>{currentFolder}</code>
              </div>
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
              {moveCount === 0 ? (
                <div className="duplicate-no-results">
                  <p>No files need organizing.</p>
                  {skipSameCount > 0 && (
                    <p className="rename-skip-detail">
                      {skipSameCount} already in correct location
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
                    {sortedFolders.map((folder) => {
                      const items = folderGroups.get(folder)!;
                      const allSelected = items.every((item) =>
                        selected.has(item.path)
                      );
                      const toggleGroup = () => {
                        const next = new Set(selected);
                        if (allSelected) {
                          items.forEach((item) => next.delete(item.path));
                        } else {
                          items.forEach((item) => next.add(item.path));
                        }
                        setSelected(next);
                      };

                      return (
                        <div key={folder} className="organize-folder-group">
                          <label
                            className="organize-folder-header"
                            onClick={toggleGroup}
                          >
                            <input
                              type="checkbox"
                              checked={allSelected}
                              onChange={toggleGroup}
                            />
                            <span className="organize-folder-path">
                              {folder}
                            </span>
                            <span className="organize-folder-count">
                              {items.length} file
                              {items.length !== 1 ? "s" : ""}
                            </span>
                          </label>
                          {items.map((p) => (
                            <label
                              key={p.path}
                              className="rename-row"
                              style={{ marginLeft: 20 }}
                            >
                              <input
                                type="checkbox"
                                checked={selected.has(p.path)}
                                onChange={() => toggleSelect(p.path)}
                              />
                              <div className="rename-row-info">
                                <div className="rename-current">
                                  {p.filename}
                                </div>
                              </div>
                            </label>
                          ))}
                        </div>
                      );
                    })}

                    {skippedPreviews.length > 0 && (
                      <div className="organize-folder-group">
                        <div className="organize-folder-header organize-folder-header-skip">
                          <span className="organize-folder-path">Skipped</span>
                          <span className="organize-folder-count">
                            {skippedPreviews.length} file
                            {skippedPreviews.length !== 1 ? "s" : ""}
                          </span>
                        </div>
                        {skippedPreviews.map((p) => (
                          <div
                            key={p.path}
                            className="rename-row rename-row-skip"
                            style={{ marginLeft: 20 }}
                          >
                            <span className="rename-skip-icon" />
                            <div className="rename-row-info">
                              <div className="rename-current">{p.filename}</div>
                              <div className="rename-skip-reason">
                                {p.status === "skip_same"
                                  ? "Already in correct location"
                                  : "No metadata found"}
                              </div>
                            </div>
                          </div>
                        ))}
                      </div>
                    )}
                  </div>

                  <div className="duplicate-actions">
                    <button
                      className="duplicate-delete-btn"
                      disabled={selected.size === 0 || organizing}
                      onClick={handleOrganize}
                    >
                      {organizing
                        ? "Organizing..."
                        : `Organize Selected (${selected.size})`}
                    </button>
                  </div>
                </>
              )}
            </>
          )}

          {state === "done" && (
            <div className="rename-done">
              <div className="mogg-decrypt-summary">
                {organizeResults.filter((r) => r.success).length > 0 && (
                  <div className="mogg-result-row mogg-result-success">
                    {organizeResults.filter((r) => r.success).length} file
                    {organizeResults.filter((r) => r.success).length !== 1
                      ? "s"
                      : ""}{" "}
                    moved
                  </div>
                )}
                {organizeResults.filter((r) => !r.success).length > 0 && (
                  <div className="mogg-result-row mogg-result-error">
                    {organizeResults.filter((r) => !r.success).length} error
                    {organizeResults.filter((r) => !r.success).length !== 1
                      ? "s"
                      : ""}
                  </div>
                )}
              </div>

              {organizeResults.filter((r) => !r.success).length > 0 && (
                <div className="mogg-decrypt-errors">
                  {organizeResults
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
                onClick={() => onClose(didOrganize)}
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
