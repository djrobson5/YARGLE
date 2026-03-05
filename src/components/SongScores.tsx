import React, { useState, useEffect } from "react";
import { invoke } from "@tauri-apps/api/core";

interface SongScore {
  date: string;
  player_name: string;
  instrument: string;
  difficulty: string;
  score: number;
  stars: number;
  percent: number;
  is_fc: boolean;
  notes_hit: number;
  notes_missed: number;
  band_score: number;
  band_stars: number;
  speed: number;
}

interface SongScoresProps {
  songName: string;
}

function StarDisplay({ count }: { count: number }) {
  if (count <= 0) return null;
  const isGold = count >= 6;
  const numStars = Math.min(count, 5);
  return (
    <span className={`song-score-stars ${isGold ? "song-score-stars-gold" : ""}`}>
      {"\u2605".repeat(numStars)}
    </span>
  );
}

export function SongScores({ songName }: SongScoresProps) {
  const [scores, setScores] = useState<SongScore[]>([]);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    if (!songName) {
      setScores([]);
      return;
    }
    let cancelled = false;
    setLoading(true);
    setError(null);
    invoke<SongScore[]>("get_song_scores", { songName })
      .then((result) => {
        if (!cancelled) setScores(result);
      })
      .catch((e) => {
        if (!cancelled) setError(String(e));
      })
      .finally(() => {
        if (!cancelled) setLoading(false);
      });
    return () => { cancelled = true; };
  }, [songName]);

  return (
    <div className="song-scores">
      <h3>Scores</h3>
      {loading && <div className="song-scores-empty">Loading...</div>}
      {error && <div className="song-scores-empty song-scores-error">{error}</div>}
      {!loading && !error && scores.length === 0 && (
        <div className="song-scores-empty">No scores found</div>
      )}
      {scores.length > 0 && (
        <div className="song-scores-list">
          {scores.map((s, i) => (
            <div key={i} className="song-score-entry">
              <div className="song-score-top">
                <span className="song-score-instrument">{s.instrument}</span>
                <span className="song-score-difficulty">{s.difficulty}</span>
                {s.is_fc && <span className="song-score-fc">FC</span>}
              </div>
              <div className="song-score-mid">
                <span className="song-score-percent">
                  {(s.percent * 100).toFixed(1)}%
                </span>
                <StarDisplay count={s.stars} />
              </div>
              <div className="song-score-bottom">
                <span className="song-score-value">{s.score.toLocaleString()}</span>
                <span className="song-score-date">{s.date}</span>
              </div>
              {s.speed !== 1.0 && (
                <div className="song-score-speed">{(s.speed * 100).toFixed(0)}% speed</div>
              )}
            </div>
          ))}
        </div>
      )}
    </div>
  );
}
