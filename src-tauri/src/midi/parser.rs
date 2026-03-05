use midly::{MidiMessage, Smf, TrackEventKind};
use std::collections::{HashMap, HashSet};

use super::types::*;

/// Map of Rock Band MIDI track names to display names
const INSTRUMENT_TRACKS: &[(&str, &str)] = &[
    ("PART GUITAR", "Guitar"),
    ("PART BASS", "Bass"),
    ("PART DRUMS", "Drums"),
    ("PART VOCALS", "Vocals"),
    ("PART KEYS", "Keys"),
];

/// MIDI note ranges per difficulty (base note for lane 0, last note for lane 4)
fn difficulty_range(difficulty: &str) -> Option<(u8, u8)> {
    match difficulty {
        "expert" => Some((96, 100)),
        "hard" => Some((84, 88)),
        "medium" => Some((72, 76)),
        "easy" => Some((60, 64)),
        _ => None,
    }
}

/// HOPO force marker note for each difficulty (note = range_lo + 5)
fn hopo_force_note(difficulty: &str) -> Option<u8> {
    match difficulty {
        "expert" => Some(101),
        "hard" => Some(89),
        "medium" => Some(77),
        "easy" => Some(65),
        _ => None,
    }
}

/// HOPO proximity threshold: notes within 1/12 of a quarter note are auto-HOPO
/// (standard Rock Band threshold: tpq * 4 / 12 = tpq / 3)
fn hopo_threshold(tpq: u16) -> u32 {
    (tpq as u32) / 3 + 1 // slight fudge to match RB behavior
}

/// Parse a lightweight chart overview from MIDI bytes.
pub fn parse_chart_overview(midi_bytes: &[u8]) -> Result<ChartOverview, String> {
    let smf = Smf::parse(midi_bytes).map_err(|e| format!("Failed to parse MIDI: {}", e))?;

    let tpq = match smf.header.timing {
        midly::Timing::Metrical(tpq) => tpq.as_int(),
        _ => return Err("SMPTE timing not supported".into()),
    };

    // Parse tempo map and time signatures from all tracks (usually track 0)
    let mut tempo_changes: Vec<TempoEvent> = Vec::new();
    let mut time_sigs: Vec<TimeSigEvent> = Vec::new();

    for track in &smf.tracks {
        let mut tick: u32 = 0;
        for event in track {
            tick += event.delta.as_int();
            match event.kind {
                TrackEventKind::Meta(midly::MetaMessage::Tempo(t)) => {
                    let bpm = 60_000_000.0 / t.as_int() as f64;
                    tempo_changes.push(TempoEvent { tick, bpm });
                }
                TrackEventKind::Meta(midly::MetaMessage::TimeSignature(num, den, _, _)) => {
                    time_sigs.push(TimeSigEvent {
                        tick,
                        numerator: num,
                        denominator: 1 << den,
                    });
                }
                _ => {}
            }
        }
    }

    if tempo_changes.is_empty() {
        tempo_changes.push(TempoEvent {
            tick: 0,
            bpm: 120.0,
        });
    }
    if time_sigs.is_empty() {
        time_sigs.push(TimeSigEvent {
            tick: 0,
            numerator: 4,
            denominator: 4,
        });
    }

    // Compute measure boundaries for density calculation
    let max_tick = smf
        .tracks
        .iter()
        .map(|t| {
            let mut tick: u32 = 0;
            for ev in t {
                tick += ev.delta.as_int();
            }
            tick
        })
        .max()
        .unwrap_or(0);

    let measure_ticks = compute_measure_boundaries(&time_sigs, tpq, max_tick);
    let total_measures = measure_ticks.len().saturating_sub(1) as u32;

    // Compute duration in ms
    let duration_ms = ticks_to_ms(max_tick, &tempo_changes, tpq);

    // Find instrument tracks
    let mut instruments = Vec::new();

    for track in &smf.tracks {
        let track_name = find_track_name(track);
        let track_name_upper = track_name.to_uppercase();

        for &(rb_name, display_name) in INSTRUMENT_TRACKS {
            if track_name_upper == rb_name {
                // Count notes per difficulty
                let mut note_on_ticks: Vec<(u8, u32)> = Vec::new(); // (note, tick)
                let mut tick: u32 = 0;
                for event in track {
                    tick += event.delta.as_int();
                    if let TrackEventKind::Midi { message, .. } = event.kind {
                        match message {
                            MidiMessage::NoteOn { key, vel } if vel.as_int() > 0 => {
                                note_on_ticks.push((key.as_int(), tick));
                            }
                            _ => {}
                        }
                    }
                }

                let count_in_range = |lo: u8, hi: u8| -> u32 {
                    note_on_ticks
                        .iter()
                        .filter(|(n, _)| *n >= lo && *n <= hi)
                        .count() as u32
                };

                let note_counts = DifficultyNoteCounts {
                    easy: count_in_range(60, 64),
                    medium: count_in_range(72, 76),
                    hard: count_in_range(84, 88),
                    expert: count_in_range(96, 100),
                };

                // Compute density: notes per measure at Expert
                let expert_ticks: Vec<u32> = note_on_ticks
                    .iter()
                    .filter(|(n, _)| *n >= 96 && *n <= 100)
                    .map(|(_, t)| *t)
                    .collect();

                let density = compute_density(&expert_ticks, &measure_ticks);

                instruments.push(InstrumentSummary {
                    name: display_name.to_string(),
                    track_name: rb_name.to_string(),
                    note_counts,
                    density,
                });
                break;
            }
        }
    }

    Ok(ChartOverview {
        duration_ms,
        total_measures,
        ticks_per_quarter: tpq,
        instruments,
    })
}

/// Parse full note data for one instrument+difficulty.
pub fn parse_instrument_notes(
    midi_bytes: &[u8],
    instrument: &str,
    difficulty: &str,
) -> Result<InstrumentNotes, String> {
    let smf = Smf::parse(midi_bytes).map_err(|e| format!("Failed to parse MIDI: {}", e))?;

    let tpq = match smf.header.timing {
        midly::Timing::Metrical(tpq) => tpq.as_int(),
        _ => return Err("SMPTE timing not supported".into()),
    };

    let (range_lo, range_hi) =
        difficulty_range(difficulty).ok_or_else(|| format!("Unknown difficulty: {}", difficulty))?;

    // Find the target track name
    let target_track = INSTRUMENT_TRACKS
        .iter()
        .find(|&&(_, name)| name.eq_ignore_ascii_case(instrument))
        .map(|&(track, _)| track)
        .ok_or_else(|| format!("Unknown instrument: {}", instrument))?;

    // Parse tempo and time sig from all tracks
    let mut tempo_changes: Vec<TempoEvent> = Vec::new();
    let mut time_sigs: Vec<TimeSigEvent> = Vec::new();
    let mut max_tick: u32 = 0;

    for track in &smf.tracks {
        let mut tick: u32 = 0;
        for event in track {
            tick += event.delta.as_int();
            match event.kind {
                TrackEventKind::Meta(midly::MetaMessage::Tempo(t)) => {
                    let bpm = 60_000_000.0 / t.as_int() as f64;
                    tempo_changes.push(TempoEvent { tick, bpm });
                }
                TrackEventKind::Meta(midly::MetaMessage::TimeSignature(num, den, _, _)) => {
                    time_sigs.push(TimeSigEvent {
                        tick,
                        numerator: num,
                        denominator: 1 << den,
                    });
                }
                _ => {}
            }
        }
        if tick > max_tick {
            max_tick = tick;
        }
    }

    if tempo_changes.is_empty() {
        tempo_changes.push(TempoEvent {
            tick: 0,
            bpm: 120.0,
        });
    }
    if time_sigs.is_empty() {
        time_sigs.push(TimeSigEvent {
            tick: 0,
            numerator: 4,
            denominator: 4,
        });
    }

    let hopo_note = hopo_force_note(difficulty);
    let threshold = hopo_threshold(tpq);
    let is_drums = target_track == "PART DRUMS";

    const OVERDRIVE_NOTE: u8 = 116;

    // Find the instrument track and extract notes + HOPO force regions + overdrive
    let mut notes: Vec<ChartNote> = Vec::new();
    let mut hopo_force_ticks: HashSet<u32> = HashSet::new();
    let mut overdrive_phrases: Vec<OverdrivePhrase> = Vec::new();

    for track in &smf.tracks {
        let track_name = find_track_name(track);
        if track_name.to_uppercase() != target_track {
            continue;
        }

        // Track active note-on events: key -> tick
        let mut active: HashMap<u8, u32> = HashMap::new();
        let mut hopo_active_start: Option<u32> = None;
        let mut od_start: Option<u32> = None;
        let mut tick: u32 = 0;

        for event in track {
            tick += event.delta.as_int();
            if let TrackEventKind::Midi { message, .. } = event.kind {
                match message {
                    MidiMessage::NoteOn { key, vel } => {
                        let k = key.as_int();

                        // Overdrive phrase (note 116)
                        if k == OVERDRIVE_NOTE {
                            if vel.as_int() > 0 {
                                od_start = Some(tick);
                            } else if let Some(start) = od_start.take() {
                                overdrive_phrases.push(OverdrivePhrase {
                                    start_tick: start,
                                    end_tick: tick,
                                });
                            }
                        }

                        // Track HOPO force marker regions
                        if !is_drums {
                            if let Some(hn) = hopo_note {
                                if k == hn {
                                    if vel.as_int() > 0 {
                                        hopo_active_start = Some(tick);
                                    } else if let Some(_start) = hopo_active_start.take() {
                                        // Mark all note ticks in this range
                                        // (handled in post-processing below)
                                    }
                                }
                            }
                        }

                        if k >= range_lo && k <= range_hi {
                            if vel.as_int() > 0 {
                                active.insert(k, tick);
                                // Record if HOPO force marker is active at this tick
                                if hopo_active_start.is_some() {
                                    hopo_force_ticks.insert(tick);
                                }
                            } else {
                                // NoteOn with vel 0 = NoteOff
                                if let Some(start) = active.remove(&k) {
                                    notes.push(ChartNote {
                                        tick: start,
                                        duration: tick.saturating_sub(start),
                                        lane: k - range_lo,
                                        is_hopo: false, // set in post-processing
                                    });
                                }
                            }
                        }
                    }
                    MidiMessage::NoteOff { key, .. } => {
                        let k = key.as_int();

                        // Overdrive off
                        if k == OVERDRIVE_NOTE {
                            if let Some(start) = od_start.take() {
                                overdrive_phrases.push(OverdrivePhrase {
                                    start_tick: start,
                                    end_tick: tick,
                                });
                            }
                        }

                        if k >= range_lo && k <= range_hi {
                            if let Some(start) = active.remove(&k) {
                                notes.push(ChartNote {
                                    tick: start,
                                    duration: tick.saturating_sub(start),
                                    lane: k - range_lo,
                                    is_hopo: false,
                                });
                            }
                        }
                        // Clear HOPO force marker
                        if let Some(hn) = hopo_note {
                            if k == hn {
                                hopo_active_start = None;
                            }
                        }
                    }
                    _ => {}
                }
            }
        }

        // Close any still-active overdrive
        if let Some(start) = od_start {
            overdrive_phrases.push(OverdrivePhrase {
                start_tick: start,
                end_tick: tick,
            });
        }

        // Close any still-active notes
        for (k, start) in active {
            notes.push(ChartNote {
                tick: start,
                duration: tick.saturating_sub(start),
                lane: k - range_lo,
                is_hopo: false,
            });
        }

        break;
    }

    notes.sort_by_key(|n| n.tick);

    // Post-process: determine HOPO status for each note
    // Drums never have HOPOs
    if !is_drums {
        // Group notes by tick to find chords
        let mut i = 0;
        while i < notes.len() {
            let tick = notes[i].tick;
            // Find end of chord (all notes at the same tick)
            let mut j = i;
            while j < notes.len() && notes[j].tick == tick {
                j += 1;
            }
            let chord_size = j - i;

            // Only single notes can be HOPO (chords are always strummed)
            if chord_size == 1 {
                let has_force = hopo_force_ticks.contains(&tick);

                // Check proximity to previous note group
                let mut is_close = false;
                let mut same_fret = true;
                if i > 0 {
                    // Find previous note group
                    let prev_tick = notes[i - 1].tick;
                    let gap = tick.saturating_sub(prev_tick);
                    is_close = gap <= threshold && gap > 0;

                    // Check if same fret as previous (collect lanes of prev chord)
                    let mut prev_start = i - 1;
                    while prev_start > 0 && notes[prev_start - 1].tick == prev_tick {
                        prev_start -= 1;
                    }
                    let prev_lanes: Vec<u8> = notes[prev_start..i]
                        .iter()
                        .map(|n| n.lane)
                        .collect();
                    same_fret = prev_lanes.len() == 1 && prev_lanes[0] == notes[i].lane;
                }

                // HOPO if force marker is active, OR if close + different fret
                if has_force || (is_close && !same_fret) {
                    notes[i].is_hopo = true;
                }
            }

            i = j;
        }
    }

    Ok(InstrumentNotes {
        instrument: instrument.to_string(),
        difficulty: difficulty.to_string(),
        ticks_per_quarter: tpq,
        tempo_changes,
        time_signatures: time_sigs,
        notes,
        overdrive_phrases,
        duration_ticks: max_tick,
    })
}

/// Extract track name from the first TrackName meta event.
fn find_track_name(track: &[midly::TrackEvent]) -> String {
    for event in track {
        if let TrackEventKind::Meta(midly::MetaMessage::TrackName(name)) = event.kind {
            return String::from_utf8_lossy(name).to_string();
        }
    }
    String::new()
}

/// Compute measure boundary ticks from time signature events.
/// Returns a sorted list of tick positions where each measure starts.
fn compute_measure_boundaries(time_sigs: &[TimeSigEvent], tpq: u16, max_tick: u32) -> Vec<u32> {
    let mut boundaries = Vec::new();
    let mut tick: u32 = 0;
    let mut sig_idx = 0;

    while tick <= max_tick {
        boundaries.push(tick);

        // Find current time signature
        while sig_idx + 1 < time_sigs.len() && time_sigs[sig_idx + 1].tick <= tick {
            sig_idx += 1;
        }

        let sig = &time_sigs[sig_idx];
        // Ticks per measure = tpq * 4 * numerator / denominator
        let ticks_per_measure = (tpq as u32) * 4 * (sig.numerator as u32) / (sig.denominator as u32);
        if ticks_per_measure == 0 {
            break;
        }
        tick += ticks_per_measure;
    }

    boundaries
}

/// Compute notes-per-measure density array.
fn compute_density(note_ticks: &[u32], measure_boundaries: &[u32]) -> Vec<u16> {
    if measure_boundaries.len() < 2 {
        return Vec::new();
    }

    let num_measures = measure_boundaries.len() - 1;
    let mut density = vec![0u16; num_measures];

    for &t in note_ticks {
        // Binary search for which measure this tick falls in
        let idx = match measure_boundaries.binary_search(&t) {
            Ok(i) => i.min(num_measures - 1),
            Err(i) => i.saturating_sub(1).min(num_measures - 1),
        };
        density[idx] = density[idx].saturating_add(1);
    }

    density
}

/// Convert a tick position to milliseconds using the tempo map.
fn ticks_to_ms(tick: u32, tempo_changes: &[TempoEvent], tpq: u16) -> f64 {
    let mut ms = 0.0;
    let mut prev_tick: u32 = 0;
    let mut us_per_tick = 60_000_000.0 / (120.0 * tpq as f64); // default 120 BPM

    for tc in tempo_changes {
        if tc.tick >= tick {
            break;
        }
        if tc.tick > prev_tick {
            ms += (tc.tick - prev_tick) as f64 * us_per_tick / 1000.0;
            prev_tick = tc.tick;
        }
        us_per_tick = 60_000_000.0 / (tc.bpm * tpq as f64);
    }

    ms += (tick - prev_tick) as f64 * us_per_tick / 1000.0;
    ms
}
