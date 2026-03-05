import { useState, useCallback } from "react";
import { invoke } from "@tauri-apps/api/core";
import type { SongSummary, SongDetails, SongMetadata } from "../types";

export function useSongFiles() {
  const [songs, setSongs] = useState<SongSummary[]>([]);
  const [selectedPath, setSelectedPath] = useState<string | null>(null);
  const [details, setDetails] = useState<SongDetails | null>(null);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [modifiedFields, setModifiedFields] = useState<Set<string>>(new Set());
  const [saving, setSaving] = useState(false);
  const [folderPath, setFolderPath] = useState<string>("");
  const [detailsCache, setDetailsCache] = useState<Map<string, SongDetails>>(new Map());
  const [albumArt, setAlbumArt] = useState<string>("");

  const openFolder = useCallback(async (path: string) => {
    setLoading(true);
    setError(null);
    setFolderPath(path);
    setDetailsCache(new Map());
    try {
      const result = await invoke<SongSummary[]>("open_folder", { path });
      setSongs(result);
      setSelectedPath(null);
      setDetails(null);
      setModifiedFields(new Set());
    } catch (e) {
      setError(String(e));
    } finally {
      setLoading(false);
    }
  }, []);

  const selectSong = useCallback(async (path: string) => {
    if (path === selectedPath) return;
    setSelectedPath(path);
    setModifiedFields(new Set());
    setError(null);

    // Check cache
    const cached = detailsCache.get(path);
    if (cached) {
      setDetails(cached);
      // Still fetch album art
      setAlbumArt("");
      invoke<string>("get_album_art", { path })
        .then(art => setAlbumArt(art))
        .catch(() => setAlbumArt(""));
      return;
    }

    setLoading(true);
    setAlbumArt("");
    try {
      const result = await invoke<SongDetails>("get_song_details", { path });
      // Remap ugc_plus (RBN2) and empty origins to C3 Customs
      if (!result.metadata.game_origin || result.metadata.game_origin === "ugc_plus") {
        result.metadata = { ...result.metadata, game_origin: "c3customs" };
        setModifiedFields(prev => new Set(prev).add("game_origin"));
      }
      setDetails(result);
      setDetailsCache(prev => new Map(prev).set(path, result));
      // Fetch high-res album art in background
      invoke<string>("get_album_art", { path })
        .then(art => setAlbumArt(art))
        .catch(() => setAlbumArt(""));
    } catch (e) {
      setError(String(e));
      setDetails(null);
    } finally {
      setLoading(false);
    }
  }, [selectedPath, detailsCache]);

  const updateMetadata = useCallback((field: string, value: string | number | null) => {
    if (!details) return;
    const newMeta = { ...details.metadata, [field]: value } as SongMetadata;
    setDetails({ ...details, metadata: newMeta });
    setModifiedFields(prev => new Set(prev).add(field));
  }, [details]);

  const updateHeader = useCallback((field: "display_name" | "description", value: string) => {
    if (!details) return;
    setDetails({ ...details, [field]: value });
    setModifiedFields(prev => new Set(prev).add(field));
  }, [details]);

  const updateThumbnail = useCallback((base64: string) => {
    if (!details) return;
    setDetails({ ...details, thumbnail_base64: base64 });
    setModifiedFields(prev => new Set(prev).add("thumbnail"));
  }, [details]);

  const saveSong = useCallback(async () => {
    if (!details) return;
    setSaving(true);
    setError(null);
    try {
      const hasHeaderChanges = modifiedFields.has("display_name") || modifiedFields.has("description");
      const hasMetaChanges = [...modifiedFields].some(f =>
        f !== "display_name" && f !== "description" && f !== "thumbnail"
      );
      const hasThumbnailChange = modifiedFields.has("thumbnail");

      // When saving with no tracked changes, still send metadata to persist defaults like C3 icon
      const sendMeta = hasMetaChanges || modifiedFields.size === 0;

      await invoke("save_song", {
        path: details.path,
        displayName: hasHeaderChanges ? details.display_name : null,
        description: hasHeaderChanges ? details.description : null,
        metadata: sendMeta ? details.metadata : null,
        thumbnailBase64: hasThumbnailChange ? details.thumbnail_base64 : null,
      });

      // Update cache
      setDetailsCache(prev => new Map(prev).set(details.path, details));

      // Update song list entry
      setSongs(prev => prev.map(s =>
        s.path === details.path
          ? { ...s, display_name: details.display_name, description: details.description }
          : s
      ));

      setModifiedFields(new Set());
    } catch (e) {
      setError(String(e));
    } finally {
      setSaving(false);
    }
  }, [details, modifiedFields]);

  return {
    songs,
    selectedPath,
    details,
    loading,
    error,
    modifiedFields,
    saving,
    folderPath,
    albumArt,
    openFolder,
    selectSong,
    updateMetadata,
    updateHeader,
    updateThumbnail,
    saveSong,
    setError,
  };
}
