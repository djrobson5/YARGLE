import React, { useCallback, useEffect, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import type {
  ChartOverview,
  InstrumentSummary,
  InstrumentNotes,
  ChartNote,
  TempoEvent,
} from "../types";

interface ChartPreviewModalProps {
  songPath: string;
  onClose: () => void;
}

const DIFFICULTIES = ["expert", "hard", "medium", "easy"] as const;

const LANE_COLORS_GUITAR = [
  "#00cc00", // green
  "#cc0000", // red
  "#cccc00", // yellow
  "#0066cc", // blue
  "#cc6600", // orange
];

const LANE_COLORS_DRUMS = [
  "#cc6600", // kick (orange)
  "#cc0000", // red
  "#cccc00", // yellow
  "#0066cc", // blue
  "#00cc00", // green
];

const LANE_WIDTH = 28;
const LANE_GAP = 4;
const HIGHWAY_PADDING = 40;
const NOTE_HEIGHT = 10;
const PIXELS_PER_TICK = 0.15;
const BEAT_LINE_COLOR = "rgba(255,255,255,0.08)";
const MEASURE_LINE_COLOR = "rgba(255,255,255,0.25)";

export function ChartPreviewModal({ songPath, onClose }: ChartPreviewModalProps) {
  const [overview, setOverview] = useState<ChartOverview | null>(null);
  const [notesData, setNotesData] = useState<InstrumentNotes | null>(null);
  const [selectedInstrument, setSelectedInstrument] = useState<string>("");
  const [selectedDifficulty, setSelectedDifficulty] = useState<string>("expert");
  const [loading, setLoading] = useState(true);
  const [notesLoading, setNotesLoading] = useState(false);
  const [error, setError] = useState<string>("");

  const canvasRef = useRef<HTMLCanvasElement>(null);
  const scrollContainerRef = useRef<HTMLDivElement>(null);
  const [scrollTop, setScrollTop] = useState(0);

  // Load overview on mount
  useEffect(() => {
    setLoading(true);
    setError("");
    invoke<ChartOverview>("get_chart_overview", { path: songPath })
      .then((ov) => {
        setOverview(ov);
        if (ov.instruments.length > 0) {
          setSelectedInstrument(ov.instruments[0].name);
        }
      })
      .catch((e) => setError(String(e)))
      .finally(() => setLoading(false));
  }, [songPath]);

  // Load notes when instrument/difficulty changes
  useEffect(() => {
    if (!selectedInstrument || !overview) return;
    setNotesLoading(true);
    invoke<InstrumentNotes>("get_chart_notes", {
      path: songPath,
      instrument: selectedInstrument,
      difficulty: selectedDifficulty,
    })
      .then((data) => {
        setNotesData(data);
        // Scroll to bottom (start of song) since highway is bottom-to-top
        requestAnimationFrame(() => {
          if (scrollContainerRef.current) {
            scrollContainerRef.current.scrollTop = scrollContainerRef.current.scrollHeight;
            setScrollTop(scrollContainerRef.current.scrollTop);
          }
        });
      })
      .catch((e) => setError(String(e)))
      .finally(() => setNotesLoading(false));
  }, [songPath, selectedInstrument, selectedDifficulty, overview]);

  // Draw canvas
  useEffect(() => {
    if (!notesData || !canvasRef.current) return;
    drawHighway(canvasRef.current, notesData, scrollTop, selectedInstrument === "Drums");
  }, [notesData, scrollTop, selectedInstrument]);

  const handleScroll = useCallback((e: React.UIEvent<HTMLDivElement>) => {
    setScrollTop(e.currentTarget.scrollTop);
  }, []);

  const currentSummary: InstrumentSummary | undefined = overview?.instruments.find(
    (i) => i.name === selectedInstrument
  );

  const totalHeight = notesData
    ? notesData.duration_ticks * PIXELS_PER_TICK + 100
    : 0;

  const handleDensityClick = useCallback(
    (e: React.MouseEvent<SVGSVGElement>) => {
      if (!overview || !currentSummary || !scrollContainerRef.current) return;
      const svg = e.currentTarget;
      const rect = svg.getBoundingClientRect();
      const fraction = (e.clientX - rect.left) / rect.width;
      // Reversed: clicking left (start of song) scrolls to bottom, right scrolls to top
      const targetScroll = (1 - fraction) * totalHeight;
      scrollContainerRef.current.scrollTop = targetScroll;
    },
    [overview, currentSummary, totalHeight]
  );

  // Close on Escape
  useEffect(() => {
    const handler = (e: KeyboardEvent) => {
      if (e.key === "Escape") onClose();
    };
    window.addEventListener("keydown", handler);
    return () => window.removeEventListener("keydown", handler);
  }, [onClose]);

  return (
    <div className="art-search-overlay" onClick={(e) => {
      if (e.target === e.currentTarget) onClose();
    }}>
      <div className="chart-preview-modal" onClick={(e) => e.stopPropagation()}>
        <div className="art-search-header">
          <h3>Chart Preview</h3>
          <button className="art-search-close" onClick={onClose}>&times;</button>
        </div>

        {loading && <div className="chart-preview-loading">Loading chart data...</div>}
        {error && <div className="chart-preview-error">{error}</div>}

        {overview && !error && (
          <>
            {/* Instrument tabs */}
            <div className="chart-preview-tabs">
              {overview.instruments.map((inst) => (
                <button
                  key={inst.name}
                  className={`chart-tab ${inst.name === selectedInstrument ? "active" : ""}`}
                  onClick={() => setSelectedInstrument(inst.name)}
                >
                  {inst.name}
                  <span className="chart-tab-count">
                    {(inst.note_counts as any)[selectedDifficulty] ?? 0}
                  </span>
                </button>
              ))}
            </div>

            {/* Difficulty selector */}
            <div className="chart-preview-difficulty">
              {DIFFICULTIES.map((d) => (
                <button
                  key={d}
                  className={`chart-diff-btn ${d === selectedDifficulty ? "active" : ""}`}
                  onClick={() => setSelectedDifficulty(d)}
                >
                  {d.charAt(0).toUpperCase() + d.slice(1)}
                </button>
              ))}
              <span className="chart-duration">
                {formatDuration(overview.duration_ms)} | {overview.total_measures} measures
              </span>
            </div>

            {/* Density minimap */}
            {currentSummary && currentSummary.density.length > 0 && (
              <div className="chart-density-bar">
                <DensityBar density={currentSummary.density} onClick={handleDensityClick} />
              </div>
            )}

            {/* Note highway */}
            <div
              className="chart-highway"
              ref={scrollContainerRef}
              onScroll={handleScroll}
            >
              {notesLoading && (
                <div className="chart-preview-loading">Loading notes...</div>
              )}
              {notesData && !notesLoading && (
                <div style={{ height: totalHeight, position: "relative" }}>
                  <canvas
                    ref={canvasRef}
                    width={LANE_WIDTH * 5 + LANE_GAP * 4 + HIGHWAY_PADDING * 2}
                    height={600}
                    style={{
                      position: "sticky",
                      top: 0,
                      display: "block",
                      margin: "0 auto",
                    }}
                  />
                </div>
              )}
              {notesData && !notesLoading && notesData.notes.length === 0 && (
                <div className="chart-preview-empty">
                  No notes for {selectedInstrument} on {selectedDifficulty}
                </div>
              )}
            </div>
          </>
        )}

        {overview && overview.instruments.length === 0 && (
          <div className="chart-preview-empty">No chart data found in this file</div>
        )}
      </div>
    </div>
  );
}

function DensityBar({
  density,
  onClick,
}: {
  density: number[];
  onClick: (e: React.MouseEvent<SVGSVGElement>) => void;
}) {
  const max = Math.max(...density, 1);
  const w = density.length;
  const h = 24;

  return (
    <svg
      viewBox={`0 0 ${w} ${h}`}
      preserveAspectRatio="none"
      className="chart-density-svg"
      onClick={onClick}
    >
      {density.map((v, i) => {
        const barH = (v / max) * h;
        const intensity = v / max;
        const r = Math.round(30 + 203 * intensity);
        const g = Math.round(30 + 39 * (1 - intensity));
        const b = Math.round(78 - 18 * intensity);
        return (
          <rect
            key={i}
            x={i}
            y={h - barH}
            width={1}
            height={barH}
            fill={`rgb(${r},${g},${b})`}
          />
        );
      })}
    </svg>
  );
}

function drawHighway(
  canvas: HTMLCanvasElement,
  data: InstrumentNotes,
  scrollTop: number,
  isDrums: boolean
) {
  const ctx = canvas.getContext("2d");
  if (!ctx) return;

  const width = canvas.width;
  const height = canvas.height;
  const colors = isDrums ? LANE_COLORS_DRUMS : LANE_COLORS_GUITAR;
  const totalH = data.duration_ticks * PIXELS_PER_TICK + 100;

  ctx.clearRect(0, 0, width, height);

  // Background
  ctx.fillStyle = "#0d0d1a";
  ctx.fillRect(0, 0, width, height);

  // Reversed Y: low ticks at bottom, high ticks at top
  // divY for a tick = totalH - tick * PIXELS_PER_TICK
  // canvasY = divY - scrollTop = totalH - tick * PIXELS_PER_TICK - scrollTop
  const tickToY = (tick: number) => {
    return totalH - tick * PIXELS_PER_TICK - scrollTop;
  };

  // Visible tick range (inverted)
  const tickBottom = (totalH - scrollTop) / PIXELS_PER_TICK;
  const tickTop = (totalH - scrollTop - height) / PIXELS_PER_TICK;

  // Draw lane backgrounds
  for (let lane = 0; lane < 5; lane++) {
    const x = HIGHWAY_PADDING + lane * (LANE_WIDTH + LANE_GAP);
    ctx.fillStyle = "rgba(255,255,255,0.02)";
    ctx.fillRect(x, 0, LANE_WIDTH, height);
  }

  // Draw beat/measure lines
  drawGridLines(ctx, data, tickTop, tickBottom, width, tickToY);

  // Draw overdrive phrases (behind notes)
  const highwayLeft = HIGHWAY_PADDING;
  const highwayRight = HIGHWAY_PADDING + 5 * LANE_WIDTH + 4 * LANE_GAP;
  for (const od of data.overdrive_phrases) {
    if (od.end_tick < tickTop || od.start_tick > tickBottom) continue;
    const yStart = tickToY(od.start_tick);
    const yEnd = tickToY(od.end_tick);
    const top = Math.min(yStart, yEnd);
    const bot = Math.max(yStart, yEnd);

    // Translucent white overlay
    ctx.fillStyle = "rgba(255, 255, 255, 0.06)";
    ctx.fillRect(highwayLeft - 2, top, highwayRight - highwayLeft + 4, bot - top);

    // White edge lines on left and right
    ctx.strokeStyle = "rgba(255, 255, 255, 0.25)";
    ctx.lineWidth = 1.5;
    ctx.beginPath();
    ctx.moveTo(highwayLeft - 2, top);
    ctx.lineTo(highwayLeft - 2, bot);
    ctx.moveTo(highwayRight + 2, top);
    ctx.lineTo(highwayRight + 2, bot);
    ctx.stroke();

    // "SP" label at phrase start (bottom in reversed view)
    if (yStart >= -20 && yStart <= height + 20) {
      ctx.fillStyle = "rgba(255, 255, 255, 0.5)";
      ctx.font = "bold 9px sans-serif";
      ctx.fillText("OD", highwayRight + 6, yStart + 3);
    }
  }

  // Binary search for first potentially visible note
  const notes = data.notes;
  let lo = 0;
  let hi = notes.length;
  while (lo < hi) {
    const mid = (lo + hi) >>> 1;
    if (notes[mid].tick + notes[mid].duration < tickTop) {
      lo = mid + 1;
    } else {
      hi = mid;
    }
  }

  // Draw visible notes
  for (let i = lo; i < notes.length; i++) {
    const note = notes[i];
    if (note.tick > tickBottom) break;

    const y = tickToY(note.tick);
    const sustainEndY = tickToY(note.tick + note.duration);
    const x = HIGHWAY_PADDING + note.lane * (LANE_WIDTH + LANE_GAP);
    const color = colors[note.lane] || "#888888";
    const cx = x + LANE_WIDTH / 2;

    // Draw sustain tail (goes upward from note head)
    if (note.duration > 0) {
      ctx.fillStyle = color + "60";
      const tailTop = Math.min(y, sustainEndY);
      const tailBot = Math.max(y, sustainEndY);
      ctx.fillRect(cx - 4, tailTop, 8, tailBot - tailTop);
    }

    if (note.is_hopo) {
      // HOPO: smaller glowing circle
      const radius = LANE_WIDTH / 2 - 4;
      ctx.beginPath();
      ctx.arc(cx, y, radius, 0, Math.PI * 2);
      ctx.fillStyle = "#0d0d1a";
      ctx.fill();
      ctx.lineWidth = 2.5;
      ctx.strokeStyle = color;
      ctx.stroke();

      // Inner glow
      ctx.beginPath();
      ctx.arc(cx, y, radius - 2, 0, Math.PI * 2);
      ctx.fillStyle = color + "40";
      ctx.fill();
    } else {
      // Regular note: rounded rectangle
      ctx.fillStyle = color;
      ctx.beginPath();
      ctx.roundRect(x + 1, y - NOTE_HEIGHT / 2, LANE_WIDTH - 2, NOTE_HEIGHT, 3);
      ctx.fill();

      // Highlight stripe
      ctx.fillStyle = "rgba(255,255,255,0.2)";
      ctx.fillRect(x + 3, y - NOTE_HEIGHT / 2 + 1, LANE_WIDTH - 6, 3);
    }
  }
}

function drawGridLines(
  ctx: CanvasRenderingContext2D,
  data: InstrumentNotes,
  tickTop: number,
  tickBottom: number,
  canvasWidth: number,
  tickToY: (tick: number) => number
) {
  const tpq = data.ticks_per_quarter;

  // Find current time signature
  let currentNum = 4;
  let currentDen = 4;
  for (const ts of data.time_signatures) {
    if (ts.tick > tickBottom) break;
    currentNum = ts.numerator;
    currentDen = ts.denominator;
  }

  const ticksPerBeat = (tpq * 4) / currentDen;
  const ticksPerMeasure = ticksPerBeat * currentNum;

  if (ticksPerBeat <= 0) return;

  const startMeasure = Math.max(0, Math.floor(tickTop / ticksPerMeasure));
  const startTick = startMeasure * ticksPerMeasure;

  for (let tick = startTick; tick <= tickBottom; tick += ticksPerBeat) {
    const y = tickToY(tick);
    if (y < -10 || y > ctx.canvas.height + 10) continue;
    const isMeasure = Math.abs(tick % ticksPerMeasure) < 1;

    ctx.strokeStyle = isMeasure ? MEASURE_LINE_COLOR : BEAT_LINE_COLOR;
    ctx.lineWidth = isMeasure ? 1.5 : 0.5;
    ctx.beginPath();
    ctx.moveTo(HIGHWAY_PADDING - 5, y);
    ctx.lineTo(canvasWidth - HIGHWAY_PADDING + 5, y);
    ctx.stroke();

    if (isMeasure) {
      const measureNum = Math.round(tick / ticksPerMeasure) + 1;
      ctx.fillStyle = "rgba(255,255,255,0.3)";
      ctx.font = "10px sans-serif";
      ctx.fillText(String(measureNum), 4, y - 4);
    }
  }
}

function formatDuration(ms: number): string {
  const totalSec = Math.round(ms / 1000);
  const min = Math.floor(totalSec / 60);
  const sec = totalSec % 60;
  return `${min}:${sec.toString().padStart(2, "0")}`;
}
