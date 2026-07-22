import React, { useState, useEffect, useCallback, useMemo, useRef } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { open } from "@tauri-apps/plugin-dialog";
import type { UpdateInfo } from "./types";
import { SearchBar } from "./components/SearchBar";
import { FileList } from "./components/FileList";
import { MetadataEditor } from "./components/MetadataEditor";
import { ScoreSyncModal } from "./components/ScoreSyncModal";
import { MoggDecryptModal } from "./components/MoggDecryptModal";
import { DuplicateModal } from "./components/DuplicateModal";
import { RenameModal } from "./components/RenameModal";
import { BatchEditModal } from "./components/BatchEditModal";
import { OrganizeModal } from "./components/OrganizeModal";
import { ValidatorModal } from "./components/ValidatorModal";
import { RhythmVerseModal } from "./components/RhythmVerseModal";
import { useSongFiles } from "./hooks/useSongFiles";

function App() {
  const {
    songs,
    selectedPath,
    details,
    loading,
    loadProgress,
    error,
    modifiedFields,
    saving,
    albumArt,
    multiSelected,
    openFolder,
    selectSong,
    toggleMultiSelect,
    clearMultiSelect,
    selectAllPaths,
    updateMetadata,
    updateHeader,
    updateThumbnail,
    saveSong,
    deleteSong,
    setError,
  } = useSongFiles();

  const [filter, setFilter] = useState("");
  const [gameOriginFilter, setGameOriginFilter] = useState<string | null>(null);
  const [showScoreSync, setShowScoreSync] = useState(false);
  const [showMoggDecrypt, setShowMoggDecrypt] = useState(false);
  const [showDuplicates, setShowDuplicates] = useState(false);
  const [showRename, setShowRename] = useState(false);
  const [showBatchEdit, setShowBatchEdit] = useState(false);
  const [showOrganize, setShowOrganize] = useState(false);
  const [showValidator, setShowValidator] = useState(false);
  const [showRhythmVerse, setShowRhythmVerse] = useState(false);
  const [currentFolder, setCurrentFolder] = useState<string | null>(null);
  const [update, setUpdate] = useState<UpdateInfo | null>(null);
  // Non-null while a one-click self-update is in flight.
  const [updating, setUpdating] = useState<{ phase: string; pct: number } | null>(
    null
  );

  // One quiet update check per launch; failures/absence stay silent.
  useEffect(() => {
    invoke<UpdateInfo | null>("check_for_update")
      .then((info) => setUpdate(info))
      .catch(() => {});
  }, []);

  // Progress of an in-flight self-update (the app relaunches on completion).
  useEffect(() => {
    const un = listen<{ phase: string; received: number; total: number }>(
      "update-progress",
      (e) => {
        const p = e.payload;
        const pct = p.total > 0 ? Math.round((p.received / p.total) * 100) : 0;
        setUpdating({ phase: p.phase, pct });
      }
    );
    return () => {
      un.then((fn) => fn());
    };
  }, []);

  const handleSelfUpdate = () => {
    if (!update?.download_url) return;
    setUpdating({ phase: "starting", pct: 0 });
    invoke("download_and_apply_update", { url: update.download_url }).catch((e) => {
      setError(String(e));
      setUpdating(null);
    });
  };

  // Resizable left pane: width persists across sessions, clamped so the
  // right panel always keeps a usable minimum.
  const clampLeftWidth = useCallback((w: number) => {
    const max = Math.max(280, window.innerWidth - 520);
    return Math.min(Math.max(w, 280), max);
  }, []);
  const [leftWidth, setLeftWidth] = useState<number>(() => {
    const saved = parseInt(localStorage.getItem("yargle-left-width") || "", 10);
    return isNaN(saved) ? 320 : saved;
  });
  const leftWidthRef = useRef(leftWidth);
  leftWidthRef.current = leftWidth;

  // Re-clamp when the window itself resizes (e.g. fullscreen toggle)
  useEffect(() => {
    const onResize = () => setLeftWidth((w) => clampLeftWidth(w));
    window.addEventListener("resize", onResize);
    return () => window.removeEventListener("resize", onResize);
  }, [clampLeftWidth]);

  const handleDividerMouseDown = useCallback(
    (e: React.MouseEvent) => {
      e.preventDefault();
      const startX = e.clientX;
      const startWidth = leftWidthRef.current;
      let last = startWidth;
      document.body.style.cursor = "col-resize";
      document.body.style.userSelect = "none";
      const onMove = (ev: MouseEvent) => {
        last = clampLeftWidth(startWidth + (ev.clientX - startX));
        setLeftWidth(last);
      };
      const onUp = () => {
        document.body.style.cursor = "";
        document.body.style.userSelect = "";
        window.removeEventListener("mousemove", onMove);
        window.removeEventListener("mouseup", onUp);
        localStorage.setItem("yargle-left-width", String(last));
      };
      window.addEventListener("mousemove", onMove);
      window.addEventListener("mouseup", onUp);
    },
    [clampLeftWidth]
  );

  const handleOpenFolder = useCallback(async () => {
    try {
      const selected = await open({ directory: true, multiple: false });
      if (selected) {
        setCurrentFolder(selected as string);
        openFolder(selected as string);
      }
    } catch (e) {
      setError(String(e));
    }
  }, [openFolder, setError]);

  // Compute filtered songs (mirrors FileList filtering logic)
  const filteredSongs = useMemo(() => {
    let result = songs;
    if (gameOriginFilter) {
      result = result.filter((s) => {
        const origin = s.game_origin || "";
        const normalized = (!origin || origin === "ugc_plus") ? "c3customs" : origin;
        return normalized === gameOriginFilter;
      });
    }
    if (filter) {
      const lower = filter.toLowerCase();
      result = result.filter(
        (s) =>
          s.display_name.toLowerCase().includes(lower) ||
          s.description.toLowerCase().includes(lower) ||
          s.album_name.toLowerCase().includes(lower) ||
          s.author.toLowerCase().includes(lower)
      );
    }
    return result;
  }, [songs, filter, gameOriginFilter]);

  const handleSelectAllVisible = useCallback(() => {
    selectAllPaths(filteredSongs.map((s) => s.path));
  }, [filteredSongs, selectAllPaths]);

  // Keyboard shortcuts
  useEffect(() => {
    const handler = (e: KeyboardEvent) => {
      if (e.ctrlKey && e.key === "s") {
        e.preventDefault();
        saveSong();
      }
    };
    window.addEventListener("keydown", handler);
    return () => window.removeEventListener("keydown", handler);
  }, [saveSong]);

  return (
    <div className="app">
      <div className="left-panel" style={{ width: leftWidth }}>
        <SearchBar
          value={filter}
          onChange={setFilter}
          onOpenFolder={handleOpenFolder}
          onOpenOptions={() => setShowScoreSync(true)}
          onDecryptMoggs={() => setShowMoggDecrypt(true)}
          onFindDuplicates={() => setShowDuplicates(true)}
          onBatchRename={() => setShowRename(true)}
          onBatchEdit={() => setShowBatchEdit(true)}
          onOrganize={() => setShowOrganize(true)}
          onValidate={() => setShowValidator(true)}
          onBrowseRhythmVerse={() => setShowRhythmVerse(true)}
          songCount={songs.length}
          songs={songs}
          gameOriginFilter={gameOriginFilter}
          onGameOriginFilter={setGameOriginFilter}
          multiSelectedCount={multiSelected.size}
          onClearMultiSelect={clearMultiSelect}
          onSelectAllVisible={handleSelectAllVisible}
          filteredCount={filteredSongs.length}
        />
        {loading && loadProgress && loadProgress.total > 0 && (
          <div className="folder-load-progress">
            <div className="mogg-decrypt-bar-outer">
              <div
                className="mogg-decrypt-bar-inner"
                style={{ width: `${Math.round((loadProgress.current / loadProgress.total) * 100)}%` }}
              />
            </div>
            <div className="folder-load-status">
              Loading songs... {loadProgress.current.toLocaleString()} / {loadProgress.total.toLocaleString()}
            </div>
          </div>
        )}
        <FileList
          songs={songs}
          selectedPath={selectedPath}
          filter={filter}
          gameOriginFilter={gameOriginFilter}
          onSelect={selectSong}
          modifiedPaths={new Set(
            modifiedFields.size > 0 && details ? [details.path] : []
          )}
          multiSelected={multiSelected}
          onToggleMultiSelect={toggleMultiSelect}
        />
      </div>
      <div
        className="pane-divider"
        onMouseDown={handleDividerMouseDown}
        title="Drag to resize"
      />
      <div className="right-panel">
        {update && (
          <div className="update-banner">
            <span>YARGLE v{update.version} is available</span>
            <div className="update-banner-actions">
              {updating ? (
                <span className="update-banner-progress">
                  {updating.phase === "downloading"
                    ? `Downloading… ${updating.pct}%`
                    : updating.phase === "installing"
                    ? "Installing…"
                    : updating.phase === "restarting"
                    ? "Restarting…"
                    : "Starting…"}
                </span>
              ) : (
                <>
                  {update.download_url && (
                    <button
                      className="update-banner-update"
                      onClick={handleSelfUpdate}
                      title="Download and install the new version, then restart"
                    >
                      Update now
                    </button>
                  )}
                  <button
                    className="update-banner-view"
                    onClick={() =>
                      invoke("rv_open_external", { url: update.url }).catch(() => {})
                    }
                  >
                    View release
                  </button>
                  <button
                    className="update-banner-dismiss"
                    onClick={() => setUpdate(null)}
                    title="Dismiss"
                  >
                    &times;
                  </button>
                </>
              )}
            </div>
          </div>
        )}
        {error && (
          <div className="error-bar">
            <span>{error}</span>
            <button onClick={() => setError(null)}>Dismiss</button>
          </div>
        )}
        {loading && !details && (
          <div className="loading">Loading...</div>
        )}
        {details && (
          <MetadataEditor
            details={details}
            albumArtBase64={albumArt}
            onUpdateMeta={updateMetadata}
            onUpdateHeader={updateHeader}
            onUpdateThumbnail={updateThumbnail}
            onSave={saveSong}
            onDelete={deleteSong}
            hasChanges={modifiedFields.size > 0}
            saving={saving}
          />
        )}
        {!details && !loading && (
          <div className="empty-state">
            <h2>YARGLE</h2>
            <p>YARG Song Metadata Editor</p>
            <p className="hint">
              {songs.length > 0
                ? "Select a song from the list to edit its metadata"
                : "Open a folder containing CON/STFS files to get started"}
            </p>
          </div>
        )}
      </div>
      {showScoreSync && (
        <ScoreSyncModal onClose={() => setShowScoreSync(false)} />
      )}
      {showMoggDecrypt && (
        <MoggDecryptModal
          paths={songs.map((s) => s.path)}
          onClose={() => setShowMoggDecrypt(false)}
        />
      )}
      {showDuplicates && (
        <DuplicateModal
          paths={songs.map((s) => s.path)}
          onClose={(deleted) => {
            setShowDuplicates(false);
            if (deleted && currentFolder) {
              openFolder(currentFolder);
            }
          }}
        />
      )}
      {showRename && (
        <RenameModal
          paths={songs.map((s) => s.path)}
          onClose={(renamed) => {
            setShowRename(false);
            if (renamed && currentFolder) {
              openFolder(currentFolder);
            }
          }}
        />
      )}
      {showOrganize && currentFolder && (
        <OrganizeModal
          paths={songs.map((s) => s.path)}
          currentFolder={currentFolder}
          onClose={(organized) => {
            setShowOrganize(false);
            if (organized && currentFolder) {
              openFolder(currentFolder);
            }
          }}
        />
      )}
      {showValidator && (
        <ValidatorModal
          paths={songs.map((s) => s.path)}
          onClose={() => setShowValidator(false)}
        />
      )}
      {showRhythmVerse && (
        <RhythmVerseModal
          librarySongs={songs}
          libraryFolder={currentFolder}
          onLibraryChanged={() => {
            if (currentFolder) openFolder(currentFolder);
          }}
          onClose={() => setShowRhythmVerse(false)}
        />
      )}
      {showBatchEdit && (
        <BatchEditModal
          paths={multiSelected.size > 0
            ? songs.filter(s => multiSelected.has(s.path)).map(s => s.path)
            : songs.map((s) => s.path)
          }
          isSelection={multiSelected.size > 0}
          onClose={(edited) => {
            setShowBatchEdit(false);
            if (edited && currentFolder) {
              openFolder(currentFolder);
            }
          }}
        />
      )}
    </div>
  );
}

export default App;
