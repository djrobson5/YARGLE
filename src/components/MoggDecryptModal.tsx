import React, { useState, useEffect } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen, UnlistenFn } from "@tauri-apps/api/event";

interface MoggDecryptModalProps {
  paths: string[];
  onClose: () => void;
}

interface BatchDecryptResult {
  total: number;
  decrypted: number;
  already_decrypted: number;
  no_mogg: number;
  errors: string[];
}

interface MoggDecryptProgress {
  current: number;
  total: number;
  filename: string;
  status: string;
}

export function MoggDecryptModal({ paths, onClose }: MoggDecryptModalProps) {
  const [state, setState] = useState<"ready" | "running" | "complete">("ready");
  const [progress, setProgress] = useState<MoggDecryptProgress | null>(null);
  const [result, setResult] = useState<BatchDecryptResult | null>(null);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    let unlisten: UnlistenFn | null = null;
    listen<MoggDecryptProgress>("mogg-decrypt-progress", (event) => {
      setProgress(event.payload);
    }).then((fn) => {
      unlisten = fn;
    });
    return () => {
      if (unlisten) unlisten();
    };
  }, []);

  const handleStart = async () => {
    setState("running");
    setError(null);
    try {
      const res = await invoke<BatchDecryptResult>("batch_decrypt_moggs", { paths });
      setResult(res);
      setState("complete");
    } catch (e) {
      setError(String(e));
      setState("complete");
    }
  };

  const progressPct = progress ? Math.round((progress.current / progress.total) * 100) : 0;

  return (
    <div className="art-search-overlay" onClick={onClose}>
      <div
        className="art-search-panel mogg-decrypt-panel"
        onClick={(e) => e.stopPropagation()}
      >
        <div className="art-search-header">
          <h3>Decrypt MOGGs</h3>
          <button className="art-search-close" onClick={onClose}>
            &times;
          </button>
        </div>

        <div className="mogg-decrypt-body">
          {state === "ready" && (
            <>
              <p className="mogg-decrypt-desc">
                Decrypt encrypted MOGG audio files inside {paths.length} CON
                package{paths.length !== 1 ? "s" : ""} so YARG can play them.
                This modifies the files in-place.
              </p>
              <button className="mogg-decrypt-start" onClick={handleStart}>
                Decrypt All
              </button>
            </>
          )}

          {state === "running" && progress && (
            <div className="mogg-decrypt-progress">
              <div className="mogg-decrypt-bar-outer">
                <div
                  className="mogg-decrypt-bar-inner"
                  style={{ width: `${progressPct}%` }}
                />
              </div>
              <div className="mogg-decrypt-status">
                {progress.current} / {progress.total}
              </div>
              <div className="mogg-decrypt-filename">{progress.filename}</div>
            </div>
          )}

          {state === "running" && !progress && (
            <div className="art-search-loading">Starting...</div>
          )}

          {state === "complete" && result && (
            <div className="mogg-decrypt-results">
              <div className="mogg-decrypt-summary">
                {result.decrypted > 0 && (
                  <div className="mogg-result-row mogg-result-success">
                    {result.decrypted} decrypted
                  </div>
                )}
                {result.already_decrypted > 0 && (
                  <div className="mogg-result-row mogg-result-skip">
                    {result.already_decrypted} already decrypted
                  </div>
                )}
                {result.no_mogg > 0 && (
                  <div className="mogg-result-row mogg-result-skip">
                    {result.no_mogg} no MOGG found
                  </div>
                )}
                {result.errors.length > 0 && (
                  <div className="mogg-result-row mogg-result-error">
                    {result.errors.length} error{result.errors.length !== 1 ? "s" : ""}
                  </div>
                )}
              </div>
              {result.errors.length > 0 && (
                <div className="mogg-decrypt-errors">
                  {result.errors.map((err, i) => (
                    <div key={i} className="mogg-error-item">
                      {err}
                    </div>
                  ))}
                </div>
              )}
              <button className="mogg-decrypt-start" onClick={onClose}>
                Done
              </button>
            </div>
          )}

          {state === "complete" && error && !result && (
            <div className="art-search-error">{error}</div>
          )}
        </div>
      </div>
    </div>
  );
}
