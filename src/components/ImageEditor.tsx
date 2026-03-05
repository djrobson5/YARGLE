import React, { useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";

interface ArtResult {
  url: string;
  source: string;
  thumbnail_url: string;
}

interface ImageEditorProps {
  thumbnailBase64: string;
  albumArtBase64: string;
  songPath: string;
  onReplace: (base64: string) => void;
  artist?: string;
  albumName?: string;
}

export function ImageEditor({
  thumbnailBase64,
  albumArtBase64,
  songPath,
  onReplace,
  artist,
  albumName,
}: ImageEditorProps) {
  const fileInputRef = useRef<HTMLInputElement>(null);
  const [showSearch, setShowSearch] = useState(false);
  const [searchArtist, setSearchArtist] = useState("");
  const [searchAlbum, setSearchAlbum] = useState("");
  const [candidates, setCandidates] = useState<ArtResult[]>([]);
  const [searching, setSearching] = useState(false);
  const [downloading, setDownloading] = useState<string | null>(null);
  const [searchError, setSearchError] = useState("");

  const displayImage = albumArtBase64 || thumbnailBase64;

  const handleFileSelect = (e: React.ChangeEvent<HTMLInputElement>) => {
    const file = e.target.files?.[0];
    if (!file) return;
    const reader = new FileReader();
    reader.onload = () => onReplace(reader.result as string);
    reader.readAsDataURL(file);
    e.target.value = "";
  };

  const openSearch = () => {
    setSearchArtist(artist || "");
    setSearchAlbum(albumName || "");
    setCandidates([]);
    setSearchError("");
    setShowSearch(true);
  };

  const doSearch = async () => {
    if (!searchArtist.trim() && !searchAlbum.trim()) return;
    setSearching(true);
    setSearchError("");
    setCandidates([]);
    try {
      const results = await invoke<ArtResult[]>("search_album_art", {
        artist: searchArtist.trim(),
        album: searchAlbum.trim(),
      });
      setCandidates(results);
      if (results.length === 0) {
        setSearchError("No album art found. Try different search terms.");
      }
    } catch (e: any) {
      setSearchError(e?.toString() || "Search failed");
    } finally {
      setSearching(false);
    }
  };

  const selectCandidate = async (art: ArtResult) => {
    setDownloading(art.url);
    try {
      const dataUrl = await invoke<string>("download_album_art", {
        url: art.url,
      });
      onReplace(dataUrl);
      setShowSearch(false);
    } catch (e: any) {
      setSearchError(e?.toString() || "Download failed");
    } finally {
      setDownloading(null);
    }
  };

  const handleSearchKeyDown = (e: React.KeyboardEvent) => {
    if (e.key === "Enter") doSearch();
  };

  return (
    <div className="image-editor">
      <h3>Album Art</h3>
      <div className="thumbnail-container">
        {displayImage ? (
          <img src={displayImage} alt="Album art" className="thumbnail-preview" />
        ) : (
          <div className="thumbnail-placeholder">No image</div>
        )}
      </div>
      <button
        className="replace-btn"
        onClick={() => fileInputRef.current?.click()}
      >
        Replace Image
      </button>
      <button className="replace-btn fetch-art-btn" onClick={openSearch}>
        Fetch Art
      </button>
      <input
        ref={fileInputRef}
        type="file"
        accept="image/png,image/jpeg,image/jpg"
        onChange={handleFileSelect}
        style={{ display: "none" }}
      />
      <p className="image-hint">Replaces header thumbnail (64x64)</p>

      {showSearch && (
        <div className="art-search-overlay" onClick={() => setShowSearch(false)}>
          <div className="art-search-panel" onClick={(e) => e.stopPropagation()}>
            <div className="art-search-header">
              <h3>Fetch Album Art</h3>
              <button className="art-search-close" onClick={() => setShowSearch(false)}>
                &times;
              </button>
            </div>
            <div className="art-search-form">
              <input
                type="text"
                placeholder="Artist"
                value={searchArtist}
                onChange={(e) => setSearchArtist(e.target.value)}
                onKeyDown={handleSearchKeyDown}
              />
              <input
                type="text"
                placeholder="Album"
                value={searchAlbum}
                onChange={(e) => setSearchAlbum(e.target.value)}
                onKeyDown={handleSearchKeyDown}
              />
              <button
                className="art-search-btn"
                onClick={doSearch}
                disabled={searching}
              >
                {searching ? "Searching..." : "Search"}
              </button>
            </div>

            {searching && (
              <div className="art-search-loading">
                <div className="art-spinner" />
                Searching iTunes & MusicBrainz...
              </div>
            )}

            {searchError && !searching && (
              <div className="art-search-error">{searchError}</div>
            )}

            {candidates.length > 0 && (
              <div className="art-search-grid">
                {candidates.map((art, i) => (
                  <div
                    key={`${art.source}-${i}`}
                    className={`art-candidate ${downloading === art.url ? "loading" : ""}`}
                    onClick={() => !downloading && selectCandidate(art)}
                  >
                    <img src={art.thumbnail_url} alt="candidate" />
                    <span className="art-source-badge">{art.source}</span>
                    {downloading === art.url && (
                      <div className="art-candidate-loading">
                        <div className="art-spinner" />
                      </div>
                    )}
                  </div>
                ))}
              </div>
            )}
          </div>
        </div>
      )}
    </div>
  );
}
