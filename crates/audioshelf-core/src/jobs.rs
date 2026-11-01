//! Conversion job queue: bounded concurrency, progress events and cancellation.
//! The core reports progress through the `EventSink` trait, so a UI can plug in
//! Tauri events, a terminal, or a test recorder without the core knowing about it.

use crate::{
    db::{now_ms, with_db, Db},
    error::AppError,
    media::{
        convert::{self, ConvertSpec, Outcome},
        probe::Chapter,
    },
};
use parking_lot::Mutex;
use rusqlite::params;
use serde::{Deserialize, Serialize};
use std::{collections::HashMap, path::PathBuf, sync::Arc};
use tokio::sync::Semaphore;
use tokio_util::sync::CancellationToken;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum JobStatus {
    Queued,
    Running,
    Done,
    Failed,
    Cancelled,
}

impl JobStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            JobStatus::Queued => "queued",
            JobStatus::Running => "running",
            JobStatus::Done => "done",
            JobStatus::Failed => "failed",
            JobStatus::Cancelled => "cancelled",
        }
    }
}

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct JobEvent {
    pub id: i64,
    pub status: JobStatus,
    /// 0.0 to 1.0.
    pub progress: f32,
    pub error: Option<String>,
}

pub trait EventSink: Send + Sync + 'static {
    fn job_event(&self, event: JobEvent);
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ConvertOpts {
    pub out_dir: PathBuf,
    pub bitrate_kbps: u32,
    pub overwrite: bool,
}

struct Inner {
    db: Db,
    sink: Arc<dyn EventSink>,
    ffmpeg: PathBuf,
    scratch: PathBuf,
    slots: Semaphore,
    cancels: Mutex<HashMap<i64, CancellationToken>>,
}

#[derive(Clone)]
pub struct JobManager(Arc<Inner>);

struct BookForConvert {
    title: String,
    author: String,
    duration_ms: i64,
    files: Vec<PathBuf>,
    chapters: Vec<Chapter>,
}

impl JobManager {
    /// `workers` bounds concurrent ffmpeg processes. Each ffmpeg already uses several cores.
    pub fn new(
        db: Db,
        sink: Arc<dyn EventSink>,
        ffmpeg: PathBuf,
        scratch: PathBuf,
        workers: usize,
    ) -> Self {
        Self(Arc::new(Inner {
            db,
            sink,
            ffmpeg,
            scratch,
            slots: Semaphore::new(workers.max(1)),
            cancels: Mutex::new(HashMap::new()),
        }))
    }

    /// Queues one conversion per book and returns job ids in the same order.
    pub async fn enqueue(
        &self,
        book_ids: Vec<i64>,
        opts: ConvertOpts,
    ) -> Result<Vec<i64>, AppError> {
        let mut ids = Vec::with_capacity(book_ids.len());
        for book_id in book_ids {
            let job_id = with_db(&self.0.db, move |c| {
                c.execute(
                    "INSERT INTO jobs (book_id, kind, status, created_at) VALUES (?1, 'convert', 'queued', ?2)",
                    params![book_id, now_ms()],
                )?;
                Ok(c.last_insert_rowid())
            })
            .await?;

            let token = CancellationToken::new();
            self.0.cancels.lock().insert(job_id, token.clone());
            self.emit(job_id, JobStatus::Queued, 0.0, None);

            let this = self.clone();
            let opts = opts.clone();
            tokio::spawn(async move { this.run(job_id, book_id, opts, token).await });
            ids.push(job_id);
        }
        Ok(ids)
    }

    /// Cancels a queued or running job. Returns an error if it already finished.
    pub fn cancel(&self, job_id: i64) -> Result<(), AppError> {
        match self.0.cancels.lock().get(&job_id) {
            Some(token) => {
                token.cancel();
                Ok(())
            }
            None => Err(AppError::Internal(format!("job {job_id} is not active"))),
        }
    }

    async fn run(&self, job_id: i64, book_id: i64, opts: ConvertOpts, cancel: CancellationToken) {
        let (status, output, error) = match self.execute(job_id, book_id, &opts, &cancel).await {
            Ok(Some(path)) => (JobStatus::Done, Some(path.display().to_string()), None),
            Ok(None) => (JobStatus::Cancelled, None, None),
            Err(e) => {
                tracing::error!(job_id, "conversion failed: {e}");
                (JobStatus::Failed, None, Some(e.to_string()))
            }
        };

        let error_for_db = error.clone();
        let _ = with_db(&self.0.db, move |c| {
            c.execute(
                "UPDATE jobs SET status = ?2, output_path = ?3, error = ?4, finished_at = ?5,
                   progress = CASE WHEN ?2 = 'done' THEN 1 ELSE progress END
                 WHERE id = ?1",
                params![job_id, status.as_str(), output, error_for_db, now_ms()],
            )
        })
        .await;

        self.0.cancels.lock().remove(&job_id);
        let progress = if status == JobStatus::Done { 1.0 } else { 0.0 };
        self.emit(job_id, status, progress, error);
    }

    /// `Ok(Some(path))` on success, `Ok(None)` when cancelled.
    async fn execute(
        &self,
        job_id: i64,
        book_id: i64,
        opts: &ConvertOpts,
        cancel: &CancellationToken,
    ) -> Result<Option<PathBuf>, AppError> {
        let _slot = tokio::select! {
            permit = self.0.slots.acquire() => permit.map_err(|e| AppError::Internal(e.to_string()))?,
            _ = cancel.cancelled() => return Ok(None),
        };
        self.emit(job_id, JobStatus::Running, 0.0, None);

        let book = load_book(&self.0.db, book_id).await?;
        if book.files.is_empty() {
            return Err(AppError::Internal(
                "no source files are present on disk".into(),
            ));
        }

        let target = opts
            .out_dir
            .join(sanitize(&book.author, "Unknown Author"))
            .join(format!("{}.m4b", sanitize(&book.title, "Untitled")));
        let out = if opts.overwrite {
            target
        } else {
            unique_path(target)
        };
        if let Some(parent) = out.parent() {
            tokio::fs::create_dir_all(parent).await?;
        }

        let work = self.0.scratch.join(format!("job-{job_id}"));
        tokio::fs::create_dir_all(&work).await?;
        let meta = work.join("chapters.ffmeta");
        tokio::fs::write(
            &meta,
            convert::ffmetadata(&book.title, &book.author, &book.chapters),
        )
        .await?;

        let me = self.clone();
        let result = convert::run(
            &self.0.ffmpeg,
            ConvertSpec {
                inputs: &book.files,
                metadata: &meta,
                output: &out,
                duration_ms: book.duration_ms,
                bitrate_kbps: opts.bitrate_kbps,
                work_dir: &work,
            },
            cancel,
            move |p| me.emit(job_id, JobStatus::Running, p, None),
        )
        .await;

        let _ = tokio::fs::remove_dir_all(&work).await;
        match result? {
            Outcome::Done => Ok(Some(out)),
            Outcome::Cancelled => Ok(None),
        }
    }

    fn emit(&self, id: i64, status: JobStatus, progress: f32, error: Option<String>) {
        self.0.sink.job_event(JobEvent {
            id,
            status,
            progress,
            error,
        });
    }
}

async fn load_book(db: &Db, book_id: i64) -> Result<BookForConvert, AppError> {
    with_db(db, move |c| {
        let (title, author, duration_ms): (String, String, i64) = c.query_row(
            "SELECT title, author, COALESCE(duration_ms, 0) FROM books WHERE id = ?1",
            [book_id],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )?;
        let files = c
            .prepare(
                "SELECT path FROM book_files WHERE book_id = ?1 AND missing = 0 ORDER BY position",
            )?
            .query_map([book_id], |r| r.get::<_, String>(0).map(PathBuf::from))?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        let chapters = c
            .prepare(
                "SELECT start_ms, end_ms, title FROM chapters WHERE book_id = ?1 ORDER BY idx",
            )?
            .query_map([book_id], |r| {
                Ok(Chapter {
                    start_ms: r.get(0)?,
                    end_ms: r.get(1)?,
                    title: r.get(2)?,
                })
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(BookForConvert {
            title,
            author,
            duration_ms,
            files,
            chapters,
        })
    })
    .await
}

/// Strips path separators and characters Windows forbids, and trims trailing dots.
pub fn sanitize(s: &str, fallback: &str) -> String {
    let cleaned: String = s
        .chars()
        .map(|c| {
            if c.is_control() || "/\\:*?\"<>|".contains(c) {
                '_'
            } else {
                c
            }
        })
        .collect();
    let trimmed: String = cleaned
        .trim()
        .trim_end_matches('.')
        .chars()
        .take(180)
        .collect();
    if trimmed.is_empty() {
        fallback.to_owned()
    } else {
        trimmed
    }
}

/// Appends " (2)", " (3)"... rather than overwriting an existing file.
pub fn unique_path(p: PathBuf) -> PathBuf {
    if !p.exists() {
        return p;
    }
    let stem = p
        .file_stem()
        .unwrap_or_default()
        .to_string_lossy()
        .into_owned();
    (2..)
        .map(|n| p.with_file_name(format!("{stem} ({n}).m4b")))
        .find(|c| !c.exists())
        .expect("unbounded range")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sanitize_removes_forbidden_characters() {
        assert_eq!(sanitize("a/b:c*?", "x"), "a_b_c__");
        assert_eq!(sanitize("  name. ", "x"), "name");
        assert_eq!(sanitize("...", "Untitled"), "Untitled");
    }

    #[test]
    fn unique_path_adds_a_counter() {
        let dir = std::env::temp_dir().join(format!("as-unique-{}", now_ms()));
        std::fs::create_dir_all(&dir).unwrap();
        let first = dir.join("Book.m4b");
        std::fs::write(&first, b"x").unwrap();
        assert_eq!(unique_path(first.clone()), dir.join("Book (2).m4b"));
        assert_eq!(unique_path(dir.join("Free.m4b")), dir.join("Free.m4b"));
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
