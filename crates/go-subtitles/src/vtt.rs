//! WebVTT parsing and serialization.
//!
//! Handles the `WEBVTT` header, optional metadata header lines, cue
//! settings after timestamps, and `NOTE` comment blocks.

use crate::model::{SubtitleCue, SubtitleDocument};
use std::time::Duration;

use crate::SubtitleError;

pub fn parse_vtt(input: &str) -> Result<SubtitleDocument, SubtitleError> {
    let input = input.strip_prefix('\u{feff}').unwrap_or(input);
    let mut doc = SubtitleDocument::new();
    let mut lines = input.lines().peekable();

    // Header: first line must start with WEBVTT.
    match lines.next() {
        Some(l) if l.trim_start().starts_with("WEBVTT") => {}
        _ => return Err(SubtitleError::MissingVttHeader),
    }

    // Optional header block until blank line; capture Kind/Language if present.
    while let Some(&l) = lines.peek() {
        if l.trim().is_empty() {
            lines.next();
            break;
        }
        if let Some(rest) = l.trim().strip_prefix("Kind:") {
            doc.language = Some(rest.trim().to_lowercase());
        } else if let Some(rest) = l.trim().strip_prefix("Language:") {
            doc.language = Some(rest.trim().to_lowercase());
        }
        lines.next();
    }

    let mut block: Vec<String> = Vec::new();
    for line in lines {
        let line = line.trim_end_matches('\r');
        if line.trim().is_empty() {
            flush_block(&mut block, &mut doc)?;
        } else if line.trim_start().starts_with("NOTE") {
            // Skip comment block until blank line — handled naturally since
            // NOTE lines land in `block`; detect and drop them on flush.
            block.push(line.to_string());
        } else {
            block.push(line.to_string());
        }
    }
    flush_block(&mut block, &mut doc)?;
    Ok(doc)
}

fn flush_block(block: &mut Vec<String>, doc: &mut SubtitleDocument) -> Result<(), SubtitleError> {
    if block.is_empty() {
        return Ok(());
    }
    let lines: Vec<String> = std::mem::take(block);
    if lines.iter().any(|l| l.trim_start().starts_with("NOTE"))
        && !lines.iter().any(|l| l.contains("-->"))
    {
        return Ok(()); // pure comment block
    }
    doc.cues.push(parse_vtt_block(&lines)?);
    Ok(())
}

fn parse_vtt_block(lines: &[String]) -> Result<SubtitleCue, SubtitleError> {
    // Find the timing line; everything before it is the optional cue id.
    let timing_idx = lines
        .iter()
        .position(|l| l.contains("-->"))
        .ok_or(SubtitleError::MissingTiming)?;

    let id = if timing_idx > 0 {
        lines[..timing_idx].join("\n")
    } else {
        String::new()
    };

    let (start, end) = parse_vtt_timing(&lines[timing_idx])?;
    let text = lines[timing_idx + 1..].join("\n");
    if text.trim().is_empty() {
        return Err(SubtitleError::EmptyCueText);
    }

    Ok(SubtitleCue {
        id: if id.is_empty() { None } else { Some(id) },
        start,
        end,
        text,
    })
}

/// `HH:MM:SS.mmm --> HH:MM:SS.mmm` possibly followed by cue settings.
pub fn parse_vtt_timing(line: &str) -> Result<(Duration, Duration), SubtitleError> {
    let arrow = line
        .find("-->")
        .ok_or_else(|| SubtitleError::BadTiming(line.to_string()))?;
    let start = parse_vtt_timestamp(line[..arrow].trim())?;
    let end_part = line[arrow + 3..].trim();
    let end_token = end_part.split_whitespace().next().unwrap_or("");
    let end = parse_vtt_timestamp(end_token)?;
    Ok((start, end))
}

fn parse_vtt_timestamp(ts: &str) -> Result<Duration, SubtitleError> {
    // MM:SS.mmm or HH:MM:SS.mmm — hours optional in VTT.
    let bad = || SubtitleError::BadTimestamp(ts.to_string());
    let (hms, ms) = ts.split_once('.').ok_or_else(bad)?;
    let parts: Vec<&str> = hms.split(':').collect();
    if parts.len() < 2 || parts.len() > 3 {
        return Err(bad());
    }
    let (h, m, s) = match parts.len() {
        3 => (
            parts[0].parse::<u64>().map_err(|_| bad())?,
            parts[1].parse::<u64>().map_err(|_| bad())?,
            parts[2].parse::<u64>().map_err(|_| bad())?,
        ),
        2 => (
            0,
            parts[0].parse::<u64>().map_err(|_| bad())?,
            parts[1].parse::<u64>().map_err(|_| bad())?,
        ),
        _ => return Err(bad()),
    };
    if m >= 60 || s >= 60 {
        return Err(bad());
    }
    let ms: u32 = ms.parse().map_err(|_| bad())?;
    if ms >= 1000 {
        return Err(bad());
    }
    Ok(Duration::from_millis(
        h * 3_600_000 + m * 60_000 + s * 1000 + ms as u64,
    ))
}

/// Serialize to WebVTT. Cue settings are not preserved (model doesn't carry them).
pub fn to_vtt(doc: &SubtitleDocument) -> String {
    let mut out = String::from("WEBVTT\n\n");
    for cue in &doc.cues {
        if let Some(id) = &cue.id {
            out.push_str(id);
            out.push('\n');
        }
        out.push_str(&format!(
            "{} --> {}\n{}\n\n",
            format_vtt_timestamp(cue.start),
            format_vtt_timestamp(cue.end),
            cue.text
        ));
    }
    out
}

pub fn format_vtt_timestamp(d: Duration) -> String {
    let ms_total = d.as_millis();
    let h = ms_total / 3_600_000;
    let m = (ms_total % 3_600_000) / 60_000;
    let s = (ms_total % 60_000) / 1000;
    let ms = ms_total % 1000;
    if h > 0 {
        format!("{h:02}:{m:02}:{s:02}.{ms:03}")
    } else {
        format!("{m:02}:{s:02}.{ms:03}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = "WEBVTT - Test\nKind: captions\nLanguage: en\n\n00:01.000 --> 00:04.000 align:start position:0%\nHello <b>world</b>\n\ncue-id-2\n00:05.500 --> 00:08.250\nSecond cue\n";

    #[test]
    fn parses_basic_vtt() {
        let doc = parse_vtt(SAMPLE).unwrap();
        assert_eq!(doc.cues.len(), 2);
        assert_eq!(doc.cues[0].start, Duration::from_millis(1000));
        assert_eq!(doc.cues[0].end, Duration::from_millis(4000));
        assert_eq!(doc.cues[0].text, "Hello <b>world</b>");
        assert_eq!(doc.cues[1].id.as_deref(), Some("cue-id-2"));
        assert!(doc.validate().is_ok());
    }

    #[test]
    fn skips_note_blocks() {
        let input =
            "WEBVTT\n\nNOTE this is a comment\nspanning lines\n\n00:01.000 --> 00:02.000\nHi\n";
        let doc = parse_vtt(input).unwrap();
        assert_eq!(doc.cues.len(), 1);
    }

    #[test]
    fn requires_header() {
        assert!(matches!(
            parse_vtt("00:01.000 --> 00:02.000\nHi\n"),
            Err(SubtitleError::MissingVttHeader)
        ));
    }

    #[test]
    fn roundtrips() {
        let doc = parse_vtt(SAMPLE).unwrap();
        let vtt = to_vtt(&doc);
        let doc2 = parse_vtt(&vtt).unwrap();
        assert_eq!(doc.cues.len(), doc2.cues.len());
        assert_eq!(doc.cues[1].id, doc2.cues[1].id);
    }
}
