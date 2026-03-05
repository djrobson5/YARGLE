import React, { useState, useEffect } from "react";
import { invoke } from "@tauri-apps/api/core";

interface ScoreFileInfo {
  exists: boolean;
  path: string;
  size: number;
  last_modified: string;
}

interface ScoreInfo {
  stable: ScoreFileInfo;
  nightly: ScoreFileInfo;
}

interface ScoreSyncModalProps {
  onClose: () => void;
}

function formatSize(bytes: number): string {
  if (bytes === 0) return "0 B";
  if (bytes < 1024) return `${bytes} B`;
  const kb = bytes / 1024;
  if (kb < 1024) return `${kb.toFixed(1)} KB`;
  const mb = kb / 1024;
  return `${mb.toFixed(1)} MB`;
}

export function ScoreSyncModal({ onClose }: ScoreSyncModalProps) {
  const [info, setInfo] = useState<ScoreInfo | null>(null);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);
  const [syncing, setSyncing] = useState(false);
  const [confirm, setConfirm] = useState<string | null>(null);
  const [successMsg, setSuccessMsg] = useState<string | null>(null);

  const loadInfo = async () => {
    setLoading(true);
    setError(null);
    try {
      const result = await invoke<ScoreInfo>("get_yarg_score_info");
      setInfo(result);
    } catch (e) {
      setError(String(e));
    } finally {
      setLoading(false);
    }
  };

  useEffect(() => {
    loadInfo();
  }, []);

  const handleSync = async (direction: string) => {
    if (!confirm) {
      setConfirm(direction);
      return;
    }
    setSyncing(true);
    setError(null);
    setSuccessMsg(null);
    try {
      const msg = await invoke<string>("sync_yarg_scores", { direction });
      setSuccessMsg(msg);
      setConfirm(null);
      // Refresh info after sync
      await loadInfo();
    } catch (e) {
      setError(String(e));
    } finally {
      setSyncing(false);
    }
  };

  const renderFileInfo = (label: string, file: ScoreFileInfo) => (
    <div className="score-sync-file">
      <div className="score-sync-file-label">{label}</div>
      {file.exists ? (
        <>
          <div className="score-sync-file-path" title={file.path}>{file.path}</div>
          <div className="score-sync-file-meta">
            <span>{formatSize(file.size)}</span>
            <span>{file.last_modified}</span>
          </div>
        </>
      ) : (
        <>
          <div className="score-sync-file-path score-sync-missing" title={file.path}>{file.path}</div>
          <div className="score-sync-file-meta score-sync-missing">Not found</div>
        </>
      )}
    </div>
  );

  return (
    <div className="art-search-overlay" onClick={onClose}>
      <div className="art-search-panel score-sync-panel" onClick={(e) => e.stopPropagation()}>
        <div className="art-search-header">
          <h3>Score Sync</h3>
          <button className="art-search-close" onClick={onClose}>&times;</button>
        </div>

        <div className="score-sync-body">
          {loading && <div className="art-search-loading">Loading score info...</div>}

          {error && <div className="art-search-error">{error}</div>}

          {successMsg && <div className="score-sync-success">{successMsg}</div>}

          {info && !loading && (
            <>
              <div className="score-sync-files">
                {renderFileInfo("Stable (release)", info.stable)}
                {renderFileInfo("Nightly", info.nightly)}
              </div>

              {confirm && (
                <div className="score-sync-confirm">
                  <p>
                    This will overwrite the{" "}
                    <strong>{confirm === "stable_to_nightly" ? "Nightly" : "Stable"}</strong>{" "}
                    scores with{" "}
                    <strong>{confirm === "stable_to_nightly" ? "Stable" : "Nightly"}</strong>{" "}
                    scores. A backup (.bak) will be created. Continue?
                  </p>
                  <div className="score-sync-confirm-btns">
                    <button
                      className="score-sync-btn score-sync-btn-confirm"
                      onClick={() => handleSync(confirm)}
                      disabled={syncing}
                    >
                      {syncing ? "Syncing..." : "Yes, sync"}
                    </button>
                    <button
                      className="score-sync-btn score-sync-btn-cancel"
                      onClick={() => setConfirm(null)}
                      disabled={syncing}
                    >
                      Cancel
                    </button>
                  </div>
                </div>
              )}

              {!confirm && (
                <div className="score-sync-actions">
                  <button
                    className="score-sync-btn score-sync-btn-direction"
                    onClick={() => handleSync("stable_to_nightly")}
                    disabled={!info.stable.exists || syncing}
                  >
                    Stable &rarr; Nightly
                  </button>
                  <button
                    className="score-sync-btn score-sync-btn-direction"
                    onClick={() => handleSync("nightly_to_stable")}
                    disabled={!info.nightly.exists || syncing}
                  >
                    Nightly &rarr; Stable
                  </button>
                </div>
              )}
            </>
          )}
        </div>
      </div>
    </div>
  );
}
