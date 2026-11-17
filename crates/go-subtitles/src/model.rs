//! Canonical subtitle domain types.
//!
//! `SubtitleDocument` is the format-independent internal representation.
//! SRT, WebVTT, and ASS/SSA are adapters — they parse into and serialize
//! from this model, but the model itself never depends on a file format.

use std::time::Duration;

/// A single subtitle cue: text shown between `start` and `end`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SubtitleCue {
    /// Optional cue identifier carried over from formats that have one
    /// (e.g. ASS event ids). Not part of timing.
    pub id: Option<String>,
    pub start: Duration,
    pub end: Duration,
    /// Cue text. May contain newlines and lightweight markup depending
    /// on the source format; normalization is an adapter concern.
    pub text: String,
}

impl SubtitleCue {
    pub fn new(start: Duration, end: Duration, text: impl Into<String>) -> Self {
        Self {
            id: None,
            start,
            end,
            text: text.into(),
        }
    }

    pub fn duration(&self) -> Duration {
        self.end.saturating_sub(self.start)
    }
}

/// Document-level metadata. Timestamps stay authoritative; this carries
/// descriptive info only.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SubtitleDocument {
    /// BCP-47-ish language tag of the cue text, if known.
    pub language: Option<String>,
    /// Human-readable title, if any.
    pub title: Option<String>,
    pub cues: Vec<SubtitleCue>,
}

impl SubtitleDocument {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn is_empty(&self) -> bool {
        self.cues.is_empty()
    }

    /// Structural validation: every cue must have positive duration and
    /// cues must not overlap (end of one <= start of next, in order).
    /// Returns the index of the first offending cue.
    pub fn validate(&self) -> Result<(), SubtitleValidationError> {
        let mut prev_end: Option<Duration> = None;
        for (i, cue) in self.cues.iter().enumerate() {
            if cue.end <= cue.start {
                return Err(SubtitleValidationError::NonPositiveDuration { index: i });
            }
            if prev_end.is_some_and(|pe| cue.start < pe) {
                return Err(SubtitleValidationError::Overlap { index: i });
            }
            prev_end = Some(cue.end);
        }
        Ok(())
    }

    /// Sort cues by start time and re-validate.
    pub fn sort_by_start(&mut self) {
        self.cues.sort_by_key(|c| c.start);
    }
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum SubtitleValidationError {
    #[error("cue {index} has non-positive duration (end <= start)")]
    NonPositiveDuration { index: usize },
    #[error("cue {index} overlaps with the previous cue")]
    Overlap { index: usize },
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validate_rejects_zero_duration() {
        let mut doc = SubtitleDocument::new();
        doc.cues.push(SubtitleCue::new(
            Duration::from_secs(1),
            Duration::from_secs(1),
            "x",
        ));
        assert!(matches!(
            doc.validate(),
            Err(SubtitleValidationError::NonPositiveDuration { index: 0 })
        ));
    }

    #[test]
    fn validate_rejects_overlap() {
        let mut doc = SubtitleDocument::new();
        doc.cues.push(SubtitleCue::new(
            Duration::from_secs(0),
            Duration::from_secs(5),
            "a",
        ));
        doc.cues.push(SubtitleCue::new(
            Duration::from_secs(4),
            Duration::from_secs(6),
            "b",
        ));
        assert!(matches!(
            doc.validate(),
            Err(SubtitleValidationError::Overlap { index: 1 })
        ));
    }

    #[test]
    fn validate_accepts_adjacent_cues() {
        let mut doc = SubtitleDocument::new();
        doc.cues.push(SubtitleCue::new(
            Duration::from_secs(0),
            Duration::from_secs(5),
            "a",
        ));
        doc.cues.push(SubtitleCue::new(
            Duration::from_secs(5),
            Duration::from_secs(9),
            "b",
        ));
        assert!(doc.validate().is_ok());
    }
}
