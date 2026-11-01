//! Reads tags, duration and chapters with ffprobe's JSON output.

use serde::Deserialize;
use std::{collections::HashMap, path::Path, process::Stdio};
use tokio::process::Command;

#[derive(Debug, Clone, Default)]
pub struct Probe {
    pub title: Option<String>,
    pub author: Option<String>,
    pub narrator: Option<String>,
    pub album: Option<String>,
    pub series: Option<String>,
    pub year: Option<i64>,
    pub description: Option<String>,
    pub codec: Option<String>,
    pub bitrate_bps: Option<i64>,
    pub duration_ms: Option<i64>,
    pub chapters: Vec<Chapter>,
    pub has_cover: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Chapter {
    pub start_ms: i64,
    pub end_ms: i64,
    pub title: String,
}

#[derive(Deserialize)]
struct Raw {
    #[serde(default)]
    streams: Vec<RawStream>,
    format: RawFormat,
    #[serde(default)]
    chapters: Vec<RawChapter>,
}

#[derive(Deserialize)]
struct RawStream {
    codec_type: Option<String>,
    codec_name: Option<String>,
    #[serde(default)]
    disposition: HashMap<String, i64>,
}

#[derive(Deserialize)]
struct RawFormat {
    duration: Option<String>,
    bit_rate: Option<String>,
    #[serde(default)]
    tags: HashMap<String, String>,
}

#[derive(Deserialize)]
struct RawChapter {
    start_time: String,
    end_time: String,
    #[serde(default)]
    tags: HashMap<String, String>,
}

/// Case-insensitive tag lookup, since containers disagree on key casing and naming.
pub fn tag(tags: &HashMap<String, String>, keys: &[&str]) -> Option<String> {
    keys.iter().find_map(|k| {
        tags.iter()
            .find(|(tk, _)| tk.eq_ignore_ascii_case(k))
            .map(|(_, v)| v.trim().to_owned())
            .filter(|v| !v.is_empty())
    })
}

/// Parses ffprobe JSON. Separated from `probe` so it can be unit-tested without a binary.
pub fn parse(json: &[u8]) -> anyhow::Result<Probe> {
    let raw: Raw = serde_json::from_slice(json)?;
    let t = &raw.format.tags;
    let audio = raw
        .streams
        .iter()
        .find(|s| s.codec_type.as_deref() == Some("audio"));
    let has_cover = raw.streams.iter().any(|s| {
        s.codec_type.as_deref() == Some("video") && s.disposition.get("attached_pic") == Some(&1)
    });

    let chapters = raw
        .chapters
        .iter()
        .enumerate()
        .filter_map(|(i, c)| {
            let start: f64 = c.start_time.parse().ok()?;
            let end: f64 = c.end_time.parse().ok()?;
            Some(Chapter {
                start_ms: (start * 1000.0).round() as i64,
                end_ms: (end * 1000.0).round() as i64,
                title: tag(&c.tags, &["title"]).unwrap_or_else(|| format!("Chapter {}", i + 1)),
            })
        })
        .collect();

    Ok(Probe {
        title: tag(t, &["title"]),
        author: tag(t, &["artist", "album_artist"]),
        // Narrator is not a standard tag; composer is a common stand-in. Users can edit it later.
        narrator: tag(t, &["narrator", "composer"]),
        album: tag(t, &["album"]),
        series: tag(t, &["series", "show", "grouping"]),
        year: tag(t, &["date", "year"]).and_then(|d| d.get(..4)?.parse().ok()),
        description: tag(t, &["comment", "description", "synopsis"]),
        codec: audio.and_then(|a| a.codec_name.clone()),
        bitrate_bps: raw.format.bit_rate.as_deref().and_then(|b| b.parse().ok()),
        duration_ms: raw
            .format
            .duration
            .as_deref()
            .and_then(|d| d.parse::<f64>().ok())
            .map(|s| (s * 1000.0).round() as i64),
        chapters,
        has_cover,
    })
}

pub async fn probe(ffprobe: &Path, file: &Path) -> anyhow::Result<Probe> {
    let out = Command::new(ffprobe)
        .args([
            "-v",
            "error",
            "-print_format",
            "json",
            "-show_format",
            "-show_streams",
            "-show_chapters",
        ])
        .arg(file)
        .stdin(Stdio::null())
        .kill_on_drop(true)
        .output()
        .await?;
    anyhow::ensure!(
        out.status.success(),
        "ffprobe failed for {}: {}",
        file.display(),
        String::from_utf8_lossy(&out.stderr).trim()
    );
    parse(&out.stdout)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_tags_chapters_and_cover() {
        let json = br#"{
          "streams": [
            {"codec_type":"audio","codec_name":"aac"},
            {"codec_type":"video","codec_name":"mjpeg","disposition":{"attached_pic":1}}
          ],
          "format": {"duration":"12.5","bit_rate":"64000",
                     "tags":{"TITLE":"Dune","ARTIST":"Frank Herbert","date":"1965-08-01"}},
          "chapters": [
            {"start_time":"0.000000","end_time":"6.250000","tags":{"title":"One"}},
            {"start_time":"6.250000","end_time":"12.500000","tags":{}}
          ]
        }"#;
        let p = parse(json).unwrap();
        assert_eq!(p.title.as_deref(), Some("Dune"));
        assert_eq!(p.author.as_deref(), Some("Frank Herbert"));
        assert_eq!(p.year, Some(1965));
        assert_eq!(p.duration_ms, Some(12_500));
        assert_eq!(p.bitrate_bps, Some(64_000));
        assert!(p.has_cover);
        assert_eq!(
            p.chapters[0],
            Chapter {
                start_ms: 0,
                end_ms: 6250,
                title: "One".into()
            }
        );
        assert_eq!(p.chapters[1].title, "Chapter 2");
    }
}
