import React, { useState, useEffect, useMemo, useCallback, useRef } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { open } from "@tauri-apps/plugin-dialog";
import { DifficultyRing, TIER_LABELS } from "./DifficultyRing";
import { InstrumentIcon } from "./YargIcon";
import type {
  CatalogFacets,
  CatalogQuery,
  CatalogStatus,
  CatalogSuggestion,
  FixTarget,
  RvBrowseResult,
  RvDownloadRecord,
  RvDownloadResult,
  RvSongFile,
  SongSummary,
} from "../types";
import {
  ArrowDown,
  ArrowLeft,
  ArrowUp,
  Check,
  ChevronLeft,
  ChevronRight,
  Dices,
  Download,
  ExternalLink,
  Minus,
  Music,
  RefreshCw,
  Search,
  Shuffle,
  SlidersHorizontal,
  X,
} from "lucide-react";

interface RhythmVerseModalProps {
  /** Currently-loaded library songs, used to flag "already in library". */
  librarySongs: SongSummary[];
  /** Folder new downloads extract into; if null the user is prompted. */
  libraryFolder: string | null;
  /** Called after a successful download so the caller can refresh the list. */
  onLibraryChanged?: () => void;
  /** Hidden-but-mounted: keeps browsing state alive while out of the way. */
  minimized?: boolean;
  /** Tuck the modal away without unmounting it (preserves query/page/scroll). */
  onMinimize?: () => void;
  /** Bring a minimized modal back to the foreground. */
  onRestore?: () => void;
  /** A broken song to replace: the next download goes next to it and takes its place. */
  fixTarget?: FixTarget | null;
  /** The fix finished (or was cancelled); the caller clears `fixTarget`. */
  onFixEnd?: () => void;
  onClose: () => void;
}

/** Parent folder of a path (either slash style). */
function parentDir(path: string): string {
  return path.replace(/[\\/]+$/, "").replace(/[\\/][^\\/]*$/, "");
}

interface DownloadProgress {
  file_id: string;
  phase: "starting" | "downloading" | "extracting" | "saving" | "done" | "error";
  received: number;
  total: number;
  message: string;
}

const RECORDS_PER_PAGE = 25;

// Downloads run concurrently up to this cap; extra clicks queue behind them.
const MAX_CONCURRENT_DOWNLOADS = 3;

// Live state of one queued/in-flight download (mirrors the backend progress
// phases, plus "queued" for songs still waiting for a free slot).
interface DlState {
  phase: "queued" | "starting" | "downloading" | "extracting" | "saving" | "done";
  pct: number;
}

const SORT_OPTIONS = [
  { value: "update_date", label: "Last updated" },
  { value: "downloads", label: "Most downloaded" },
  { value: "title", label: "Title" },
  { value: "artist", label: "Artist" },
] as const;

// The offline catalog can sort on more than the live API.
const CATALOG_SORT_OPTIONS = [
  { value: "update_date", label: "Last updated" },
  { value: "uploaded", label: "Date added" },
  { value: "downloads", label: "Most downloaded" },
  { value: "title", label: "Title" },
  { value: "artist", label: "Artist" },
  { value: "length", label: "Length" },
  { value: "year", label: "Year" },
] as const;

const DECADES = ["2020s", "2010s", "2000s", "1990s", "1980s", "1970s", "1960s", "Older"];

function decadeRange(d: string): [number | null, number | null] {
  if (!d) return [null, null];
  if (d === "Older") return [null, 1959];
  const y = parseInt(d, 10);
  return [y, y + 9];
}

const LENGTHS = [
  { value: "", label: "Any length", min: null, max: null },
  { value: "short", label: "Under 3 min", min: null, max: 179 },
  { value: "mid", label: "3–5 min", min: 180, max: 300 },
  { value: "long", label: "5–8 min", min: 301, max: 480 },
  { value: "epic", label: "Over 8 min", min: 481, max: null },
] as const;

const ADDED_WITHIN = [
  { value: 0, label: "Added any time" },
  { value: 7, label: "Past week" },
  { value: 30, label: "Past month" },
  { value: 90, label: "Past 3 months" },
  { value: 365, label: "Past year" },
] as const;

// How many charts "Surprise me" picks.
const SURPRISE_COUNT = 5;

// Catalog-only filters (the live API can't do these).
interface Filters {
  instruments: string[]; // must be charted: guitar, bass, drums, vocals, keys
  genre: string;
  gameformat: string;
  decade: string;
  length: string;
  addedDays: number;
  charter: string;
  hideOwned: boolean;
}

const NO_FILTERS: Filters = {
  instruments: [],
  genre: "",
  gameformat: "",
  decade: "",
  length: "",
  addedDays: 0,
  charter: "",
  hideOwned: false,
};

function activeFilterCount(f: Filters): number {
  return (
    f.instruments.length +
    [f.genre, f.gameformat, f.decade, f.length, f.charter.trim()].filter(Boolean).length +
    (f.addedDays > 0 ? 1 : 0) +
    (f.hideOwned ? 1 : 0)
  );
}

// "just now" / "12 min ago" / "3 h ago" / a date, for the last catalog sync.
function formatAgo(iso: string): string {
  const t = Date.parse(iso);
  if (isNaN(t)) return "";
  const min = Math.round((Date.now() - t) / 60000);
  if (min < 1) return "just now";
  if (min < 60) return `${min} min ago`;
  if (min < 24 * 60) return `${Math.round(min / 60)} h ago`;
  return new Date(t).toLocaleDateString();
}

// Core band instruments, in the usual display order.
const INSTRUMENTS = [
  { key: "diff_guitar", label: "G", name: "Guitar", icon: "guitar" },
  { key: "diff_bass", label: "B", name: "Bass", icon: "bass" },
  { key: "diff_drums", label: "D", name: "Drums", icon: "drums" },
  { key: "diff_vocals", label: "V", name: "Vocals", icon: "vocals" },
  { key: "diff_keys", label: "K", name: "Keys", icon: "keys" },
] as const;

// A tier >= 1 means the instrument is charted; 0 / -1 / null means absent.
function isCharted(tier: number | null): boolean {
  return tier != null && tier >= 1;
}

function norm(s: string): string {
  return (s || "").toLowerCase().replace(/[^a-z0-9]/g, "");
}

// Copy text to the clipboard, falling back to a hidden textarea for webviews
// where navigator.clipboard is unavailable or blocked.
async function copyText(text: string): Promise<boolean> {
  try {
    if (navigator.clipboard?.writeText) {
      await navigator.clipboard.writeText(text);
      return true;
    }
  } catch {
    /* fall through to the execCommand fallback */
  }
  try {
    const ta = document.createElement("textarea");
    ta.value = text;
    ta.style.position = "fixed";
    ta.style.opacity = "0";
    document.body.appendChild(ta);
    ta.focus();
    ta.select();
    const ok = document.execCommand("copy");
    document.body.removeChild(ta);
    return ok;
  } catch {
    return false;
  }
}

// Last path segment of a file/folder path (handles both \ and / separators).
function baseName(p: string): string {
  if (!p) return "";
  const parts = p.split(/[\\/]/);
  return parts[parts.length - 1] || "";
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

// Friendly names for RhythmVerse's game-format codes (used as the icon tooltip
// / text fallback). Codes come back lowercase from the API.
const GAME_FORMAT_LABELS: Record<string, string> = {
  ch: "Clone Hero",
  rv: "RhythmVerse",
  yarg: "YARG",
  gh: "Guitar Hero",
  ps: "Phase Shift",
  rb2: "Rock Band 2",
  rb3: "Rock Band 3",
  rb3xbox: "Rock Band 3 (Xbox 360)",
  rb3ps3: "Rock Band 3 (PS3)",
  rb3wii: "Rock Band 3 (Wii)",
  tbrb: "The Beatles: Rock Band",
  wtde: "GH: Warriors of Rock — Definitive Edition",
};

// Show the game format as its icon (Clone Hero, Rock Band, YARG, …), bundled
// under public/icons/games/. Codes with no bundled icon fall back to the
// original text badge via onError.
function GameFormatBadge({ code }: { code: string }) {
  const [failed, setFailed] = useState(false);
  const label = GAME_FORMAT_LABELS[code.toLowerCase()] || code.toUpperCase();
  if (failed) {
    return (
      <span className="rv-format" title={label}>
        {code}
      </span>
    );
  }
  return (
    <img
      className="rv-format-icon"
      src={`/icons/games/${code.toLowerCase()}.png`}
      alt={label}
      title={label}
      loading="lazy"
      onError={() => setFailed(true)}
    />
  );
}

export function RhythmVerseModal({
  librarySongs,
  libraryFolder,
  onLibraryChanged,
  minimized = false,
  onMinimize,
  onRestore,
  fixTarget = null,
  onFixEnd,
  onClose,
}: RhythmVerseModalProps) {
  // "Fix it": message shown after a replacement, and the broken song's linked
  // RhythmVerse file (highlighted in results when present).
  const [fixNotice, setFixNotice] = useState<string | null>(null);
  const [fixLinkedId, setFixLinkedId] = useState<string | null>(null);
  const [text, setText] = useState(""); // input box
  const [query, setQuery] = useState(""); // submitted search
  const [sortBy, setSortBy] = useState<string>("update_date");
  const [sortOrder, setSortOrder] = useState<"DESC" | "ASC">("DESC");
  const [page, setPage] = useState(1);
  // Editable page-number field (the "Page __ of N" jump box). Kept as its own
  // string state so typing doesn't refetch on every keystroke — the jump is
  // committed on Enter/blur. Synced back to `page` whenever the page changes.
  const [pageInput, setPageInput] = useState("1");

  const [result, setResult] = useState<RvBrowseResult | null>(null);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);

  // file_id -> download record for files held locally. Gives the exact "in
  // library" match plus the version baseline (rv_upload_date) for updates.
  const [downloads, setDownloads] = useState<Map<string, RvDownloadRecord>>(
    new Map()
  );
  // file_ids whose off-site link the user has opened in the browser
  const [openedIds, setOpenedIds] = useState<Set<string>>(new Set());
  // Per-file progress for everything currently downloading OR queued, keyed by
  // file_id. Up to MAX_CONCURRENT_DOWNLOADS run at once; the rest wait in
  // `queueRef` with phase "queued". An entry is removed on success (the row then
  // shows "In library") or on error (reverts to a Download button).
  const [dlProgress, setDlProgress] = useState<Map<string, DlState>>(new Map());
  // Pending (song, dest) pairs waiting for a free slot, and the live in-flight
  // count. Refs, not state, so the scheduler reads current values without
  // stale-closure races and without re-rendering on every tick.
  const queueRef = useRef<Array<{ song: RvSongFile; dest: string; fix?: FixTarget }>>([]);
  const activeCountRef = useRef(0);
  // file_ids that are queued or downloading — guards against enqueuing the same
  // song twice (e.g. a double-click).
  const inFlightRef = useRef<Set<string>>(new Set());
  // true while a library rescan (triggered by the Refresh button) is in flight
  const [libRefreshing, setLibRefreshing] = useState(false);
  // file_id whose link was just copied, for a transient "Copied!" indicator
  const [copiedId, setCopiedId] = useState<string | null>(null);

  // Offline catalog: once a full sync has finished, browsing queries the local
  // copy (instant, offline, extra filters) instead of the live API.
  const [catalog, setCatalog] = useState<CatalogStatus | null>(null);
  const useCatalog = !!catalog?.ready;
  const [filters, setFilters] = useState<Filters>(NO_FILTERS);
  const [showFilters, setShowFilters] = useState(false);
  const [charterText, setCharterText] = useState("");
  const [facets, setFacets] = useState<CatalogFacets | null>(null);
  // >0 while showing "Surprise me" picks; bumping it reshuffles.
  const [surprise, setSurprise] = useState(0);
  // Bumped after the owned-song keys reach the backend, so "hide songs I
  // already have" re-queries against the fresh set.
  const [ownedVersion, setOwnedVersion] = useState(0);
  const [suggestions, setSuggestions] = useState<CatalogSuggestion[]>([]);
  const [suggestOpen, setSuggestOpen] = useState(false);
  const [suggestIdx, setSuggestIdx] = useState(-1);

  useEffect(() => {
    invoke<CatalogStatus>("catalog_status")
      .then(setCatalog)
      .catch(() => {});
    const un = listen<CatalogStatus>("catalog-sync", (e) => setCatalog(e.payload));
    return () => {
      un.then((fn) => fn());
    };
  }, []);

  // Filter option lists; refreshed whenever a sync finishes.
  const catalogSyncing = !!catalog?.syncing;
  useEffect(() => {
    if (!useCatalog || catalogSyncing) return;
    invoke<CatalogFacets>("catalog_facets")
      .then(setFacets)
      .catch(() => {});
  }, [useCatalog, catalogSyncing]);

  const updateFilters = (patch: Partial<Filters>) => {
    setFilters((f) => ({ ...f, ...patch }));
    setSurprise(0);
    setPage(1);
  };

  // Charter box: commit to the filters after a short typing pause.
  useEffect(() => {
    const t = window.setTimeout(() => {
      if (charterText.trim() !== filters.charter) updateFilters({ charter: charterText.trim() });
    }, 300);
    return () => window.clearTimeout(t);
  }, [charterText]);

  // Catalog search is local, so search as you type (after a short pause).
  useEffect(() => {
    if (!useCatalog) return;
    const t = window.setTimeout(() => {
      const q = text.trim();
      if (q !== query) {
        setQuery(q);
        setPage(1);
        setSurprise(0);
      }
    }, 250);
    return () => window.clearTimeout(t);
  }, [text, useCatalog]);

  // Type-ahead: artists and titles starting with what's typed.
  useEffect(() => {
    if (!useCatalog || !suggestOpen) {
      setSuggestions([]);
      return;
    }
    const q = text.trim();
    if (q.length < 2) {
      setSuggestions([]);
      return;
    }
    let cancelled = false;
    const t = window.setTimeout(() => {
      invoke<CatalogSuggestion[]>("catalog_suggest", { text: q })
        .then((s) => {
          if (!cancelled) {
            // Hide a lone suggestion that's exactly what's typed.
            const rest = s.filter((x) => x.value.toLowerCase() !== q.toLowerCase());
            setSuggestions(rest);
            setSuggestIdx(-1);
          }
        })
        .catch(() => {});
    }, 120);
    return () => {
      cancelled = true;
      window.clearTimeout(t);
    };
  }, [text, useCatalog, suggestOpen]);

  const pickSuggestion = (s: CatalogSuggestion) => {
    setText(s.value);
    setQuery(s.value);
    setPage(1);
    setSurprise(0);
    setSuggestOpen(false);
  };

  const refreshDownloads = useCallback(() => {
    invoke<RvDownloadRecord[]>("rv_download_records")
      .then((recs) => setDownloads(new Map(recs.map((r) => [r.file_id, r]))))
      .catch(() => {});
  }, []);

  useEffect(() => {
    refreshDownloads();
    invoke<string[]>("rv_opened_ids")
      .then((ids) => setOpenedIds(new Set(ids)))
      .catch(() => {});
  }, [refreshDownloads]);

  // Rescan the library so files added manually (e.g. after an "Open ↗"
  // external download) get picked up without restarting the app.
  const handleRefreshLibrary = useCallback(() => {
    if (!onLibraryChanged) return;
    setLibRefreshing(true);
    onLibraryChanged();
  }, [onLibraryChanged]);

  // Clear the refreshing flag once the parent hands us a new library snapshot.
  useEffect(() => {
    setLibRefreshing(false);
  }, [librarySongs]);

  // Fold each backend progress event into the per-file map. Events are tagged
  // with file_id, so several concurrent downloads update independently. Ignore
  // events for files we're no longer tracking (already finished/removed).
  useEffect(() => {
    const un = listen<DownloadProgress>("rv-download-progress", (e) => {
      const p = e.payload;
      if (p.phase === "error") return; // surfaced by the runDownload catch
      setDlProgress((cur) => {
        const prev = cur.get(p.file_id);
        if (!prev) return cur;
        const pct =
          p.phase === "extracting" || p.phase === "done"
            ? 100
            : p.total > 0
            ? Math.round((p.received / p.total) * 100)
            : prev.pct;
        const next = new Map(cur);
        next.set(p.file_id, { phase: p.phase as DlState["phase"], pct });
        return next;
      });
    });
    return () => {
      un.then((fn) => fn());
    };
  }, []);

  // Copy this song's RhythmVerse page link so it can be pasted into the
  // editor's "RhythmVerse Link" field — the only way to grab the link for a
  // song that's already in the library (its Download/Open button is gone).
  const handleCopyLink = useCallback((song: RvSongFile) => {
    const link = song.detail_url || song.download_url;
    copyText(link).then((ok) => {
      if (ok) {
        setCopiedId(song.file_id);
        window.setTimeout(
          () => setCopiedId((c) => (c === song.file_id ? null : c)),
          1500
        );
      } else {
        setError("Couldn't copy the link to the clipboard.");
      }
    });
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

  // "Got it": the user confirms they've manually placed this (usually off-site)
  // download in their library. Records the exact file_id so the badge is precise
  // regardless of how the chart's own metadata is spelled.
  const handleMarkDownloaded = useCallback(
    (song: RvSongFile) => {
      invoke("rv_mark_downloaded", {
        fileId: song.file_id,
        songId: song.song_id,
        artist: song.artist,
        title: song.title,
        fileName: song.file_name,
        uploaded: song.uploaded,
      })
        .then(() => refreshDownloads())
        .catch((e) => setError(String(e)));
    },
    [refreshDownloads]
  );

  // Update path for an EXTERNAL file: YARGLE can't scrape the off-site host, so
  // re-open the link for a manual re-download and optimistically refresh the
  // timestamp (mirrors the "Opened" flow) so the Update flag clears. Preserves
  // the linked folder path (touch, not re-mark).
  const handleUpdateExternal = useCallback(
    (song: RvSongFile) => {
      invoke("rv_open_external", { url: song.external_url })
        .then(() => {
          setOpenedIds((prev) => new Set(prev).add(song.file_id));
          invoke("rv_touch_downloaded", { fileId: song.file_id })
            .then(() => refreshDownloads())
            .catch(() => {});
        })
        .catch((e) => setError(String(e)));
    },
    [refreshDownloads]
  );

  // Undo a manual "Got it" mark (backend only deletes path-less manual marks,
  // never a real YARGLE download record).
  const handleUnmarkDownloaded = useCallback(
    (song: RvSongFile) => {
      invoke("rv_unmark_downloaded", { fileId: song.file_id })
        .then(() => refreshDownloads())
        .catch((e) => setError(String(e)));
    },
    [refreshDownloads]
  );

  // Run one download to completion, then free its slot and pull the next queued
  // item. The backend `rv_download` is fully independent per call (own HTTP
  // client/cookie jar, own buffer), so several can safely run at once.
  const runDownload = async (song: RvSongFile, dest: string, fix?: FixTarget) => {
    setDlProgress((cur) => new Map(cur).set(song.file_id, { phase: "starting", pct: 0 }));
    try {
      const res = await invoke<RvDownloadResult>("rv_download", {
        fileId: song.file_id,
        destFolder: dest,
        songId: song.song_id,
        artist: song.artist,
        title: song.title,
        fileName: song.file_name,
        uploaded: song.uploaded,
        // Off-site files (Drive, Mediafire, …) are resolved by the backend.
        externalUrl: song.external_url || null,
      });
      if (fix) {
        // Recycle the broken copy and give the new one its name.
        await invoke<string>("rv_replace_broken", {
          brokenPath: fix.path,
          newPath: res.extracted_to,
        });
        setFixNotice(`Replaced the broken copy of ${fix.name}.`);
      }
      refreshDownloads();
      onLibraryChanged?.();
    } catch (e) {
      setError(String(e));
    } finally {
      // Succeeded (row becomes "In library") or failed (reverts to a Download
      // button) — either way drop it from the map and free the slot.
      setDlProgress((cur) => {
        const next = new Map(cur);
        next.delete(song.file_id);
        return next;
      });
      inFlightRef.current.delete(song.file_id);
      activeCountRef.current -= 1;
      pumpQueue();
    }
  };

  // Start queued downloads until the concurrency cap is reached.
  const pumpQueue = () => {
    while (
      activeCountRef.current < MAX_CONCURRENT_DOWNLOADS &&
      queueRef.current.length > 0
    ) {
      const next = queueRef.current.shift()!;
      activeCountRef.current += 1;
      void runDownload(next.song, next.dest, next.fix);
    }
  };

  // Queue a song for download (up to MAX_CONCURRENT_DOWNLOADS run at once).
  // Resolves the destination once, up front, so a burst of clicks never stacks
  // folder pickers.
  const handleDownload = async (song: RvSongFile) => {
    if (inFlightRef.current.has(song.file_id)) return; // already queued/running
    // In fix mode the first download replaces the broken song: it lands in the
    // same folder so it can take the broken copy's name afterwards.
    const fix = fixTarget ?? undefined;
    if (fix) onFixEnd?.();
    let dest = fix ? parentDir(fix.path) : libraryFolder;
    if (!dest) {
      const picked = await open({ directory: true, multiple: false });
      if (!picked) return;
      dest = picked as string;
    }
    setError(null);
    inFlightRef.current.add(song.file_id);
    setDlProgress((cur) => new Map(cur).set(song.file_id, { phase: "queued", pct: 0 }));
    queueRef.current.push({ song, dest, fix });
    pumpQueue();
  };

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
      // Also index the folder/file basename. The chart's song.ini `name` is
      // often trimmed (e.g. drops "(LIVE at Reading Festival)") while the
      // download folder keeps RhythmVerse's full title — matching against it
      // catches externally-downloaded songs the display_name would miss. This
      // stays a full exact-string match (parenthetical preserved), so it does
      // NOT re-introduce live-vs-studio false positives.
      const base = baseName(s.path);
      if (base) artistTitle.add(norm(base));
    }
    return { artistTitle };
  }, [librarySongs]);

  const inLibrary = useCallback(
    (song: RvSongFile): boolean => {
      if (downloads.has(song.file_id)) return true; // exact: downloaded via YARGLE
      if (song.artist && song.title) {
        // Test both orderings: library display names / folders are usually
        // "Artist - Title" but some (and many CON headers) are "Title - Artist".
        return (
          libIndex.artistTitle.has(norm(song.artist + song.title)) ||
          libIndex.artistTitle.has(norm(song.title + song.artist))
        );
      }
      return false;
    },
    [libIndex, downloads]
  );

  // True when the site's CURRENT upload_date is newer than the upload_date of
  // the version we hold. Both come from RhythmVerse's own clock, so there's no
  // cross-clock skew (the old code compared the site's clock against our local
  // "downloaded_at", which false-positived on freshly-uploaded files). An empty
  // baseline (unknown version) never flags — it gets backfilled below instead.
  const needsUpdate = useCallback(
    (song: RvSongFile): boolean => {
      const rec = downloads.get(song.file_id);
      if (!rec || !rec.rv_upload_date) return false;
      const held = parseRvDate(rec.rv_upload_date);
      const current = parseRvDate(song.uploaded);
      if (held == null || current == null) return false;
      return current > held + 1000; // strictly newer (1s guard on formatting)
    },
    [downloads]
  );

  // Backfill the version baseline for held songs that don't have one yet (e.g.
  // editor links, which carry no RV data): record the site's current
  // upload_date as "the version you have". Runs once per file_id per session,
  // and only touches empty baselines, so it never masks a real update.
  const backfilledRef = useRef<Set<string>>(new Set());
  useEffect(() => {
    if (!result) return;
    const toFix = result.songs.filter((s) => {
      const rec = downloads.get(s.file_id);
      return (
        rec &&
        !rec.rv_upload_date &&
        !!s.uploaded &&
        !backfilledRef.current.has(s.file_id)
      );
    });
    if (toFix.length === 0) return;
    Promise.all(
      toFix.map((s) => {
        backfilledRef.current.add(s.file_id);
        return invoke("rv_set_upload_baseline", {
          fileId: s.file_id,
          uploaded: s.uploaded,
        }).catch(() => {});
      })
    ).then(() => refreshDownloads());
  }, [result, downloads, refreshDownloads]);

  // Hand the library's artist+title keys to the backend for "hide songs I
  // already have" (the catalog can't see the in-memory library otherwise).
  useEffect(() => {
    if (!useCatalog) return;
    const t = window.setTimeout(() => {
      invoke("catalog_set_owned", { keys: Array.from(libIndex.artistTitle) })
        .then(() => setOwnedVersion((v) => v + 1))
        .catch(() => {});
    }, 500);
    return () => window.clearTimeout(t);
  }, [libIndex, useCatalog]);

  useEffect(() => {
    let cancelled = false;
    setLoading(true);
    setError(null);
    let request: Promise<RvBrowseResult>;
    if (useCatalog) {
      const [yearMin, yearMax] = decadeRange(filters.decade);
      const len = LENGTHS.find((l) => l.value === filters.length) ?? LENGTHS[0];
      const q: CatalogQuery = {
        text: query,
        charter: filters.charter,
        genre: filters.genre,
        gameformat: filters.gameformat,
        instruments: filters.instruments,
        yearMin,
        yearMax,
        lengthMin: len.min,
        lengthMax: len.max,
        addedWithinDays: filters.addedDays || null,
        // Fix mode is looking for a song you have, so never hide it.
        hideOwned: filters.hideOwned && !fixTarget,
        sortBy,
        sortOrder,
        page: surprise > 0 ? 1 : page,
        records: surprise > 0 ? SURPRISE_COUNT : RECORDS_PER_PAGE,
        random: surprise > 0,
      };
      request = invoke<RvBrowseResult>("catalog_query", { query: q });
    } else {
      request = invoke<RvBrowseResult>("rv_browse", {
        game: "yarg",
        text: query,
        page,
        records: RECORDS_PER_PAGE,
        // The live API only knows the shared sort keys.
        sortBy: SORT_OPTIONS.some((o) => o.value === sortBy) ? sortBy : "update_date",
        sortOrder,
      });
    }
    request
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
    // ownedVersion only matters while hiding owned songs.
  }, [
    query,
    sortBy,
    sortOrder,
    page,
    useCatalog,
    filters,
    surprise,
    filters.hideOwned ? ownedVersion : 0,
    !!fixTarget,
  ]);

  // Reset the results list to the top whenever the visible set changes (page
  // turn, new search, re-sort, or new filters). Without this, turning the page
  // while scrolled to the bottom leaves you stranded at the bottom of the new page.
  const resultsRef = useRef<HTMLDivElement>(null);
  useEffect(() => {
    resultsRef.current?.scrollTo({ top: 0 });
  }, [query, sortBy, sortOrder, page, filters, surprise]);

  // Keep the jump box showing the actual page after Prev/Next or a jump.
  useEffect(() => {
    setPageInput(String(page));
  }, [page]);

  const submitSearch = () => {
    setPage(1);
    setSurprise(0);
    setSuggestOpen(false);
    setQuery(text.trim());
  };

  // Entering fix mode: search for the broken song and look up its link.
  useEffect(() => {
    if (!fixTarget) return;
    setFixNotice(null);
    const q = [fixTarget.artist, fixTarget.title].filter(Boolean).join(" ").trim() || fixTarget.name;
    setText(q);
    setQuery(q);
    setPage(1);
    // Filters could hide the very song being fixed.
    setFilters(NO_FILTERS);
    setCharterText("");
    setSurprise(0);
    setFixLinkedId(null);
    invoke<string | null>("rv_linked_file_id", { path: fixTarget.path })
      .then((id) => setFixLinkedId(id))
      .catch(() => {});
  }, [fixTarget?.path]);

  const totalPages = result
    ? Math.max(1, Math.ceil(result.total_filtered / RECORDS_PER_PAGE))
    : 1;

  // Parse the jump box and go there, clamped to [1, totalPages]. Invalid or
  // unchanged input just snaps the field back to the current page.
  const commitPageInput = () => {
    const n = parseInt(pageInput, 10);
    if (!Number.isNaN(n)) {
      const target = Math.min(totalPages, Math.max(1, n));
      if (target !== page) {
        setPage(target);
        return;
      }
    }
    setPageInput(String(page));
  };

  // Split the in-flight map into "downloading now" vs "waiting" for the status
  // bar and the minimized-pill badge.
  let downloadingCount = 0;
  let queuedCount = 0;
  for (const st of dlProgress.values()) {
    if (st.phase === "queued") queuedCount += 1;
    else downloadingCount += 1;
  }

  // Clicking outside the panel (on the dimmed backdrop / main window) tucks the
  // modal away rather than closing it, so a stray click never loses your place.
  const dismiss = onMinimize ?? onClose;

  return (
    <>
      <div
        className="art-search-overlay"
        style={minimized ? { display: "none" } : undefined}
        onClick={dismiss}
      >
      <div className="art-search-panel rv-panel" onClick={(e) => e.stopPropagation()}>
        <div className="art-search-header">
          <h3>Browse RhythmVerse</h3>
          <div className="rv-header-actions">
            {onMinimize && (
              <button
                className="art-search-close"
                onClick={onMinimize}
                title="Minimize — keep your place and return to the app (e.g. to paste a link)"
                aria-label="Minimize"
              >
                <Minus size={18} />
              </button>
            )}
            <button
              className="art-search-close"
              onClick={onClose}
              title="Close"
              aria-label="Close"
            >
              <X size={18} />
            </button>
          </div>
        </div>

        {fixTarget ? (
          <div className="rv-fix-banner">
            <span>
              <b>Fixing {fixTarget.name}.</b> The next song you download replaces this broken copy
              {fixLinkedId ? " (its linked version is highlighted)" : ""}.
            </span>
            <button className="rv-fix-cancel" onClick={() => onFixEnd?.()}>
              Cancel
            </button>
          </div>
        ) : fixNotice ? (
          <div className="rv-fix-banner rv-fix-done">
            <span>{fixNotice}</span>
            <button className="rv-fix-cancel" onClick={() => setFixNotice(null)}>
              Dismiss
            </button>
          </div>
        ) : null}

        <div className="rv-controls">
          <div className="rv-search-wrap">
            <input
              type="text"
              className="rv-search-input"
              placeholder={
                useCatalog
                  ? "Search titles, artists, albums, charters…"
                  : "Search songs, artists…"
              }
              value={text}
              onChange={(e) => {
                setText(e.target.value);
                setSuggestOpen(true);
              }}
              onBlur={() => setSuggestOpen(false)}
              onKeyDown={(e) => {
                if (suggestions.length > 0 && suggestOpen) {
                  if (e.key === "ArrowDown" || e.key === "ArrowUp") {
                    e.preventDefault();
                    const n = suggestions.length;
                    setSuggestIdx((i) =>
                      e.key === "ArrowDown" ? (i + 1) % n : (i - 1 + n) % n
                    );
                    return;
                  }
                  if (e.key === "Escape") {
                    setSuggestOpen(false);
                    return;
                  }
                  if (e.key === "Enter" && suggestIdx >= 0) {
                    pickSuggestion(suggestions[suggestIdx]);
                    return;
                  }
                }
                if (e.key === "Enter") submitSearch();
              }}
              autoFocus
            />
            {suggestOpen && suggestions.length > 0 && (
              <ul className="rv-suggest">
                {suggestions.map((sg, i) => (
                  <li
                    key={`${sg.kind}:${sg.value}`}
                    className={i === suggestIdx ? "rv-suggest-active" : ""}
                    // mousedown, not click: fires before the input's blur closes the list
                    onMouseDown={(e) => {
                      e.preventDefault();
                      pickSuggestion(sg);
                    }}
                  >
                    <span className="rv-suggest-value">{sg.value}</span>
                    <span className="rv-suggest-kind">
                      {sg.kind} · {sg.count.toLocaleString()}
                    </span>
                  </li>
                ))}
              </ul>
            )}
          </div>
          <button className="rv-search-btn" onClick={submitSearch}>
            <Search size={15} /> Search
          </button>
          <select
            className="rv-sort-select"
            value={sortBy}
            onChange={(e) => {
              setSortBy(e.target.value);
              setPage(1);
              setSurprise(0);
            }}
          >
            {(useCatalog ? CATALOG_SORT_OPTIONS : SORT_OPTIONS).map((o) => (
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
              setSurprise(0);
            }}
          >
            {sortOrder === "DESC" ? <ArrowDown size={15} /> : <ArrowUp size={15} />}
          </button>
          {useCatalog && (
            <button
              className={`rv-filter-btn${showFilters ? " rv-filter-btn-open" : ""}`}
              onClick={() => setShowFilters((v) => !v)}
              title="Filter by instrument, genre, decade, length, charter and more"
            >
              <SlidersHorizontal size={14} /> Filters
              {activeFilterCount(filters) > 0 && (
                <span className="rv-filter-count">{activeFilterCount(filters)}</span>
              )}
            </button>
          )}
          {onLibraryChanged && (
            <button
              className="rv-order-btn"
              disabled={libRefreshing}
              title="Rescan your library — use after manually adding files from an external (Open) download so they show as In library"
              onClick={handleRefreshLibrary}
            >
              <RefreshCw size={14} className={libRefreshing ? "spin" : undefined} />
            </button>
          )}
        </div>

        {useCatalog && showFilters && (
          <div className="rv-filters">
            <div className="rv-filter-insts" title="Only show charts with these instruments">
              {INSTRUMENTS.map((inst) => {
                const key = inst.name.toLowerCase();
                const on = filters.instruments.includes(key);
                return (
                  <button
                    key={key}
                    className={`rv-inst-chip${on ? " rv-inst-chip-on" : ""}`}
                    title={on ? `${inst.name} required` : `Require ${inst.name.toLowerCase()}`}
                    onClick={() => {
                      // Functional update: quick successive toggles must not
                      // overwrite each other from a stale `filters`.
                      setFilters((f) => ({
                        ...f,
                        instruments: f.instruments.includes(key)
                          ? f.instruments.filter((i) => i !== key)
                          : [...f.instruments, key],
                      }));
                      setSurprise(0);
                      setPage(1);
                    }}
                    aria-pressed={on}
                  >
                    <InstrumentIcon instrument={inst.icon} size={22} />
                  </button>
                );
              })}
            </div>
            <select
              className="rv-sort-select"
              value={filters.genre}
              onChange={(e) => updateFilters({ genre: e.target.value })}
            >
              <option value="">Any genre</option>
              {facets?.genres.map((g) => (
                <option key={g.value} value={g.value}>
                  {g.value} ({g.count.toLocaleString()})
                </option>
              ))}
            </select>
            <select
              className="rv-sort-select"
              value={filters.decade}
              onChange={(e) => updateFilters({ decade: e.target.value })}
            >
              <option value="">Any decade</option>
              {DECADES.map((d) => (
                <option key={d} value={d}>
                  {d === "Older" ? "Before 1960" : d}
                </option>
              ))}
            </select>
            <select
              className="rv-sort-select"
              value={filters.length}
              onChange={(e) => updateFilters({ length: e.target.value })}
            >
              {LENGTHS.map((l) => (
                <option key={l.value} value={l.value}>
                  {l.label}
                </option>
              ))}
            </select>
            <select
              className="rv-sort-select"
              value={filters.addedDays}
              onChange={(e) => updateFilters({ addedDays: Number(e.target.value) })}
            >
              {ADDED_WITHIN.map((a) => (
                <option key={a.value} value={a.value}>
                  {a.label}
                </option>
              ))}
            </select>
            <select
              className="rv-sort-select"
              value={filters.gameformat}
              onChange={(e) => updateFilters({ gameformat: e.target.value })}
            >
              <option value="">Any format</option>
              {facets?.formats.map((f) => (
                <option key={f.value} value={f.value}>
                  {`${GAME_FORMAT_LABELS[f.value] || f.value.toUpperCase()} (${f.count.toLocaleString()})`}
                </option>
              ))}
            </select>
            <input
              type="text"
              className="rv-search-input rv-charter-input"
              placeholder="Charter or uploader"
              value={charterText}
              onChange={(e) => setCharterText(e.target.value)}
            />
            <label
              className="rv-filter-check"
              title="Hide charts that match a song already in your library"
            >
              <input
                type="checkbox"
                checked={filters.hideOwned}
                onChange={(e) => updateFilters({ hideOwned: e.target.checked })}
              />
              Hide songs I have
            </label>
            <button
              className="rv-surprise-btn"
              onClick={() => setSurprise((n) => n + 1)}
              title={`Pick ${SURPRISE_COUNT} random charts that match your search and filters`}
            >
              <Dices size={14} /> Surprise me
            </button>
            {activeFilterCount(filters) > 0 && (
              <button
                className="rv-filter-clear"
                onClick={() => {
                  updateFilters(NO_FILTERS);
                  setCharterText("");
                }}
              >
                Clear filters
              </button>
            )}
          </div>
        )}

        <div className="rv-status-bar">
          {result && !error && (
            <span>
              {surprise > 0
                ? `${result.songs.length} random pick${
                    result.songs.length !== 1 ? "s" : ""
                  } from ${result.total_filtered.toLocaleString()} matching`
                : `${result.total_filtered.toLocaleString()} ${
                    query.trim().length >= (useCatalog ? 1 : 3)
                      ? `result${result.total_filtered !== 1 ? "s" : ""} for "${query.trim()}"`
                      : "songs"
                  }`}
            </span>
          )}
          {loading && <span className="rv-status-loading">Loading…</span>}
          {catalog && (
            <span
              className={`rv-catalog-status${catalog.error ? " rv-catalog-error" : ""}`}
              title={
                catalog.error
                  ? `Last catalog sync failed: ${catalog.error}. Retrying in a few minutes.`
                  : useCatalog
                  ? `${catalog.rows.toLocaleString()} charts stored offline. Syncs every 30 minutes.`
                  : "Downloading the RhythmVerse catalog for instant offline search and filters. Live results until it's done."
              }
            >
              {catalog.syncing && catalog.mode === "full"
                ? `Building offline catalog… ${
                    catalog.pages_total > 0
                      ? Math.round((catalog.pages_done / catalog.pages_total) * 100)
                      : 0
                  }%`
                : catalog.syncing
                ? "Offline catalog · syncing…"
                : catalog.error
                ? "Catalog sync failed"
                : useCatalog
                ? `Offline catalog · synced ${formatAgo(catalog.last_sync)}`
                : ""}
              {!catalog.syncing && (catalog.ready || catalog.error) && (
                <button
                  className="rv-catalog-sync"
                  onClick={() => invoke("catalog_sync_now").catch(() => {})}
                  title="Sync the offline catalog now"
                >
                  <RefreshCw size={12} />
                </button>
              )}
            </span>
          )}
          {(downloadingCount > 0 || queuedCount > 0) && (
            <span className="rv-status-dl">
              {downloadingCount > 0 ? `Downloading ${downloadingCount}` : ""}
              {downloadingCount > 0 && queuedCount > 0 ? " · " : ""}
              {queuedCount > 0 ? `${queuedCount} queued` : ""}
            </span>
          )}
        </div>

        <div className="rv-results" ref={resultsRef}>
          {error && <div className="art-search-error">{error}</div>}

          {!error && result && result.songs.length === 0 && !loading && (
            <div className="duplicate-no-results">
              <p>No matches found{query ? ` for "${query}"` : ""}.</p>
            </div>
          )}

          {!error &&
            result &&
            result.songs.map((song) => {
              const have = !fixTarget && inLibrary(song);
              const hasUpdate = !fixTarget && needsUpdate(song);
              const isFixMatch = !!fixTarget && song.file_id === fixLinkedId;
              const dl = dlProgress.get(song.file_id);
              const isActive = !!dl;
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
              // Off-site but fetchable in-app (Drive, Mediafire, Dropbox, …).
              const extAuto = isExternal && song.external_auto;
              // Off-site and browser-only (MEGA, Ko-fi, other pages).
              const extManual = isExternal && !song.external_auto;
              const extHost = isExternal ? externalHostName(song.external_url) : "";
              return (
                <div key={song.file_id} className={`rv-row${isFixMatch ? " rv-row-fix-match" : ""}`}>
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
                      <div className="rv-thumb-placeholder"><Music size={22} /></div>
                    )}
                  </div>
                  <div className="rv-info">
                    <div className="rv-title">
                      <a
                        className="rv-title-link"
                        href={song.detail_url || song.download_url}
                        onClick={(e) => {
                          e.preventDefault();
                          handleCopyLink(song);
                        }}
                        title="Click to copy this song's RhythmVerse link — paste it into the editor's RhythmVerse Link field"
                      >
                        {song.title || song.file_name}
                      </a>
                      {copiedId === song.file_id && (
                        <span className="rv-copied"><Check size={12} /> Copied</span>
                      )}
                    </div>
                    <div className="rv-artist">{song.artist || "Unknown artist"}</div>
                    {meta && <div className="rv-meta">{meta}</div>}
                    {byline && <div className="rv-byline">{byline}</div>}
                    <div className="rv-instruments">
                      {INSTRUMENTS.map((inst) => {
                        // diff_* are already tiers (0-7), not raw ranks, so
                        // they feed the ring directly (clamped for safety).
                        const raw = song[inst.key];
                        const on = isCharted(raw);
                        const tier = on ? Math.min(7, raw as number) : 0;
                        return (
                          <div
                            key={inst.key}
                            className={`rv-inst-ring ${on ? "" : "rv-inst-ring-off"}`}
                            title={
                              on
                                ? `${inst.name}: ${TIER_LABELS[tier]} (${tier})`
                                : `${inst.name}: not charted`
                            }
                          >
                            <DifficultyRing tier={tier} size={34} instrument={inst.icon} />
                          </div>
                        );
                      })}
                    </div>
                  </div>
                  <div className="rv-side">
                    <div className="rv-stats">
                      {song.gameformat && <GameFormatBadge code={song.gameformat} />}
                      {isExternal && (
                        <span className="rv-ext-host" title={`Hosted on ${extHost}`}>
                          <ExternalLink size={11} /> {extHost}
                        </span>
                      )}
                      {formatDuration(song.song_length_sec) && (
                        <span>{formatDuration(song.song_length_sec)}</span>
                      )}
                      {formatBytes(song.size_bytes) && (
                        <span>{formatBytes(song.size_bytes)}</span>
                      )}
                      <span title="Downloads">
                        <Download size={11} /> {formatCount(song.downloads)}
                      </span>
                    </div>
                    {isActive ? (
                      <div className="rv-dl-progress">
                        <div className="rv-dl-bar-outer">
                          <div
                            className={`rv-dl-bar-inner${
                              dl!.phase === "queued" ? " rv-dl-bar-queued" : ""
                            }`}
                            style={{
                              width: dl!.phase === "queued" ? "100%" : `${dl!.pct}%`,
                            }}
                          />
                        </div>
                        <span className="rv-dl-phase">
                          {dl!.phase === "queued"
                            ? "Queued…"
                            : dl!.phase === "extracting"
                            ? "Extracting…"
                            : dl!.phase === "saving"
                            ? "Saving…"
                            : dl!.phase === "starting"
                            ? "Starting…"
                            : `${dl!.pct}%`}
                        </span>
                      </div>
                    ) : hasUpdate ? (
                      extManual ? (
                        <button
                          className="rv-update-btn"
                          onClick={() => handleUpdateExternal(song)}
                          title={`Updated on RhythmVerse (${formatDate(song.uploaded)}) since you got it — re-open ${extHost} to download the new version`}
                        >
                          <ExternalLink size={14} /> Update
                        </button>
                      ) : (
                        <button
                          className="rv-update-btn"
                          onClick={() => handleDownload(song)}
                          title={`Updated on RhythmVerse (${formatDate(song.uploaded)}) since you downloaded it — click to re-download and replace`}
                        >
                          <RefreshCw size={14} /> Update
                        </button>
                      )
                    ) : have ? (
                      // Only a path-less "Got it" mark can be undone; a real
                      // download record (with a folder path) can't.
                      downloads.get(song.file_id)?.manual ? (
                        <button
                          className="rv-in-lib rv-in-lib-manual"
                          onClick={() => handleUnmarkDownloaded(song)}
                          title="Marked as in your library — click to undo"
                        >
                          <Check size={14} /> In library
                        </button>
                      ) : (
                        <span className="rv-in-lib" title="Already in your library">
                          <Check size={14} /> In library
                        </span>
                      )
                    ) : extAuto ? (
                      <div className="rv-ext-actions">
                        <button
                          className="rv-dl-btn"
                          onClick={() => handleDownload(song)}
                          title={
                            libraryFolder
                              ? `Download from ${extHost} into ${libraryFolder}`
                              : `Download from ${extHost} (you'll choose a folder)`
                          }
                        >
                          <Download size={14} /> Download
                        </button>
                        <button
                          className="rv-open-alt-btn"
                          onClick={() => handleOpenExternal(song)}
                          title={`Open on ${extHost} in your browser instead`}
                        >
                          <ExternalLink size={14} /> Open
                        </button>
                      </div>
                    ) : extManual ? (
                      <div className="rv-ext-actions">
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
                          <ExternalLink size={14} /> {openedIds.has(song.file_id) ? "Opened" : "Open"}
                        </button>
                        <button
                          className="rv-gotit-btn"
                          onClick={() => handleMarkDownloaded(song)}
                          title="Already grabbed this and added it to your library? Mark it as In library (exact match by file ID)"
                        >
                          <Check size={14} /> Got it
                        </button>
                      </div>
                    ) : (
                      <button
                        className="rv-dl-btn"
                        onClick={() => handleDownload(song)}
                        title={
                          libraryFolder
                            ? `Download into ${libraryFolder}`
                            : "Download (you'll choose a folder)"
                        }
                      >
                        <Download size={14} /> Download
                      </button>
                    )}
                  </div>
                </div>
              );
            })}
        </div>

        {surprise > 0 ? (
          <div className="rv-pagination">
            <button className="rv-page-btn" onClick={() => setSurprise(0)}>
              <ArrowLeft size={14} /> Back to list
            </button>
            <button
              className="rv-page-btn"
              disabled={loading}
              onClick={() => setSurprise((n) => n + 1)}
            >
              <Shuffle size={14} /> Shuffle again
            </button>
          </div>
        ) : (
        <div className="rv-pagination">
          <button
            className="rv-page-btn"
            disabled={page <= 1 || loading}
            onClick={() => setPage((p) => Math.max(1, p - 1))}
          >
            <ChevronLeft size={15} /> Prev
          </button>
          <span className="rv-page-info">
            Page{" "}
            <input
              className="rv-page-input"
              type="text"
              inputMode="numeric"
              value={pageInput}
              onChange={(e) => setPageInput(e.target.value.replace(/[^0-9]/g, ""))}
              onKeyDown={(e) => {
                if (e.key === "Enter") {
                  e.preventDefault();
                  commitPageInput();
                }
              }}
              onBlur={commitPageInput}
              onFocus={(e) => e.target.select()}
              title="Type a page number and press Enter to jump"
              aria-label="Page number — type and press Enter to jump"
            />{" "}
            of {totalPages.toLocaleString()}
          </span>
          <button
            className="rv-page-btn"
            disabled={page >= totalPages || loading}
            onClick={() => setPage((p) => p + 1)}
          >
            Next <ChevronRight size={15} />
          </button>
        </div>
        )}
      </div>
    </div>
      {minimized && (
        <button
          className="rv-restore-pill"
          onClick={onRestore}
          title="Resume browsing RhythmVerse where you left off"
        >
          <span className="rv-restore-icon"><Music size={15} /></span>
          RhythmVerse
          {dlProgress.size > 0 && (
            <span
              className="rv-restore-badge"
              title={`${dlProgress.size} download${
                dlProgress.size !== 1 ? "s" : ""
              } in progress`}
            >
              {dlProgress.size}
            </span>
          )}
        </button>
      )}
    </>
  );
}
