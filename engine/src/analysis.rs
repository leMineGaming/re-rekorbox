//! Track analysis for a deck, built from the rekordbox analysis files (library).

use std::sync::Arc;

use rekordbox_formats::library::LibTrack;

use crate::deck::{BeatMap, Grid, HOT_CUES};

/// What a deck takes from an analysis.
#[derive(Debug, Clone, PartialEq)]
pub struct TrackAnalysis {
    /// Constant grid (tempo of the first beat, first downbeat) – also the fallback for display.
    pub grid: Option<Grid>,
    /// Exact beat positions (all tracks with a grid; needed for tracks with changing tempo).
    pub beats: Option<Arc<BeatMap>>,
    pub hot_cues: [Option<f64>; HOT_CUES],
    pub memory_cues: Vec<f64>,
    pub title: String,
    pub variable_tempo: bool,
}

impl TrackAnalysis {
    pub fn from_library(t: &LibTrack) -> TrackAnalysis {
        let grid = match (t.bpm(), t.first_downbeat_s()) {
            (Some(bpm), Some(first_beat_s)) => Some(Grid { bpm, first_beat_s }),
            _ => None,
        };
        let times: Vec<f64> = t.beats.iter().map(|b| f64::from(b.time_ms) / 1000.0).collect();
        let numbers: Vec<u16> = t.beats.iter().map(|b| b.beat).collect();
        TrackAnalysis {
            grid,
            beats: BeatMap::new(times, &numbers).map(|m| Arc::new(m.with_bpms(t.beats.iter().map(|b| b.bpm()).collect()))),
            hot_cues: t.hot_cues_s(),
            memory_cues: t.memory_cues_s(),
            title: match &t.artist {
                Some(a) => format!("{a} - {}", t.title),
                None => t.title.clone(),
            },
            variable_tempo: t.variable_tempo(),
        }
    }
}
