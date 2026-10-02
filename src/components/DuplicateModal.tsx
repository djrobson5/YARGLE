import React, { useState, useEffect, useMemo } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen, UnlistenFn } from "@tauri-apps/api/event";
import type { DeleteResult, SongSummary } from "../types";
import { ChevronDown, ChevronUp, X } from "lucide-react";
import { InstrumentIcon } from "./YargIcon";

interface DuplicateModalProps {
  songs: SongSummary[];
  /** Library root, used to offer "only these folders" scopes. */
  rootFolder: string | null;
  onClose: (deleted: boolean) => void;
}

interface Extras {
  album_art: boolean;
  background: boolean;
  highway: boolean;
  video: boolean;
  stems: string[];
  audio_channels: number | null;
  venue: boolean;
}

interface DuplicateEntry {
  path: string;
  display_name: string;
  description: string;
  is_folder: boolean;
  file_size: number;
  chart_hash: string | null;
  chart_kind: string;
  // Copies in a group with the same match_id have byte-identical charts.
  match_id: number | null;
  has_drums: boolean;
  has_guitar: boolean;
  has_bass: boolean;
  has_vocals: boolean;
  has_keys: boolean;
  extras: Extras;
}

interface DuplicateGroup {
  key: string;
  display_name: string;
  kind: "identical" | "versions";
  entries: DuplicateEntry[];
}

interface ScanProgress {
  current: number;
  total: number;
  phase: string;
}

type Tab = "identical" | "versions";

// Rendering thousands of groups at once is slow; reveal them in pages.
const PAGE = 150;

function formatSize(bytes: number): string {
  if (bytes < 1024) return `${bytes} B`;
  if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(1)} KB`;
  return `${(bytes / (1024 * 1024)).toFixed(1)} MB`;
}

function splitPath(path: string): string[] {
  return path.replace(/\\/g, "/").split("/").filter(Boolean);
}

function fileName(path: string): string {
  const parts = splitPath(path);
  return parts[parts.length - 1] || path;
}

function isUnder(parts: string[], root: string[]): boolean {
  return root.every((seg, i) => parts[i]?.toLowerCase() === seg.toLowerCase());
}

/** Where a copy lives, relative to the library root. */
function parentLabel(path: string, root: string | null): string {
  const parts = splitPath(path).slice(0, -1);
  if (root) {
    const r = splitPath(root);
    if (isUnder(parts, r)) return parts.slice(r.length).join(" / ") || "(library root)";
  }
  return parts.join(" / ");
}

/** First folder under the root, used for the scope picker. */
function topFolder(path: string, root: string): string | null {
  const parts = splitPath(path);
  const r = splitPath(root);
  if (!isUnder(parts, r)) return null;
  return parts.length > r.length + 1 ? parts[r.length] : null;
}

/** How much a copy carries beyond the chart; the best-scoring copy is kept. */
function extrasScore(e: DuplicateEntry): number {
  const x = e.extras;
  return (
    (x.album_art ? 1 : 0) +
    (x.background ? 1 : 0) +
    (x.highway ? 1 : 0) +
    (x.video ? 2 : 0) +
    (x.venue ? 1 : 0) +
    x.stems.length * 0.5 +
    (x.audio_channels ?? 0) * 0.1
  );
}

function bestCopy(entries: DuplicateEntry[]): DuplicateEntry {
  return [...entries].sort(
    (a, b) =>
      extrasScore(b) - extrasScore(a) ||
      b.file_size - a.file_size ||
      a.path.length - b.path.length
  )[0];
}

const CHART_LETTERS = "ABCDEFGHIJKLMNOPQRSTUVWXYZ";

function ExtrasChips({ e }: { e: DuplicateEntry }) {
  const x = e.extras;
  const chips: { label: string; title: string }[] = [];
  if (x.album_art) chips.push({ label: "Art", title: "Album art" });
  if (x.background) chips.push({ label: "Background", title: "Custom background image" });
  if (x.highway) chips.push({ label: "Highway", title: "Custom highway image" });
  if (x.video) chips.push({ label: "Video", title: "Background video" });
  if (x.stems.length > 0)
    chips.push({
      label: `${x.stems.length} track${x.stems.length !== 1 ? "s" : ""}`,
      title: `Separate instrument audio: ${x.stems.join(", ")}`,
    });
  if (x.audio_channels)
    chips.push({ label: `${x.audio_channels}-ch audio`, title: "Audio channels in the multitrack .mogg" });
  if (x.venue) chips.push({ label: "Venue", title: "Has lipsync / venue data (.milo)" });
  if (chips.length === 0) return null;
  return (
    <div className="dup-chips">
      {chips.map((c) => (
        <span key={c.label} className="dup-chip" title={c.title}>
          {c.label}
        </span>
      ))}
    </div>
  );
}

export function DuplicateModal({ songs, rootFolder, onClose }: DuplicateModalProps) {
  const [state, setState] = useState<"ready" | "scanning" | "results">("ready");
  const [progress, setProgress] = useState<ScanProgress | null>(null);
  const [groups, setGroups] = useState<DuplicateGroup[]>([]);
  const [selected, setSelected] = useState<Set<string>>(new Set());
  const [error, setError] = useState<string | null>(null);
  const [deleting, setDeleting] = useState(false);
  const [confirmDelete, setConfirmDelete] = useState(false);
  const [didDelete, setDidDelete] = useState(false);
  const [deleteErrors, setDeleteErrors] = useState<string[]>([]);
  // Paths the Recycle Bin refused (drive has no bin), awaiting a permanent-delete confirm.
  const [pendingPermanent, setPendingPermanent] = useState<string[]>([]);
  const [tab, setTab] = useState<Tab>("identical");
  const [shown, setShown] = useState(PAGE);
  // Scope: empty = whole library; otherwise only songs under these top folders.
  const [scope, setScope] = useState<Set<string>>(new Set());
  const [scopeOpen, setScopeOpen] = useState(false);

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

  const folders = useMemo(() => {
    if (!rootFolder) return [];
    const counts = new Map<string, number>();
    for (const s of songs) {
      const f = topFolder(s.path, rootFolder);
      if (f) counts.set(f, (counts.get(f) ?? 0) + 1);
    }
    return [...counts.entries()].sort((a, b) => a[0].localeCompare(b[0]));
  }, [songs, rootFolder]);

  const scopedSongs = useMemo(() => {
    if (scope.size === 0 || !rootFolder) return songs;
    return songs.filter((s) => {
      const f = topFolder(s.path, rootFolder);
      return f !== null && scope.has(f);
    });
  }, [songs, scope, rootFolder]);

  const identical = useMemo(() => groups.filter((g) => g.kind === "identical"), [groups]);
  const versions = useMemo(() => groups.filter((g) => g.kind === "versions"), [groups]);
  const visible = tab === "identical" ? identical : versions;

  const handleScan = async () => {
    setState("scanning");
    setError(null);
    try {
      const input = scopedSongs.map((s) => ({
        path: s.path,
        artist: s.artist,
        song_title: s.song_title,
        display_name: s.display_name,
        description: s.description,
      }));
      const result = await invoke<DuplicateGroup[]>("find_duplicates", { songs: input });
      setGroups(result);
      setTab(result.some((g) => g.kind === "identical") ? "identical" : "versions");
      setShown(PAGE);
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
      // Never allow selecting every copy in a group.
      const wouldBeSelected = group.entries.filter((e) => e.path === path || next.has(e.path));
      if (wouldBeSelected.length >= group.entries.length) return;
      next.add(path);
    }
    setSelected(next);
  };

  // In every identical group, keep the copy with the most extras and select the rest.
  const selectIdenticalExtras = () => {
    const next = new Set(selected);
    for (const g of identical) {
      const keep = bestCopy(g.entries);
      for (const e of g.entries) {
        if (e.path === keep.path) next.delete(e.path);
        else next.add(e.path);
      }
    }
    setSelected(next);
  };

  const handleDelete = async (permanent = false) => {
    setDeleting(true);
    setConfirmDelete(false);
    setPendingPermanent([]);
    try {
      const toDelete = permanent ? pendingPermanent : Array.from(selected);
      const result = await invoke<DeleteResult>("delete_files", { paths: toDelete, permanent });
      setDeleteErrors(result.failures);

      // Remove deleted entries from groups
      const kept = new Set(result.not_trashable);
      const deletedSet = new Set(
        toDelete.filter((p) => !kept.has(p) && !result.failures.some((f) => f.startsWith(p)))
      );
      if (deletedSet.size > 0) setDidDelete(true);

      const updated = groups
        .map((g) => ({
          ...g,
          entries: g.entries.filter((e) => !deletedSet.has(e.path)),
        }))
        .filter((g) => g.entries.length > 1);

      setGroups(updated);
      // Whatever the Recycle Bin refused stays selected for the permanent confirm.
      setSelected(new Set(result.not_trashable));
      setPendingPermanent(result.not_trashable);
    } catch (e) {
      setDeleteErrors([String(e)]);
    }
    setDeleting(false);
  };

  const progressPct =
    progress && progress.total > 0 ? Math.round((progress.current / progress.total) * 100) : 0;

  const renderEntry = (group: DuplicateGroup, entry: DuplicateEntry) => {
    const chartLabel =
      group.kind === "versions"
        ? entry.match_id === null
          ? "No chart"
          : `Chart ${CHART_LETTERS[entry.match_id] ?? entry.match_id + 1}`
        : null;
    return (
      <label key={entry.path} className="rename-row">
        <input
          type="checkbox"
          checked={selected.has(entry.path)}
          onChange={() => toggleSelect(entry.path, group)}
        />
        <div className="rename-row-info">
          <div className="dup-entry-title">
            <span className="rename-current">{fileName(entry.path)}</span>
            <button
              className="dup-reveal-btn"
              title="Show in Explorer"
              onClick={(ev) => {
                ev.preventDefault();
                invoke("reveal_in_explorer", { path: entry.path }).catch(() => {});
              }}
            >
              Show
            </button>
          </div>
          <div className="duplicate-entry-meta" title={entry.path}>
            {parentLabel(entry.path, rootFolder)}
          </div>
          <div className="duplicate-entry-meta">
            <span className={`dup-format ${entry.is_folder ? "dup-format-folder" : "dup-format-con"}`}>
              {entry.is_folder ? "Folder" : "CON"}
            </span>
            {chartLabel && (
              <span
                className={`dup-chart-tag${entry.match_id === null ? " dup-chart-none" : ""}`}
                title="Copies with the same letter have identical charts"
              >
                {chartLabel}
              </span>
            )}
            {formatSize(entry.file_size)}
            {entry.chart_kind && entry.chart_kind !== "mid" && ` · ${entry.chart_kind}`}
            {entry.description && ` · ${entry.description}`}
          </div>
          <div className="duplicate-instruments">
            <InstrumentIcon instrument="drums" size={20} off={!entry.has_drums} title={entry.has_drums ? "Drums" : "No drums"} />
            <InstrumentIcon instrument="guitar" size={20} off={!entry.has_guitar} title={entry.has_guitar ? "Guitar" : "No guitar"} />
            <InstrumentIcon instrument="bass" size={20} off={!entry.has_bass} title={entry.has_bass ? "Bass" : "No bass"} />
            <InstrumentIcon instrument="vocals" size={20} off={!entry.has_vocals} title={entry.has_vocals ? "Vocals" : "No vocals"} />
            <InstrumentIcon instrument="keys" size={20} off={!entry.has_keys} title={entry.has_keys ? "Keys" : "No keys"} />
          </div>
          <ExtrasChips e={entry} />
        </div>
      </label>
    );
  };

  return (
    <div className="art-search-overlay" onClick={() => onClose(didDelete)}>
      <div
        className="art-search-panel duplicate-panel"
        onClick={(e) => e.stopPropagation()}
      >
        <div className="art-search-header">
          <h3>Find Duplicates</h3>
          <button
            className="art-search-close" aria-label="Close"
            onClick={() => onClose(didDelete)}
          >
            <X size={18} />
          </button>
        </div>

        <div className="duplicate-body">
          {state === "ready" && (
            <>
              <p className="mogg-decrypt-desc">
                Finds songs that appear more than once (same artist and title), then compares
                their charts. <b>Identical</b> copies have the same chart, so the extras can go.{" "}
                <b>Different versions</b> are other charters or revisions: compare before removing.
              </p>
              {folders.length > 1 && (
                <div className="dup-scope">
                  <button className="dup-scope-toggle" onClick={() => setScopeOpen((o) => !o)}>
                    {scope.size === 0
                      ? "Searching the whole library"
                      : `Searching ${scope.size} folder${scope.size !== 1 ? "s" : ""}`}{" "}
                    {scopeOpen ? <ChevronUp size={14} /> : <ChevronDown size={14} />}
                  </button>
                  {scopeOpen && (
                    <div className="dup-scope-list">
                      {folders.map(([name, count]) => (
                        <label key={name} className="dup-scope-row">
                          <input
                            type="checkbox"
                            checked={scope.has(name)}
                            onChange={() => {
                              const next = new Set(scope);
                              if (next.has(name)) next.delete(name);
                              else next.add(name);
                              setScope(next);
                            }}
                          />
                          <span>{name}</span>
                          <span className="dup-scope-count">{count}</span>
                        </label>
                      ))}
                      {scope.size > 0 && (
                        <button className="dup-scope-clear" onClick={() => setScope(new Set())}>
                          Search the whole library instead
                        </button>
                      )}
                    </div>
                  )}
                </div>
              )}
              <div className="dialog-footer">
                <button
                  className="mogg-decrypt-start"
                  onClick={handleScan}
                  disabled={scopedSongs.length < 2}
                >
                  Scan {scopedSongs.length} song{scopedSongs.length !== 1 ? "s" : ""}
                </button>
              </div>
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
                <div className="art-search-loading">Grouping songs by name…</div>
              )}
            </div>
          )}

          {state === "results" && error && (
            <div className="art-search-error">{error}</div>
          )}

          {state === "results" && !error && groups.length === 0 && (
            <div className="duplicate-no-results">
              <p>No duplicates found.</p>
              <div className="dialog-footer">
                <button
                  className="mogg-decrypt-start"
                  onClick={() => onClose(didDelete)}
                >
                  Done
                </button>
              </div>
            </div>
          )}

          {state === "results" && !error && groups.length > 0 && (
            <>
              <div className="dup-tabs">
                <button
                  className={`dup-tab${tab === "identical" ? " dup-tab-active" : ""}`}
                  onClick={() => { setTab("identical"); setShown(PAGE); }}
                >
                  Identical ({identical.length})
                </button>
                <button
                  className={`dup-tab${tab === "versions" ? " dup-tab-active" : ""}`}
                  onClick={() => { setTab("versions"); setShown(PAGE); }}
                >
                  Different versions ({versions.length})
                </button>
                {tab === "identical" && identical.length > 0 && (
                  <button
                    className="dup-select-extras"
                    onClick={selectIdenticalExtras}
                    title="In every identical group, keep the copy with the most extras (art, video, separate tracks…) and select the rest"
                  >
                    Select extra copies
                  </button>
                )}
              </div>
              <p className="dup-tab-hint">
                {tab === "identical"
                  ? "Same chart in every copy. Keeping one is enough."
                  : "Same song, different charts. Copies with the same letter match each other."}
              </p>

              <div className="duplicate-groups">
                {visible.length === 0 && (
                  <div className="duplicate-no-results"><p>Nothing here.</p></div>
                )}
                {visible.slice(0, shown).map((group) => (
                  <div key={group.key} className="duplicate-group-card">
                    <div className="duplicate-group-header">
                      {group.display_name}
                      <span className="duplicate-group-count">
                        {group.entries.length} copies
                      </span>
                    </div>
                    <div className="rename-list" style={{ maxHeight: "none", marginBottom: 0 }}>
                      {group.entries.map((entry) => renderEntry(group, entry))}
                    </div>
                  </div>
                ))}
                {visible.length > shown && (
                  <button className="dup-show-more" onClick={() => setShown((n) => n + PAGE)}>
                    Show more ({visible.length - shown} left)
                  </button>
                )}
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

              <div className="duplicate-actions dialog-footer">
                {pendingPermanent.length > 0 ? (
                  <div className="score-sync-confirm">
                    <p>
                      {pendingPermanent.length} file
                      {pendingPermanent.length !== 1 ? "s are" : " is"} on a drive with no
                      Recycle Bin. Delete permanently? This cannot be undone.
                    </p>
                    <div className="score-sync-confirm-btns">
                      <button
                        className="score-sync-btn score-sync-btn-confirm"
                        onClick={() => handleDelete(true)}
                        disabled={deleting}
                      >
                        {deleting ? "Deleting..." : "Delete Permanently"}
                      </button>
                      <button
                        className="score-sync-btn score-sync-btn-cancel"
                        onClick={() => {
                          setPendingPermanent([]);
                          setSelected(new Set());
                        }}
                        disabled={deleting}
                      >
                        Keep
                      </button>
                    </div>
                  </div>
                ) : confirmDelete ? (
                  <div className="score-sync-confirm">
                    <p>
                      Move {selected.size} cop{selected.size !== 1 ? "ies" : "y"} to the Recycle Bin?
                    </p>
                    <div className="score-sync-confirm-btns">
                      <button
                        className="score-sync-btn score-sync-btn-confirm"
                        onClick={() => handleDelete()}
                        disabled={deleting}
                      >
                        {deleting ? "Deleting..." : "Move to Recycle Bin"}
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
