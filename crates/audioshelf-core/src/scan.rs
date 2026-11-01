//! Folder scanning. Walks roots, diffs against the database by (size, mtime),
//! probes changed folders, groups files into books, and persists in one transaction.
//!
//! Rule: one folder is one book. A single .m4b is a one-file book; multi-part
//! MP3s in one folder form one book whose chapters are offset by file duration.

use crate::{
    db::{now_ms, with_db, Db},
    error::AppError,
    media::probe::{self, Chapter, Probe},
};
use futures_util::{stream, StreamExt};
use rusqlite::{params, Connection};
use serde::Serialize;
use std::{
    collections::{BTreeMap, HashMap, HashSet},
    path::{Path, PathBuf},
    time::UNIX_EPOCH,
};
use walkdir::WalkDir;

pub const AUDIO_EXTENSIONS: [&str; 3] = ["m4b", "m4a", "mp3"];
const PROBE_CONCURRENCY: usize = 4;

#[derive(Debug, Serialize, Default, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ScanReport {
    pub files_seen: usize,
    pub files_probed: usize,
    pub books: usize,
}

#[derive(Clone, Debug)]
pub struct FileMeta {
    pub path: PathBuf,
    pub size: i64,
    pub mtime_ms: i64,
}

fn key(p: &Path) -> String {
    p.to_string_lossy().into_owned()
}

fn stem(p: &Path) -> String {
    p.file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("Untitled")
        .to_owned()
}

pub fn is_audio(path: &Path) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| AUDIO_EXTENSIONS.iter().any(|x| x.eq_ignore_ascii_case(e)))
}

/// Pure filesystem walk. Symlinks are not followed, which avoids loops.
pub fn walk(roots: &[PathBuf]) -> Vec<FileMeta> {
    let mut out = Vec::new();
    for root in roots {
        for entry in WalkDir::new(root)
            .follow_links(false)
            .into_iter()
            .filter_map(Result::ok)
        {
            if !entry.file_type().is_file() || !is_audio(entry.path()) {
                continue;
            }
            let Ok(md) = entry.metadata() else { continue };
            let mtime_ms = md
                .modified()
                .ok()
                .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
                .map_or(0, |d| d.as_millis() as i64);
            out.push(FileMeta {
                path: entry.path().to_path_buf(),
                size: md.len() as i64,
                mtime_ms,
            });
        }
    }
    out
}

/// Folders that contain at least one new or changed file. Those folders are
/// re-probed in full so book metadata and chapter offsets are rebuilt from complete data.
pub fn dirty_folders(files: &[FileMeta], known: &HashMap<String, (i64, i64)>) -> HashSet<String> {
    files
        .iter()
        .filter(|f| known.get(&key(&f.path)) != Some(&(f.size, f.mtime_ms)))
        .filter_map(|f| f.path.parent().map(key))
        .collect()
}

pub async fn scan(db: &Db, ffprobe: &Path, roots: Vec<PathBuf>) -> Result<ScanReport, AppError> {
    let files = tokio::task::spawn_blocking(move || walk(&roots)).await?;

    let known: HashMap<String, (i64, i64)> = with_db(db, |c| {
        c.prepare("SELECT path, size_bytes, mtime_ms FROM book_files")?
            .query_map([], |r| Ok((r.get(0)?, (r.get(1)?, r.get(2)?))))?
            .collect()
    })
    .await?;

    let dirty = dirty_folders(&files, &known);
    let to_probe: Vec<FileMeta> = files
        .iter()
        .filter(|f| f.path.parent().map(key).is_some_and(|d| dirty.contains(&d)))
        .cloned()
        .collect();

    let ffprobe = ffprobe.to_path_buf();
    let probed: Vec<(FileMeta, Probe)> = stream::iter(to_probe)
        .map(|f| {
            let ffprobe = ffprobe.clone();
            async move {
                let p = probe::probe(&ffprobe, &f.path).await;
                (f, p)
            }
        })
        .buffer_unordered(PROBE_CONCURRENCY)
        .filter_map(|(f, p)| async move {
            match p {
                Ok(p) => Some((f, p)),
                Err(e) => {
                    tracing::warn!("skipping unreadable file: {e:#}");
                    None
                }
            }
        })
        .collect()
        .await;

    let seen: Vec<String> = files.iter().map(|f| key(&f.path)).collect();
    let seen_json = serde_json::to_string(&seen).map_err(|e| AppError::Internal(e.to_string()))?;
    let files_probed = probed.len();
    let books = with_db(db, move |c| persist(c, &seen_json, probed)).await?;

    Ok(ScanReport {
        files_seen: files.len(),
        files_probed,
        books,
    })
}

/// Writes books, files and chapters. Returns the total number of books afterwards.
pub fn persist(
    c: &mut Connection,
    seen_json: &str,
    probed: Vec<(FileMeta, Probe)>,
) -> rusqlite::Result<usize> {
    let tx = c.transaction()?;
    let now = now_ms();

    // Files that vanished are flagged, never deleted, so user data survives moves and drive swaps.
    tx.execute(
        "UPDATE book_files SET missing = 1 WHERE path NOT IN (SELECT value FROM json_each(?1))",
        [seen_json],
    )?;

    let mut groups: BTreeMap<String, Vec<(FileMeta, Probe)>> = BTreeMap::new();
    for (f, p) in probed {
        let dir = f.path.parent().map(key).unwrap_or_default();
        groups.entry(dir).or_default().push((f, p));
    }

    for (dir, mut parts) in groups {
        parts.sort_by(|a, b| a.0.path.cmp(&b.0.path));
        let first = &parts[0].1;
        // Multi-part MP3s carry the book title in the album tag; single files use title.
        let title = first
            .album
            .clone()
            .filter(|_| parts.len() > 1)
            .or_else(|| first.title.clone())
            .unwrap_or_else(|| stem(&parts[0].0.path));
        let total_ms: i64 = parts.iter().map(|(_, p)| p.duration_ms.unwrap_or(0)).sum();

        let book_id: i64 = tx.query_row(
            "INSERT INTO books (group_key, title, author, narrator, series, year, description,
                                codec, bitrate_bps, duration_ms, missing, added_at, updated_at)
             VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,0,?11,?11)
             ON CONFLICT(group_key) DO UPDATE SET
               title=excluded.title, author=excluded.author, narrator=excluded.narrator,
               series=excluded.series, year=excluded.year, description=excluded.description,
               codec=excluded.codec, bitrate_bps=excluded.bitrate_bps,
               duration_ms=excluded.duration_ms, missing=0, updated_at=excluded.updated_at
             RETURNING id",
            params![
                dir,
                title,
                first.author.clone().unwrap_or_default(),
                first.narrator.clone().unwrap_or_default(),
                first.series,
                first.year,
                first.description,
                first.codec,
                first.bitrate_bps,
                total_ms,
                now,
            ],
            |r| r.get(0),
        )?;

        tx.execute("DELETE FROM chapters WHERE book_id = ?1", [book_id])?;
        let mut offset = 0i64;
        let mut idx = 0i64;
        for (pos, (f, p)) in parts.iter().enumerate() {
            let len = p.duration_ms.unwrap_or(0);
            tx.execute(
                "INSERT INTO book_files (book_id, path, position, size_bytes, mtime_ms, duration_ms, missing)
                 VALUES (?1,?2,?3,?4,?5,?6,0)
                 ON CONFLICT(path) DO UPDATE SET book_id=excluded.book_id, position=excluded.position,
                   size_bytes=excluded.size_bytes, mtime_ms=excluded.mtime_ms,
                   duration_ms=excluded.duration_ms, missing=0",
                params![book_id, key(&f.path), pos as i64, f.size, f.mtime_ms, p.duration_ms],
            )?;
            // Embedded chapters win; otherwise each file becomes one chapter.
            let chapters = if p.chapters.is_empty() {
                vec![Chapter {
                    start_ms: 0,
                    end_ms: len,
                    title: stem(&f.path),
                }]
            } else {
                p.chapters.clone()
            };
            for ch in chapters {
                tx.execute(
                    "INSERT INTO chapters (book_id, idx, start_ms, end_ms, title) VALUES (?1,?2,?3,?4,?5)",
                    params![book_id, idx, offset + ch.start_ms, offset + ch.end_ms, ch.title],
                )?;
                idx += 1;
            }
            offset += len;
        }
    }

    // A book is missing only when none of its files are on disk.
    tx.execute(
        "UPDATE books SET missing = CASE WHEN EXISTS (
             SELECT 1 FROM book_files f WHERE f.book_id = books.id AND f.missing = 0
         ) THEN 0 ELSE 1 END",
        [],
    )?;
    tx.execute(
        "DELETE FROM books WHERE id NOT IN (SELECT book_id FROM book_files)",
        [],
    )?;
    let count: i64 = tx.query_row("SELECT COUNT(*) FROM books", [], |r| r.get(0))?;
    tx.commit()?;
    Ok(count as usize)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn meta(path: &str, size: i64, mtime: i64) -> FileMeta {
        FileMeta {
            path: PathBuf::from(path),
            size,
            mtime_ms: mtime,
        }
    }

    #[test]
    fn dirty_folder_pulls_in_whole_folder() {
        let files = vec![
            meta("/lib/a/part1.mp3", 10, 1),
            meta("/lib/a/part2.mp3", 10, 1),
            meta("/lib/b/x.m4b", 5, 1),
        ];
        let mut known = HashMap::new();
        known.insert("/lib/a/part1.mp3".to_string(), (10, 1));
        known.insert("/lib/a/part2.mp3".to_string(), (10, 1));
        known.insert("/lib/b/x.m4b".to_string(), (5, 1));
        assert!(dirty_folders(&files, &known).is_empty());

        // Only part2 changed, so its whole folder is dirty but folder b is untouched.
        known.insert("/lib/a/part2.mp3".to_string(), (9, 1));
        let dirty = dirty_folders(&files, &known);
        assert_eq!(dirty, HashSet::from(["/lib/a".to_string()]));
    }

    #[test]
    fn audio_extension_check_is_case_insensitive() {
        assert!(is_audio(Path::new("Book.M4B")));
        assert!(!is_audio(Path::new("cover.jpg")));
    }
}
