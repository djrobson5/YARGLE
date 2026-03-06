import React, { useState } from "react";
import type { SongDetails, ValidationIssue } from "../types";
import { ChartPreviewModal } from "./ChartPreviewModal";
import { ImageEditor } from "./ImageEditor";
import { IconSelector } from "./IconSelector";
import { SongScores } from "./SongScores";

interface MetadataEditorProps {
  details: SongDetails;
  albumArtBase64: string;
  onUpdateMeta: (field: string, value: string | number | null) => void;
  onUpdateHeader: (field: "display_name" | "description", value: string) => void;
  onUpdateThumbnail: (base64: string) => void;
  onSave: () => void;
  onDelete: (path: string) => Promise<void>;
  hasChanges: boolean;
  saving: boolean;
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

const TIER_LABELS = [
  "No Part", "Warmup", "Apprentice", "Solid",
  "Moderate", "Challenging", "Nightmare", "Impossible",
];

function rankToTier(field: string, value: number | null | undefined): number {
  if (value == null || value <= 0) return 0;
  const thresholds = TIER_THRESHOLDS[field] || TIER_THRESHOLDS.rank_drum;
  let tier = 0;
  for (let i = 1; i < thresholds.length; i++) {
    if (value >= thresholds[i]) tier = i;
  }
  return tier;
}

function DifficultyRing({ tier }: { tier: number }) {
  const size = 44;
  const cx = size / 2;
  const cy = size / 2;
  const r = 17;
  const segments = 7;
  const gapDeg = 18;
  const segmentAngle = (360 - segments * gapDeg) / segments;
  const strokeW = 5;

  const arcPath = (i: number) => {
    const startDeg = -90 + i * (segmentAngle + gapDeg);
    const endDeg = startDeg + segmentAngle;
    const startRad = (startDeg * Math.PI) / 180;
    const endRad = (endDeg * Math.PI) / 180;
    const x1 = cx + r * Math.cos(startRad);
    const y1 = cy + r * Math.sin(startRad);
    const x2 = cx + r * Math.cos(endRad);
    const y2 = cy + r * Math.sin(endRad);
    return `M ${x1} ${y1} A ${r} ${r} 0 0 1 ${x2} ${y2}`;
  };

  return (
    <svg width={size} height={size} viewBox={`0 0 ${size} ${size}`}>
      {/* Empty track segments — visible dark slots */}
      {Array.from({ length: segments }, (_, i) => (
        <path
          key={`bg-${i}`}
          d={arcPath(i)}
          fill="none"
          stroke="#3a3a5c"
          strokeWidth={strokeW}
          strokeLinecap="round"
        />
      ))}
      {/* Filled segments */}
      {Array.from({ length: segments }, (_, i) => {
        if (i >= tier) return null;
        const isDevil = tier === 7;
        return (
          <path
            key={`fg-${i}`}
            d={arcPath(i)}
            fill="none"
            stroke={isDevil ? "#e94560" : "#e0e0e0"}
            strokeWidth={strokeW}
            strokeLinecap="round"
          />
        );
      })}
      {/* Center tier number */}
      {tier > 0 && (
        <text
          x={cx}
          y={cy + 1}
          textAnchor="middle"
          dominantBaseline="central"
          fill={tier === 7 ? "#e94560" : "#e0e0e0"}
          fontSize="13"
          fontWeight="700"
          fontFamily="sans-serif"
        >
          {tier}
        </text>
      )}
    </svg>
  );
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
}: MetadataEditorProps) {
  const [showChart, setShowChart] = useState(false);
  const [confirmDelete, setConfirmDelete] = useState(false);
  const [deleting, setDeleting] = useState(false);
  const [deleteError, setDeleteError] = useState<string | null>(null);
  const m = details.metadata;

  const numOrNull = (val: string): number | null => {
    if (val === "") return null;
    const n = parseInt(val, 10);
    return isNaN(n) ? null : n;
  };

  return (
    <div className="metadata-editor">
      <div className="editor-header">
        <h2>{m.name || details.display_name || "Untitled"}</h2>
        <div className="editor-header-buttons">
          <button
            className="chart-btn"
            onClick={() => setShowChart(true)}
          >
            Chart
          </button>
          <button
            className={`save-btn ${hasChanges ? "has-changes" : ""}`}
            onClick={onSave}
            disabled={saving}
          >
            {saving ? "Saving..." : hasChanges ? "Save Changes" : "Save"}
          </button>
          <button
            className="delete-song-btn"
            onClick={() => setConfirmDelete(true)}
            disabled={deleting}
          >
            Delete
          </button>
        </div>
        {confirmDelete && (
          <div className="delete-confirm-bar">
            <span>Permanently delete this file? This cannot be undone.</span>
            {deleteError && <span className="delete-error">{deleteError}</span>}
            <div className="delete-confirm-btns">
              <button
                className="delete-confirm-yes"
                disabled={deleting}
                onClick={async () => {
                  setDeleting(true);
                  setDeleteError(null);
                  try {
                    await onDelete(details.path);
                  } catch (e) {
                    setDeleteError(String(e));
                    setDeleting(false);
                  }
                }}
              >
                {deleting ? "Deleting..." : "Yes, Delete"}
              </button>
              <button
                className="delete-confirm-no"
                disabled={deleting}
                onClick={() => { setConfirmDelete(false); setDeleteError(null); }}
              >
                Cancel
              </button>
            </div>
          </div>
        )}
      </div>

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
                ["Drums", "rank_drum"],
                ["Guitar", "rank_guitar"],
                ["Bass", "rank_bass"],
                ["Vocals", "rank_vocals"],
                ["Keys", "rank_keys"],
                ["Band", "rank_band"],
                ["Pro Guitar", "rank_real_guitar"],
                ["Pro Bass", "rank_real_bass"],
                ["Pro Keys", "rank_real_keys"],
              ] as const).map(([label, field]) => {
                const rawVal = (m as any)[field] as number | null | undefined;
                const tier = rankToTier(field, rawVal);
                return (
                  <div key={field} className="rank-field-with-ring">
                    <div className="rank-ring-container" title={TIER_LABELS[tier]}>
                      <DifficultyRing tier={tier} />
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
                  </div>
                ))}
              </div>
            </section>
          )}
        </div>

        <div className="editor-sidebar">
          <ImageEditor
            thumbnailBase64={details.thumbnail_base64}
            albumArtBase64={albumArtBase64}
            songPath={details.path}
            onReplace={onUpdateThumbnail}
            artist={m.artist}
            albumName={m.album_name}
          />
          <SongScores songName={m.name} />
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
