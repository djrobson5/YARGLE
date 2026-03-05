import React, { useState, useEffect, useCallback } from "react";
import { open } from "@tauri-apps/plugin-dialog";
import { SearchBar } from "./components/SearchBar";
import { FileList } from "./components/FileList";
import { MetadataEditor } from "./components/MetadataEditor";
import { ScoreSyncModal } from "./components/ScoreSyncModal";
import { useSongFiles } from "./hooks/useSongFiles";

function App() {
  const {
    songs,
    selectedPath,
    details,
    loading,
    error,
    modifiedFields,
    saving,
    albumArt,
    openFolder,
    selectSong,
    updateMetadata,
    updateHeader,
    updateThumbnail,
    saveSong,
    setError,
  } = useSongFiles();

  const [filter, setFilter] = useState("");
  const [showScoreSync, setShowScoreSync] = useState(false);

  const handleOpenFolder = useCallback(async () => {
    try {
      const selected = await open({ directory: true, multiple: false });
      if (selected) {
        openFolder(selected as string);
      }
    } catch (e) {
      setError(String(e));
    }
  }, [openFolder, setError]);

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
      <div className="left-panel">
        <SearchBar
          value={filter}
          onChange={setFilter}
          onOpenFolder={handleOpenFolder}
          onOpenOptions={() => setShowScoreSync(true)}
          songCount={songs.length}
        />
        <FileList
          songs={songs}
          selectedPath={selectedPath}
          filter={filter}
          onSelect={selectSong}
          modifiedPaths={new Set(
            modifiedFields.size > 0 && details ? [details.path] : []
          )}
        />
      </div>
      <div className="right-panel">
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
    </div>
  );
}

export default App;
