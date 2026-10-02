import React, { useState, useEffect } from "react";
import { invoke } from "@tauri-apps/api/core";
import { BROKEN_FIELD } from "../types";
import type { SongDetails, ValidationIssue } from "../types";
import { ChartPreviewModal } from "./ChartPreviewModal";
import { ImageEditor } from "./ImageEditor";
import { IconSelector } from "./IconSelector";
import { SongScores } from "./SongScores";
import { DifficultyRing, TIER_LABELS } from "./DifficultyRing";
import { ChartNoAxesColumn, FolderOpen, Link2, Music, Save, Trash2 } from "lucide-react";

interface MetadataEditorProps {
  details: SongDetails;
  albumArtBase64: string;
  onUpdateMeta: (field: string, value: string | number | null) => void;
  onUpdateHeader: (field: "display_name" | "description", value: string) => void;
  onUpdateThumbnail: (base64: string) => void;
  onSave: () => void;
  // Resolves false if the drive has no Recycle Bin and nothing was deleted.
  onDelete: (path: string, permanent?: boolean) => Promise<boolean>;
  hasChanges: boolean;
  saving: boolean;
  // Re-download a broken song (missing chart/audio) through the chart browser.
  onFixBroken?: () => void;
}

const GENRES = [
  "alternative", "blues", "classic", "classicrock", "country", "emo",
  "fusion", "glam", "grunge", "hiphoprap", "indierock", "inspirational",
  "jazz", "jrock", "latin", "metal", "new_wave", "novelty", "numetal",
  "other", "pop", "poprock", "prog", "punk", "rb", "reggaeska",
  "rock", "southernrock", "urban", "world",
];

const RATINGS = [
  { value: 1, label: "Family Friendly" },
  { value: 2, label: "Supervision Recommended" },
  { value: 3, label: "Mature Content" },
];

const VOCAL_GENDERS = ["male", "female"];

// Pull a RhythmVerse file_id out of a pasted link or raw token. Accepts a full
// rhythmverse.co/download/… or /songfile/… URL (takes the id segment) or the
// bare id. IDs are hex, sometimes with a legacy ".digits" suffix — validate
// loosely but reject anything that isn't id-shaped so typos don't get stored.
function extractRvFileId(input: string): string | null {
  const s = input.trim();
  if (!s) return null;
  const m = s.match(/(?:download|songfile)\/([^/?#\s]+)/i);
  const token = (m ? m[1] : s).trim();
  return /^[0-9a-fA-F]+(\.[0-9]+)?$/.test(token) ? token : null;
}

// DTA rank value thresholds per instrument → tier (1-7)
const TIER_THRESHOLDS: Record<string, number[]> = {
  rank_drum:        [0, 1, 133, 169, 208, 294, 349, 401],
  rank_guitar:      [0, 1, 145, 194, 247, 301, 354, 406],
  rank_bass:        [0, 1, 166, 220, 259, 298, 349, 401],
  rank_vocals:      [0, 1, 139, 180, 220, 259, 298, 373],
  rank_keys:        [0, 1, 133, 169, 208, 294, 349, 401], // same as drums (TBD)
  rank_band:        [0, 1, 159, 219, 274, 328, 383, 454],
  rank_real_guitar: [0, 1, 145, 194, 247, 301, 354, 406],
  rank_real_bass:   [0, 1, 166, 220, 259, 298, 349, 401],
  rank_real_keys:   [0, 1, 133, 169, 208, 294, 349, 401],
};

function rankToTier(field: string, value: number | null | undefined): number {
  if (value == null || value <= 0) return 0;
  const thresholds = TIER_THRESHOLDS[field] || TIER_THRESHOLDS.rank_drum;
  let tier = 0;
  for (let i = 1; i < thresholds.length; i++) {
    if (value >= thresholds[i]) tier = i;
  }
  return tier;
}

// Convert a stored difficulty value to a 0-7 tier for the ring, picking the
// right scale. CON/DTA values are Rock Band ranks (0-400+) → threshold-mapped.
// Folder (song.ini) values are natively 0-6 YARG tiers, so a small value IS the
// tier — but some charts (usually RB3 conversions) leave rank-scale numbers in
// song.ini, so a large folder value still gets threshold-mapped. The 7 cutoff
// works because real YARG tiers never exceed 6 and real ranks are ~100+.
function difficultyTier(
  field: string,
  value: number | null | undefined,
  isFolder: boolean
): number {
  if (value == null || value <= 0) return 0;
  if (isFolder && value <= 7) return Math.min(7, value);
  return rankToTier(field, value);
}

function Field({
  label,
  value,
  onChange,
  type = "text",
  className = "",
}: {
  label: string;
  value: string | number;
  onChange: (val: string) => void;
  type?: string;
  className?: string;
}) {
  return (
    <div className={`field ${className}`}>
      <label>{label}</label>
      <input
        type={type}
        value={value ?? ""}
        onChange={(e) => onChange(e.target.value)}
      />
    </div>
  );
}

function SelectField({
  label,
  value,
  options,
  onChange,
}: {
  label: string;
  value: string | number;
  options: { value: string | number; label: string }[];
  onChange: (val: string) => void;
}) {
  return (
    <div className="field">
      <label>{label}</label>
      <select value={value ?? ""} onChange={(e) => onChange(e.target.value)}>
        <option value="">--</option>
        {options.map((o) => (
          <option key={o.value} value={o.value}>
            {o.label}
          </option>
        ))}
      </select>
    </div>
  );
}

export function MetadataEditor({
  details,
  albumArtBase64,
  onUpdateMeta,
  onUpdateHeader,
  onUpdateThumbnail,
  onSave,
  onDelete,
  hasChanges,
  saving,
  onFixBroken,
}: MetadataEditorProps) {
  const [showChart, setShowChart] = useState(false);
  const [confirmDelete, setConfirmDelete] = useState(false);
  const [deleting, setDeleting] = useState(false);
  const [deleteError, setDeleteError] = useState<string | null>(null);
  // Set once the Recycle Bin refuses the file; the next confirm deletes permanently.
  const [needsPermanent, setNeedsPermanent] = useState(false);
  // RhythmVerse link (kept in YARGLE's own DB, keyed by this song's path — not
  // written into the chart, and separate from the metadata Save).
  const [rvLinkInput, setRvLinkInput] = useState("");
  const [rvLinkedId, setRvLinkedId] = useState<string | null>(null);
  const [rvLinkBusy, setRvLinkBusy] = useState(false);
  const [rvLinkError, setRvLinkError] = useState<string | null>(null);
  const m = details.metadata;

  const numOrNull = (val: string): number | null => {
    if (val === "") return null;
    const n = parseInt(val, 10);
    return isNaN(n) ? null : n;
  };

  // A pending delete confirm belongs to the song it was opened on — never let a
  // "Delete Permanently" carry over to the next selection.
  useEffect(() => {
    setConfirmDelete(false);
    setDeleting(false);
    setDeleteError(null);
    setNeedsPermanent(false);
  }, [details.path]);

  // Load the current link whenever the selected song changes.
  useEffect(() => {
    setRvLinkInput("");
    setRvLinkError(null);
    setRvLinkedId(null);
    invoke<string | null>("rv_linked_file_id", { path: details.path })
      .then((id) => setRvLinkedId(id))
      .catch(() => {});
  }, [details.path]);

  const saveRvLink = () => {
    const fileId = extractRvFileId(rvLinkInput);
    if (!fileId) {
      setRvLinkError("Enter a RhythmVerse file ID or a rhythmverse.co link.");
      return;
    }
    setRvLinkBusy(true);
    setRvLinkError(null);
    invoke("rv_link_song", {
      fileId,
      destPath: details.path,
      songId: null,
      artist: m.artist,
      title: m.name,
      fileName: null,
      uploaded: null,
    })
      .then(() => {
        setRvLinkedId(fileId);
        setRvLinkInput("");
      })
      .catch((e) => setRvLinkError(String(e)))
      .finally(() => setRvLinkBusy(false));
  };

  const clearRvLink = () => {
    setRvLinkBusy(true);
    setRvLinkError(null);
    invoke("rv_unlink_song", { destPath: details.path })
      .then(() => setRvLinkedId(null))
      .catch((e) => setRvLinkError(String(e)))
      .finally(() => setRvLinkBusy(false));
  };

  const heroArt = albumArtBase64 || details.thumbnail_base64;
  const heroSub = [m.artist, m.album_name, m.year_released ? String(m.year_released) : ""]
    .filter(Boolean)
    .join(" · ");

  return (
    <div className="metadata-editor">
      {/* Album-art hero (YARG song sidebar): blurred art backdrop fading into
          the panel, the crisp art, then title/artist and the art actions. */}
      <div className="editor-hero">
        <div
          className="editor-hero-backdrop"
          style={heroArt ? { backgroundImage: `url(${heroArt})` } : undefined}
        />
        <div className="editor-header-buttons">
          <button
            className="chart-btn"
            onClick={() => {
              invoke("reveal_in_explorer", { path: details.path }).catch(() => {});
            }}
            title={
              details.is_folder
                ? "Open this song's folder in File Explorer"
                : "Show this file in File Explorer"
            }
          >
            <FolderOpen size={15} /> Explorer
          </button>
          <button
            className="chart-btn"
            onClick={() => setShowChart(true)}
          >
            <ChartNoAxesColumn size={15} /> Chart
          </button>
          <button
            className={`save-btn ${hasChanges ? "has-changes" : ""}`}
            onClick={onSave}
            disabled={saving}
          >
            <Save size={15} />
            {saving ? "Saving..." : hasChanges ? "Save Changes" : "Save"}
          </button>
          <button
            className="delete-song-btn"
            onClick={() => setConfirmDelete(true)}
            disabled={deleting}
          >
            <Trash2 size={15} /> Delete
          </button>
        </div>
        <div className="editor-hero-main">
          <div className="editor-hero-art">
            {heroArt ? <img src={heroArt} alt="Album art" /> : <Music size={44} />}
          </div>
          <div className="editor-hero-text">
            <h2>{m.name || details.display_name || "Untitled"}</h2>
            {heroSub && <div className="editor-hero-sub">{heroSub}</div>}
            <ImageEditor
              buttonsOnly
              thumbnailBase64={details.thumbnail_base64}
              albumArtBase64={albumArtBase64}
              songPath={details.path}
              onReplace={onUpdateThumbnail}
              artist={m.artist}
              albumName={m.album_name}
            />
          </div>
        </div>
      </div>
        {confirmDelete && (
          <div className="delete-confirm-bar">
            <span>
              {needsPermanent
                ? "This drive has no Recycle Bin. Delete permanently? This cannot be undone."
                : "Move this song to the Recycle Bin?"}
            </span>
            {deleteError && <span className="delete-error">{deleteError}</span>}
            <div className="delete-confirm-btns">
              <button
                className="delete-confirm-yes"
                disabled={deleting}
                onClick={async () => {
                  setDeleting(true);
                  setDeleteError(null);
                  try {
                    const deleted = await onDelete(details.path, needsPermanent);
                    if (!deleted) {
                      setNeedsPermanent(true);
                      setDeleting(false);
                    }
                  } catch (e) {
                    setDeleteError(String(e));
                    setDeleting(false);
                  }
                }}
              >
                {deleting
                  ? "Deleting..."
                  : needsPermanent
                    ? "Delete Permanently"
                    : "Move to Recycle Bin"}
              </button>
              <button
                className="delete-confirm-no"
                disabled={deleting}
                onClick={() => {
                  setConfirmDelete(false);
                  setDeleteError(null);
                  setNeedsPermanent(false);
                }}
              >
                Cancel
              </button>
            </div>
          </div>
        )}

      <div className="editor-content">
        <div className="editor-main">
          <section>
            <h3>Header Info</h3>
            <Field
              label="Display Name"
              value={details.display_name}
              onChange={(v) => onUpdateHeader("display_name", v)}
            />
            <Field
              label="Description"
              value={details.description}
              onChange={(v) => onUpdateHeader("description", v)}
            />
          </section>

          <section>
            <h3>Song Info</h3>
            <Field
              label="Title"
              value={m.name}
              onChange={(v) => onUpdateMeta("name", v)}
            />
            <Field
              label="Artist"
              value={m.artist}
              onChange={(v) => onUpdateMeta("artist", v)}
            />
            <Field
              label="Album"
              value={m.album_name}
              onChange={(v) => onUpdateMeta("album_name", v)}
            />
            <div className="field-row">
              <Field
                label="Track #"
                value={m.album_track_number ?? ""}
                onChange={(v) => onUpdateMeta("album_track_number", numOrNull(v))}
                type="number"
              />
              <Field
                label="Year"
                value={m.year_released ?? ""}
                onChange={(v) => onUpdateMeta("year_released", numOrNull(v))}
                type="number"
              />
            </div>
            <Field
              label="Author"
              value={m.author}
              onChange={(v) => onUpdateMeta("author", v)}
            />
          </section>

          <section>
            <h3>Classification</h3>
            <SelectField
              label="Genre"
              value={m.genre}
              options={GENRES.map((g) => ({ value: g, label: g }))}
              onChange={(v) => onUpdateMeta("genre", v)}
            />
            <Field
              label="Sub-genre"
              value={m.sub_genre}
              onChange={(v) => onUpdateMeta("sub_genre", v)}
            />
            <SelectField
              label="Vocal Gender"
              value={m.vocal_gender}
              options={VOCAL_GENDERS.map((g) => ({ value: g, label: g }))}
              onChange={(v) => onUpdateMeta("vocal_gender", v)}
            />
            <SelectField
              label="Rating"
              value={m.rating ?? ""}
              options={RATINGS.map((r) => ({ value: r.value, label: r.label }))}
              onChange={(v) => onUpdateMeta("rating", numOrNull(v))}
            />
            <IconSelector
              value={m.game_origin}
              onChange={(v) => onUpdateMeta("game_origin", v)}
            />
          </section>

          <section>
            <h3>Difficulty Rankings</h3>
            <div className="rank-grid">
              {([
                ["Drums", "rank_drum", "drums"],
                ["Guitar", "rank_guitar", "guitar"],
                ["Bass", "rank_bass", "bass"],
                ["Vocals", "rank_vocals", "vocals"],
                ["Keys", "rank_keys", "keys"],
                ["Band", "rank_band", "band"],
                ["Pro Guitar", "rank_real_guitar", "realGuitar"],
                ["Pro Bass", "rank_real_bass", "realBass"],
                ["Pro Keys", "rank_real_keys", "realKeys"],
              ] as const).map(([label, field, instrument]) => {
                const rawVal = (m as any)[field] as number | null | undefined;
                const tier = difficultyTier(field, rawVal, details.is_folder);
                return (
                  <div key={field} className="rank-field-with-ring">
                    <div className="rank-ring-container" title={TIER_LABELS[tier]}>
                      <DifficultyRing tier={tier} instrument={instrument} />
                    </div>
                    <div className="rank-input-wrap">
                      <label>{label}</label>
                      <input
                        type="number"
                        value={rawVal ?? ""}
                        onChange={(e) => onUpdateMeta(field, numOrNull(e.target.value))}
                      />
                      <span className="rank-tier-label">{TIER_LABELS[tier]}</span>
                    </div>
                  </div>
                );
              })}
            </div>
          </section>

          <section>
            <h3>Technical</h3>
            <Field
              label="Song ID"
              value={m.song_id ?? ""}
              onChange={(v) => onUpdateMeta("song_id", numOrNull(v))}
              type="number"
            />
            <div className="field-row">
              <Field
                label="Preview Start"
                value={m.preview_start ?? ""}
                onChange={(v) => onUpdateMeta("preview_start", numOrNull(v))}
                type="number"
              />
              <Field
                label="Preview End"
                value={m.preview_end ?? ""}
                onChange={(v) => onUpdateMeta("preview_end", numOrNull(v))}
                type="number"
              />
            </div>
            <Field
              label="Song Length (ms)"
              value={m.song_length ?? ""}
              onChange={(v) => onUpdateMeta("song_length", numOrNull(v))}
              type="number"
            />
            <Field
              label="Shortname"
              value={m.shortname}
              onChange={(v) => onUpdateMeta("shortname", v)}
            />
            <div className="field rv-link-field">
              <label>RhythmVerse Link</label>
              {rvLinkedId ? (
                <div className="rv-link-current">
                  <span
                    className="rv-link-badge"
                    title="Linked to a RhythmVerse file — the browser shows this song as In library (exact) and can flag updates."
                  >
                    <Link2 size={13} /> Linked: {rvLinkedId}
                  </span>
                  <button
                    className="rv-link-clear"
                    disabled={rvLinkBusy}
                    onClick={clearRvLink}
                  >
                    Unlink
                  </button>
                </div>
              ) : (
                <div className="rv-link-current">
                  <input
                    type="text"
                    placeholder="rhythmverse.co/download/… or file ID"
                    value={rvLinkInput}
                    onChange={(e) => setRvLinkInput(e.target.value)}
                    onKeyDown={(e) => {
                      if (e.key === "Enter") saveRvLink();
                    }}
                  />
                  <button
                    className="rv-link-save"
                    disabled={rvLinkBusy || !rvLinkInput.trim()}
                    onClick={saveRvLink}
                  >
                    Link
                  </button>
                </div>
              )}
              <span className="rv-link-hint">
                Ties this song to its RhythmVerse entry by file ID, so the browser
                marks it “In library” and can detect updates — handy for songs you
                downloaded from an external host (Google Drive, etc.).
              </span>
              {rvLinkError && <span className="rv-link-error">{rvLinkError}</span>}
            </div>
          </section>

          {details.validation_issues && details.validation_issues.length > 0 && (
            <section className="validation-section">
              <h3>Validation ({details.validation_issues.length} issues)</h3>
              <div className="validation-box">
                {details.validation_issues.map((issue: ValidationIssue, i: number) => (
                  <div key={i} className={`validation-row validation-${issue.level.toLowerCase()}`}>
                    <span className="validation-icon">
                      {issue.level === "Error" ? "\u2716" : issue.level === "Warning" ? "\u26A0" : "\u2139"}
                    </span>
                    <span className="validation-message">{issue.message}</span>
                    {issue.field === BROKEN_FIELD && onFixBroken && (
                      <button
                        className="validator-fix-btn"
                        onClick={onFixBroken}
                        title="Find this song in the chart browser; the download replaces this broken copy"
                      >
                        Fix it
                      </button>
                    )}
                  </div>
                ))}
              </div>
            </section>
          )}
        </div>

        <div className="editor-sidebar">
          <SongScores songName={m.name} artist={m.artist} />
        </div>
      </div>

      {showChart && (
        <ChartPreviewModal
          songPath={details.path}
          onClose={() => setShowChart(false)}
        />
      )}
    </div>
  );
}
