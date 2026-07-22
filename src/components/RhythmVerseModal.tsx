import React, { useState, useEffect, useMemo, useCallback } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { open } from "@tauri-apps/plugin-dialog";
import type { RvBrowseResult, RvDownloadRecord, RvSongFile, SongSummary } from "../types";

interface RhythmVerseModalProps {
  /** Currently-loaded library songs, used to flag "already in library". */
  librarySongs: SongSummary[];
  /** Folder new downloads extract into; if null the user is prompted. */
  libraryFolder: string | null;
  /** Called after a successful download so the caller can refresh the list. */
  onLibraryChanged?: () => void;
  onClose: () => void;
}

interface DownloadProgress {
  file_id: string;
  phase: "starting" | "downloading" | "extracting" | "saving" | "done" | "error";
  received: number;
  total: number;
  message: string;
}

const RECORDS_PER_PAGE = 25;

const SORT_OPTIONS = [
  { value: "update_date", label: "Last updated" },
  { value: "downloads", label: "Most downloaded" },
  { value: "title", label: "Title" },
  { value: "artist", label: "Artist" },
] as const;

// Core band instruments, in the usual display order.
const INSTRUMENTS = [
  { key: "diff_guitar", label: "G", name: "Guitar" },
  { key: "diff_bass", label: "B", name: "Bass" },
  { key: "diff_drums", label: "D", name: "Drums" },
  { key: "diff_vocals", label: "V", name: "Vocals" },
  { key: "diff_keys", label: "K", name: "Keys" },
] as const;

// A tier >= 1 means the instrument is charted; 0 / -1 / null means absent.
function isCharted(tier: number | null): boolean {
  return tier != null && tier >= 1;
}

function norm(s: string): string {
  return (s || "").toLowerCase().replace(/[^a-z0-9]/g, "");
}

function formatBytes(n: number | null): string {
  if (!n || n <= 0) return "";
  if (n < 1024 * 1024) return `${Math.max(1, Math.round(n / 1024))} KB`;
  return `${(n / (1024 * 1024)).toFixed(1)} MB`;
}

function formatCount(n: number | null): string {
  if (n == null) return "0";
  if (n >= 1000) return `${(n / 1000).toFixed(1)}k`;
  return String(n);
}

function formatDuration(sec: number | null): string {
  if (!sec || sec <= 0) return "";
  const m = Math.floor(sec / 60);
  const s = Math.floor(sec % 60);
  return `${m}:${s.toString().padStart(2, "0")}`;
}

// Friendly name for an off-site download host.
function externalHostName(url: string): string {
  try {
    const h = new URL(url).hostname.replace(/^www\./, "");
    if (h.includes("drive.google")) return "Google Drive";
    if (h.includes("mediafire")) return "Mediafire";
    if (h.includes("dropbox")) return "Dropbox";
    if (h.includes("mega.")) return "MEGA";
    if (h.includes("ko-fi")) return "Ko-fi";
    if (h.includes("youtu")) return "YouTube";
    return h;
  } catch {
    return "external site";
  }
}

// RhythmVerse timestamps ("YYYY-MM-DD HH:MM:SS") are UTC (verified against
// live upload times). Returns ms since epoch, or null for unset/zero dates.
function parseRvDate(s: string): number | null {
  if (!s || s.startsWith("0000")) return null;
  const t = Date.parse(s.replace(" ", "T") + "Z");
  return isNaN(t) ? null : t;
}

// "2026-07-22 00:19:09" -> "Jul 22, 2026" (empty for unset/zero dates).
function formatDate(s: string): string {
  if (!s || s.startsWith("0000")) return "";
  const d = new Date(s.replace(" ", "T"));
  if (isNaN(d.getTime())) return s.slice(0, 10);
  return d.toLocaleDateString(undefined, {
    year: "numeric",
    month: "short",
    day: "numeric",
  });
}

export function RhythmVerseModal({
  librarySongs,
  libraryFolder,
  onLibraryChanged,
  onClose,
}: RhythmVerseModalProps) {
  const [text, setText] = useState(""); // input box
  const [query, setQuery] = useState(""); // submitted search
  const [sortBy, setSortBy] = useState<string>("update_date");
  const [sortOrder, setSortOrder] = useState<"DESC" | "ASC">("DESC");
  const [page, setPage] = useState(1);

  const [result, setResult] = useState<RvBrowseResult | null>(null);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);

  // file_id -> downloaded_at for files fetched via YARGLE. Gives the exact
  // "in library" match plus update detection against the site's upload date.
  const [downloads, setDownloads] = useState<Map<string, string>>(new Map());
  // file_ids whose off-site link the user has opened in the browser
  const [openedIds, setOpenedIds] = useState<Set<string>>(new Set());
  // the single in-flight download, if any (downloads run one at a time)
  const [activeDownload, setActiveDownload] = useState<{
    fileId: string;
    phase: string;
    pct: number;
  } | null>(null);

  const refreshDownloads = useCallback(() => {
    invoke<RvDownloadRecord[]>("rv_download_records")
      .then((recs) =>
        setDownloads(new Map(recs.map((r) => [r.file_id, r.downloaded_at])))
      )
      .catch(() => {});
  }, []);

  useEffect(() => {
    refreshDownloads();
    invoke<string[]>("rv_opened_ids")
      .then((ids) => setOpenedIds(new Set(ids)))
      .catch(() => {});
  }, [refreshDownloads]);

  // Track progress of the active download.
  useEffect(() => {
    const un = listen<DownloadProgress>("rv-download-progress", (e) => {
      const p = e.payload;
      setActiveDownload((cur) => {
        if (!cur || cur.fileId !== p.file_id) return cur;
        const pct =
          p.phase === "extracting" || p.phase === "done"
            ? 100
            : p.total > 0
            ? Math.round((p.received / p.total) * 100)
            : cur.pct;
        return { fileId: p.file_id, phase: p.phase, pct };
      });
    });
    return () => {
      un.then((fn) => fn());
    };
  }, []);

  const handleOpenExternal = useCallback((song: RvSongFile) => {
    invoke("rv_open_external", { url: song.external_url })
      .then(() => {
        setOpenedIds((prev) => new Set(prev).add(song.file_id));
        invoke("rv_mark_opened", {
          fileId: song.file_id,
          artist: song.artist,
          title: song.title,
          externalUrl: song.external_url,
        }).catch(() => {});
      })
      .catch((e) => setError(String(e)));
  }, []);

  const handleDownload = useCallback(
    async (song: RvSongFile) => {
      if (activeDownload) return; // one at a time
      let dest = libraryFolder;
      if (!dest) {
        const picked = await open({ directory: true, multiple: false });
        if (!picked) return;
        dest = picked as string;
      }
      setError(null);
      setActiveDownload({ fileId: song.file_id, phase: "starting", pct: 0 });
      try {
        await invoke("rv_download", {
          fileId: song.file_id,
          destFolder: dest,
          songId: song.song_id,
          artist: song.artist,
          title: song.title,
          fileName: song.file_name,
        });
        refreshDownloads();
        onLibraryChanged?.();
      } catch (e) {
        setError(String(e));
      } finally {
        setActiveDownload(null);
      }
    },
    [activeDownload, libraryFolder, onLibraryChanged, refreshDownloads]
  );

  // Build a normalized artist+title index of the local library. Deliberately
  // NO title-only matching: many distinct songs share a title ("My Way"), and
  // a false "In library" hides the Download button on a song the user wants.
  const libIndex = useMemo(() => {
    const artistTitle = new Set<string>();
    for (const s of librarySongs) {
      const dn = s.display_name || "";
      // Display names are conventionally "Artist - Title"; norm() strips the
      // separator, so this compares against norm(artist + title) directly.
      if (dn) artistTitle.add(norm(dn));
      // Also index the artist prefix + the canonical title field, in case the
      // display name's title spelling differs from the actual song title.
      const idx = dn.indexOf(" - ");
      if (idx > 0 && s.title_name) {
        artistTitle.add(norm(dn.slice(0, idx) + s.title_name));
      }
    }
    return { artistTitle };
  }, [librarySongs]);

  const inLibrary = useCallback(
    (song: RvSongFile): boolean => {
      if (downloads.has(song.file_id)) return true; // exact: downloaded via YARGLE
      if (song.artist && song.title) {
        return libIndex.artistTitle.has(norm(song.artist + song.title));
      }
      return false;
    },
    [libIndex, downloads]
  );

  // True when we downloaded this exact file and the site's upload date is
  // newer than our download (1 min slack against clock skew). Only knowable
  // for files fetched via YARGLE — heuristic matches carry no file linkage.
  const needsUpdate = useCallback(
    (song: RvSongFile): boolean => {
      const dl = downloads.get(song.file_id);
      if (!dl) return false;
      const uploaded = parseRvDate(song.uploaded);
      const downloaded = Date.parse(dl);
      if (uploaded == null || isNaN(downloaded)) return false;
      return uploaded > downloaded + 60_000;
    },
    [downloads]
  );

  useEffect(() => {
    let cancelled = false;
    setLoading(true);
    setError(null);
    invoke<RvBrowseResult>("rv_browse", {
      game: "yarg",
      text: query,
      page,
      records: RECORDS_PER_PAGE,
      sortBy,
      sortOrder,
    })
      .then((res) => {
        if (!cancelled) setResult(res);
      })
      .catch((e) => {
        if (!cancelled) {
          setError(String(e));
          setResult(null);
        }
      })
      .finally(() => {
        if (!cancelled) setLoading(false);
      });
    return () => {
      cancelled = true;
    };
  }, [query, sortBy, sortOrder, page]);

  const submitSearch = () => {
    setPage(1);
    setQuery(text.trim());
  };

  const totalPages = result
    ? Math.max(1, Math.ceil(result.total_filtered / RECORDS_PER_PAGE))
    : 1;

  return (
    <div className="art-search-overlay" onClick={onClose}>
      <div className="art-search-panel rv-panel" onClick={(e) => e.stopPropagation()}>
        <div className="art-search-header">
          <h3>Browse RhythmVerse</h3>
          <button className="art-search-close" onClick={onClose}>
            &times;
          </button>
        </div>

        <div className="rv-controls">
          <input
            type="text"
            className="rv-search-input"
            placeholder="Search songs, artists…"
            value={text}
            onChange={(e) => setText(e.target.value)}
            onKeyDown={(e) => {
              if (e.key === "Enter") submitSearch();
            }}
            autoFocus
          />
          <button className="rv-search-btn" onClick={submitSearch}>
            Search
          </button>
          <select
            className="rv-sort-select"
            value={sortBy}
            onChange={(e) => {
              setSortBy(e.target.value);
              setPage(1);
            }}
          >
            {SORT_OPTIONS.map((o) => (
              <option key={o.value} value={o.value}>
                {o.label}
              </option>
            ))}
          </select>
          <button
            className="rv-order-btn"
            title={sortOrder === "DESC" ? "Descending" : "Ascending"}
            onClick={() => {
              setSortOrder((o) => (o === "DESC" ? "ASC" : "DESC"));
              setPage(1);
            }}
          >
            {sortOrder === "DESC" ? "↓" : "↑"}
          </button>
        </div>

        <div className="rv-status-bar">
          {result && !error && (
            <span>
              {result.total_filtered.toLocaleString()}{" "}
              {query.trim().length >= 3
                ? `result${result.total_filtered !== 1 ? "s" : ""} for "${query.trim()}"`
                : "songs"}
            </span>
          )}
          {loading && <span className="rv-status-loading">Loading…</span>}
        </div>

        <div className="rv-results">
          {error && <div className="art-search-error">{error}</div>}

          {!error && result && result.songs.length === 0 && !loading && (
            <div className="duplicate-no-results">
              <p>No matches found{query ? ` for "${query}"` : ""}.</p>
            </div>
          )}

          {!error &&
            result &&
            result.songs.map((song) => {
              const have = inLibrary(song);
              const hasUpdate = needsUpdate(song);
              const isActive = activeDownload?.fileId === song.file_id;
              const meta = [song.album, song.year ? String(song.year) : "", song.genre]
                .filter(Boolean)
                .join(" · ");
              const byline = [
                formatDate(song.uploaded),
                song.uploader ? `by ${song.uploader}` : "",
              ]
                .filter(Boolean)
                .join(" · ");
              const isExternal = !!song.external_url;
              const extHost = isExternal ? externalHostName(song.external_url) : "";
              return (
                <div key={song.file_id} className="rv-row">
                  <div className="rv-thumb">
                    {song.album_art_url ? (
                      <img
                        src={song.album_art_url}
                        alt=""
                        loading="lazy"
                        onError={(e) => {
                          (e.target as HTMLImageElement).style.visibility = "hidden";
                        }}
                      />
                    ) : (
                      <div className="rv-thumb-placeholder">♪</div>
                    )}
                  </div>
                  <div className="rv-info">
                    <div className="rv-title">{song.title || song.file_name}</div>
                    <div className="rv-artist">{song.artist || "Unknown artist"}</div>
                    {meta && <div className="rv-meta">{meta}</div>}
                    {byline && <div className="rv-byline">{byline}</div>}
                    <div className="rv-instruments">
                      {INSTRUMENTS.map((inst) => {
                        const tier = song[inst.key];
                        const on = isCharted(tier);
                        return (
                          <span
                            key={inst.key}
                            className={`rv-inst ${on ? "rv-inst-on" : "rv-inst-off"}`}
                            title={
                              on
                                ? `${inst.name}: difficulty ${tier}`
                                : `${inst.name}: not charted`
                            }
                          >
                            {inst.label}
                          </span>
                        );
                      })}
                    </div>
                  </div>
                  <div className="rv-side">
                    <div className="rv-stats">
                      {song.gameformat && (
                        <span className="rv-format">{song.gameformat}</span>
                      )}
                      {isExternal && (
                        <span className="rv-ext-host" title={`Hosted on ${extHost}`}>
                          ↗ {extHost}
                        </span>
                      )}
                      {formatDuration(song.song_length_sec) && (
                        <span>{formatDuration(song.song_length_sec)}</span>
                      )}
                      {formatBytes(song.size_bytes) && (
                        <span>{formatBytes(song.size_bytes)}</span>
                      )}
                      <span title="Downloads">
                        {"↓"} {formatCount(song.downloads)}
                      </span>
                    </div>
                    {isActive ? (
                      <div className="rv-dl-progress">
                        <div className="rv-dl-bar-outer">
                          <div
                            className="rv-dl-bar-inner"
                            style={{ width: `${activeDownload!.pct}%` }}
                          />
                        </div>
                        <span className="rv-dl-phase">
                          {activeDownload!.phase === "extracting"
                            ? "Extracting…"
                            : activeDownload!.phase === "saving"
                            ? "Saving…"
                            : activeDownload!.phase === "starting"
                            ? "Starting…"
                            : `${activeDownload!.pct}%`}
                        </span>
                      </div>
                    ) : hasUpdate ? (
                      <button
                        className="rv-update-btn"
                        disabled={!!activeDownload}
                        onClick={() => handleDownload(song)}
                        title={`Updated on RhythmVerse (${formatDate(song.uploaded)}) since you downloaded it — click to re-download and replace`}
                      >
                        Update {"⟳"}
                      </button>
                    ) : have ? (
                      <span className="rv-in-lib" title="Already in your library">
                        {"✓"} In library
                      </span>
                    ) : isExternal ? (
                      <button
                        className={`rv-ext-btn${
                          openedIds.has(song.file_id) ? " rv-ext-opened" : ""
                        }`}
                        onClick={() => handleOpenExternal(song)}
                        title={
                          openedIds.has(song.file_id)
                            ? `Opened on ${extHost} before — click to open again`
                            : `Hosted on ${extHost} — opens in your browser to download manually`
                        }
                      >
                        {openedIds.has(song.file_id) ? "Opened ↗" : "Open ↗"}
                      </button>
                    ) : (
                      <button
                        className="rv-dl-btn"
                        disabled={!!activeDownload}
                        onClick={() => handleDownload(song)}
                        title={
                          libraryFolder
                            ? `Download into ${libraryFolder}`
                            : "Download (you'll choose a folder)"
                        }
                      >
                        Download
                      </button>
                    )}
                  </div>
                </div>
              );
            })}
        </div>

        <div className="rv-pagination">
          <button
            className="rv-page-btn"
            disabled={page <= 1 || loading}
            onClick={() => setPage((p) => Math.max(1, p - 1))}
          >
            {"←"} Prev
          </button>
          <span className="rv-page-info">
            Page {page} of {totalPages.toLocaleString()}
          </span>
          <button
            className="rv-page-btn"
            disabled={page >= totalPages || loading}
            onClick={() => setPage((p) => p + 1)}
          >
            Next {"→"}
          </button>
        </div>
      </div>
    </div>
  );
}
