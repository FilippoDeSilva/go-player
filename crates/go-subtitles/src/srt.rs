//! SRT (SubRip) parsing and serialization.
//!
//! Format: blocks of `index` / `HH:MM:SS,mmm --> HH:MM:SS,mmm` / text,
//! separated by blank lines. Tolerates BOM, CRLF, and missing indices.

use crate::model::{SubtitleCue, SubtitleDocument};
use std::time::Duration;

use crate::SubtitleError;

/// Parse SRT text into a canonical document. Does not sort or validate;
/// call [`SubtitleDocument::validate`] or [`SubtitleDocument::sort_by_start`].
pub fn parse_srt(input: &str) -> Result<SubtitleDocument, SubtitleError> {
    let input = input.strip_prefix('\u{feff}').unwrap_or(input);
    let mut doc = SubtitleDocument::new();
    let mut block: Vec<&str> = Vec::new();

    let flush = |block: &mut Vec<&str>, doc: &mut SubtitleDocument| -> Result<(), SubtitleError> {
        if block.is_empty() {
            return Ok(());
        }
        let lines: Vec<&str> = std::mem::take(block);
        doc.cues.push(parse_srt_block(&lines)?);
        Ok(())
    };

    for line in input.lines() {
        if line.trim().is_empty() {
            flush(&mut block, &mut doc)?;
        } else {
            block.push(line.trim_end_matches('\r'));
        }
    }
    flush(&mut block, &mut doc)?;
    Ok(doc)
}

fn parse_srt_block(lines: &[&str]) -> Result<SubtitleCue, SubtitleError> {
    // Expected: [index?] timing text...
    let (timing_idx, index_id) = match lines.first() {
        Some(first) if first.contains("-->") => (0usize, None),
        Some(_) => (
            1usize,
            lines[0].trim().parse::<u32>().ok().map(|n| n.to_string()),
        ),
        None => return Err(SubtitleError::EmptyBlock),
    };

    let timing_line = lines.get(timing_idx).ok_or(SubtitleError::MissingTiming)?;
    let (start, end) = parse_srt_timing(timing_line)?;

    let text = lines[timing_idx + 1..].join("\n");
    if text.trim().is_empty() {
        return Err(SubtitleError::EmptyCueText);
    }

    Ok(SubtitleCue {
        id: index_id,
        start,
        end,
        text,
    })
}

/// `HH:MM:SS,mmm --> HH:MM:SS,mmm` with optional trailing positioning.
pub fn parse_srt_timing(line: &str) -> Result<(Duration, Duration), SubtitleError> {
    let arrow = line
        .find("-->")
        .ok_or_else(|| SubtitleError::BadTiming(line.to_string()))?;
    let start = parse_srt_timestamp(line[..arrow].trim())?;
    // Trailing settings after the end timestamp (e.g. "X1:.. X2:..") are ignored.
    let end_part = line[arrow + 3..].trim();
    let end_token = end_part.split_whitespace().next().unwrap_or("");
    let end = parse_srt_timestamp(end_token)?;
    Ok((start, end))
}

fn parse_srt_timestamp(ts: &str) -> Result<Duration, SubtitleError> {
    // HH:MM:SS,mmm — hours may exceed 99.
    let bad = || SubtitleError::BadTimestamp(ts.to_string());
    let (hms, ms) = ts.split_once(',').ok_or_else(bad)?;
    let mut parts = hms.split(':');
    let h: u64 = parts.next().and_then(|p| p.parse().ok()).ok_or_else(bad)?;
    let m: u64 = parts.next().and_then(|p| p.parse().ok()).ok_or_else(bad)?;
    let s: u64 = parts.next().and_then(|p| p.parse().ok()).ok_or_else(bad)?;
    if parts.next().is_some() || m >= 60 || s >= 60 {
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

pub fn format_srt_timestamp(d: Duration) -> String {
    let ms_total = d.as_millis();
    let h = ms_total / 3_600_000;
    let m = (ms_total % 3_600_000) / 60_000;
    let s = (ms_total % 60_000) / 1000;
    let ms = ms_total % 1000;
    format!("{h:02}:{m:02}:{s:02},{ms:03}")
}

/// Serialize a document to SRT. Cues are written in stored order.
pub fn to_srt(doc: &SubtitleDocument) -> String {
    let mut out = String::new();
    for (i, cue) in doc.cues.iter().enumerate() {
        let index = cue
            .id
            .as_deref()
            .and_then(|s| s.parse::<u32>().ok())
            .unwrap_or(i as u32 + 1);
        out.push_str(&format!(
            "{index}\n{} --> {}\n{}\n\n",
            format_srt_timestamp(cue.start),
            format_srt_timestamp(cue.end),
            cue.text
        ));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = "1\n00:00:01,000 --> 00:00:04,000\nHello world\n\n2\n00:00:05,500 --> 00:00:08,250\nSecond line\nstill second\n";

    #[test]
    fn parses_basic_srt() {
        let doc = parse_srt(SAMPLE).unwrap();
        assert_eq!(doc.cues.len(), 2);
        assert_eq!(doc.cues[0].start, Duration::from_millis(1000));
        assert_eq!(doc.cues[0].end, Duration::from_millis(4000));
        assert_eq!(doc.cues[0].text, "Hello world");
        assert_eq!(doc.cues[1].text, "Second line\nstill second");
        assert!(doc.validate().is_ok());
    }

    #[test]
    fn handles_bom_and_crlf() {
        let input = "\u{feff}1\r\n00:00:01,000 --> 00:00:02,000\r\nHi\r\n";
        let doc = parse_srt(input).unwrap();
        assert_eq!(doc.cues.len(), 1);
    }

    #[test]
    fn roundtrips() {
        let doc = parse_srt(SAMPLE).unwrap();
        let srt = to_srt(&doc);
        let doc2 = parse_srt(&srt).unwrap();
        assert_eq!(doc.cues.len(), doc2.cues.len());
        assert_eq!(doc.cues[0].start, doc2.cues[0].start);
        assert_eq!(doc.cues[1].text, doc2.cues[1].text);
    }

    #[test]
    fn rejects_missing_timing() {
        assert!(parse_srt("1\nno timing here\n").is_err());
    }

    #[test]
    fn ignores_trailing_positioning() {
        let doc = parse_srt("1\n00:00:01,000 --> 00:00:02,000 X1:10 X2:20\nHi\n").unwrap();
        assert_eq!(doc.cues[0].end, Duration::from_millis(2000));
    }
}
