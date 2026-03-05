use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChartOverview {
    pub duration_ms: f64,
    pub total_measures: u32,
    pub ticks_per_quarter: u16,
    pub instruments: Vec<InstrumentSummary>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InstrumentSummary {
    pub name: String,
    pub track_name: String,
    pub note_counts: DifficultyNoteCounts,
    /// Notes per measure at Expert difficulty
    pub density: Vec<u16>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DifficultyNoteCounts {
    pub easy: u32,
    pub medium: u32,
    pub hard: u32,
    pub expert: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InstrumentNotes {
    pub instrument: String,
    pub difficulty: String,
    pub ticks_per_quarter: u16,
    pub tempo_changes: Vec<TempoEvent>,
    pub time_signatures: Vec<TimeSigEvent>,
    pub notes: Vec<ChartNote>,
    pub overdrive_phrases: Vec<OverdrivePhrase>,
    pub duration_ticks: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OverdrivePhrase {
    pub start_tick: u32,
    pub end_tick: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChartNote {
    pub tick: u32,
    pub duration: u32,
    pub lane: u8,
    pub is_hopo: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TempoEvent {
    pub tick: u32,
    pub bpm: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TimeSigEvent {
    pub tick: u32,
    pub numerator: u8,
    pub denominator: u8,
}
