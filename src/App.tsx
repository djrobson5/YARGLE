import React, { useState, useEffect, useCallback } from "react";
import { open } from "@tauri-apps/plugin-dialog";
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
  const [currentFolder, setCurrentFolder] = useState<string | null>(null);

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
          onDecryptMoggs={() => setShowMoggDecrypt(true)}
          onFindDuplicates={() => setShowDuplicates(true)}
          onBatchRename={() => setShowRename(true)}
          onBatchEdit={() => setShowBatchEdit(true)}
          onOrganize={() => setShowOrganize(true)}
          onValidate={() => setShowValidator(true)}
          songCount={songs.length}
          songs={songs}
          gameOriginFilter={gameOriginFilter}
          onGameOriginFilter={setGameOriginFilter}
        />
        <FileList
          songs={songs}
          selectedPath={selectedPath}
          filter={filter}
          gameOriginFilter={gameOriginFilter}
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
      {showBatchEdit && (
        <BatchEditModal
          paths={songs.map((s) => s.path)}
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
