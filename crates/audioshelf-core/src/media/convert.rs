//! FFmpeg conversion to M4B with chapters, progress and cancellation.
//! Writes to `<output>.part` and renames on success, so a crash never leaves a
//! plausible-looking half file behind.

use super::probe::Chapter;
use std::{
    ffi::OsString,
    path::{Path, PathBuf},
    process::Stdio,
    time::{Duration, Instant},
};
use tokio::{
    io::{AsyncBufReadExt, AsyncReadExt, BufReader},
    process::Command,
};
use tokio_util::sync::CancellationToken;

#[derive(Debug, PartialEq, Eq)]
pub enum Outcome {
    Done,
    Cancelled,
}

pub struct ConvertSpec<'a> {
    /// One file, or several parts joined by the concat demuxer.
    pub inputs: &'a [PathBuf],
    /// FFMETADATA1 file carrying chapters and tags.
    pub metadata: &'a Path,
    pub output: &'a Path,
    pub duration_ms: i64,
    pub bitrate_kbps: u32,
    pub work_dir: &'a Path,
}

/// Builds an FFMETADATA1 document with escaped values and one atom per chapter.
pub fn ffmetadata(title: &str, artist: &str, chapters: &[Chapter]) -> String {
    fn esc(s: &str) -> String {
        let mut o = String::with_capacity(s.len());
        for c in s.chars() {
            if matches!(c, '=' | ';' | '#' | '\\' | '\n') {
                o.push('\\');
            }
            o.push(c);
        }
        o
    }
    let mut out = format!(
        ";FFMETADATA1\ntitle={}\nartist={}\n",
        esc(title),
        esc(artist)
    );
    for c in chapters {
        out += &format!(
            "\n[CHAPTER]\nTIMEBASE=1/1000\nSTART={}\nEND={}\ntitle={}\n",
            c.start_ms,
            c.end_ms,
            esc(&c.title)
        );
    }
    out
}

async fn input_args(inputs: &[PathBuf], work: &Path) -> std::io::Result<Vec<OsString>> {
    if let [single] = inputs {
        return Ok(vec!["-i".into(), single.into()]);
    }
    let list = work.join("concat.txt");
    let body: String = inputs
        .iter()
        .map(|p| format!("file '{}'\n", p.to_string_lossy().replace('\'', r"'\''")))
        .collect();
    tokio::fs::write(&list, body).await?;
    Ok(vec![
        "-f".into(),
        "concat".into(),
        "-safe".into(),
        "0".into(),
        "-i".into(),
        list.into(),
    ])
}

pub async fn run(
    ffmpeg: &Path,
    spec: ConvertSpec<'_>,
    cancel: &CancellationToken,
    mut on_progress: impl FnMut(f32),
) -> anyhow::Result<Outcome> {
    let tmp = PathBuf::from(format!("{}.part", spec.output.display()));
    let inputs = input_args(spec.inputs, spec.work_dir).await?;

    let mut child = Command::new(ffmpeg)
        .args([
            "-hide_banner",
            "-nostdin",
            "-y",
            "-loglevel",
            "error",
            "-nostats",
            "-progress",
            "pipe:1",
        ])
        .args(&inputs)
        .arg("-i")
        .arg(spec.metadata)
        .args([
            "-map",
            "0:a",
            "-map_metadata",
            "1",
            "-map_chapters",
            "1",
            "-vn",
        ])
        .args(["-c:a", "aac", "-b:a"])
        .arg(format!("{}k", spec.bitrate_kbps))
        .args(["-movflags", "+faststart", "-f", "ipod"])
        .arg(&tmp)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true)
        .spawn()?;

    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| anyhow::anyhow!("ffmpeg has no stdout"))?;
    let mut stderr = child
        .stderr
        .take()
        .ok_or_else(|| anyhow::anyhow!("ffmpeg has no stderr"))?;
    // Drain stderr concurrently so ffmpeg can never block on a full pipe.
    let stderr_task = tokio::spawn(async move {
        let mut s = String::new();
        let _ = stderr.read_to_string(&mut s).await;
        s
    });

    let total_us = (spec.duration_ms.max(1) * 1000) as f64;
    let mut lines = BufReader::new(stdout).lines();
    let mut last_emit = Instant::now() - Duration::from_secs(1);

    loop {
        tokio::select! {
            _ = cancel.cancelled() => {
                let _ = child.start_kill();
                let _ = child.wait().await;
                let _ = tokio::fs::remove_file(&tmp).await;
                return Ok(Outcome::Cancelled);
            }
            line = lines.next_line() => match line? {
                None => break,
                Some(l) => {
                    if let Some(us) = l.strip_prefix("out_time_us=").and_then(|v| v.parse::<f64>().ok()) {
                        if last_emit.elapsed() >= Duration::from_millis(250) {
                            on_progress(((us / total_us) as f32).clamp(0.0, 1.0));
                            last_emit = Instant::now();
                        }
                    }
                }
            }
        }
    }

    let status = child.wait().await?;
    let stderr_text = stderr_task.await.unwrap_or_default();
    if !status.success() {
        let _ = tokio::fs::remove_file(&tmp).await;
        anyhow::bail!("ffmpeg exited with {status}: {}", stderr_text.trim());
    }
    tokio::fs::rename(&tmp, spec.output).await?;
    on_progress(1.0);
    Ok(Outcome::Done)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn metadata_escapes_reserved_characters() {
        let ch = [Chapter {
            start_ms: 0,
            end_ms: 1000,
            title: "a=b;c#d".into(),
        }];
        let text = ffmetadata("T", "A", &ch);
        assert!(text.contains(r"title=a\=b\;c\#d"));
        assert!(text.starts_with(";FFMETADATA1"));
        assert!(text.contains("START=0\nEND=1000"));
    }
}
