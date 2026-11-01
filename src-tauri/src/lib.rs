//! Tauri shell: wires audioshelf-core into commands and window events.
//! No business logic lives here; each command delegates to the core crate.

use audioshelf_core::{
    db::{self, with_db, Db},
    jobs::{ConvertOpts, EventSink, JobEvent, JobManager},
    library::{self, BookDetail, BookSummary},
    scan::{self, ScanReport},
    AppError,
};
use std::{path::PathBuf, sync::Arc};
use tauri::{AppHandle, Emitter, Manager, State};

pub const JOB_EVENT: &str = "job://progress";

pub struct AppState {
    pub db: Db,
    pub ffprobe: PathBuf,
    pub jobs: JobManager,
}

/// Forwards core job events to the WebView.
struct TauriSink(AppHandle);

impl EventSink for TauriSink {
    fn job_event(&self, event: JobEvent) {
        let _ = self.0.emit(JOB_EVENT, event);
    }
}

/// Prefers the sidecar next to the app binary (where Tauri places `externalBin`);
/// falls back to PATH for `tauri dev`.
pub fn bundled_binary(name: &str) -> PathBuf {
    let file = if cfg!(windows) {
        format!("{name}.exe")
    } else {
        name.to_owned()
    };
    std::env::current_exe()
        .ok()
        .and_then(|exe| exe.parent().map(|d| d.join(&file)))
        .filter(|p| p.exists())
        .unwrap_or_else(|| PathBuf::from(file))
}

#[tauri::command]
async fn scan_library(
    state: State<'_, AppState>,
    roots: Vec<PathBuf>,
) -> Result<ScanReport, AppError> {
    scan::scan(&state.db, &state.ffprobe, roots).await
}

#[tauri::command]
async fn search_books(
    state: State<'_, AppState>,
    query: String,
    offset: i64,
    limit: i64,
) -> Result<Vec<BookSummary>, AppError> {
    with_db(&state.db, move |c| {
        library::search(c, &query, offset, limit)
    })
    .await
}

#[tauri::command]
async fn get_book(state: State<'_, AppState>, id: i64) -> Result<Option<BookDetail>, AppError> {
    with_db(&state.db, move |c| library::detail(c, id)).await
}

#[tauri::command]
async fn enqueue_convert(
    state: State<'_, AppState>,
    book_ids: Vec<i64>,
    opts: ConvertOpts,
) -> Result<Vec<i64>, AppError> {
    state.jobs.enqueue(book_ids, opts).await
}

#[tauri::command]
fn cancel_job(state: State<'_, AppState>, job_id: i64) -> Result<(), AppError> {
    state.jobs.cancel(job_id)
}

pub fn run() {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()),
        )
        .init();

    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .setup(|app| {
            let data = app.path().app_data_dir()?;
            std::fs::create_dir_all(&data)?;
            let db = db::open(&data.join("library.sqlite"))?;
            let jobs = JobManager::new(
                db.clone(),
                Arc::new(TauriSink(app.handle().clone())),
                bundled_binary("ffmpeg"),
                data.join("scratch"),
                2,
            );
            app.manage(AppState {
                db,
                ffprobe: bundled_binary("ffprobe"),
                jobs,
            });
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            scan_library,
            search_books,
            get_book,
            enqueue_convert,
            cancel_job,
        ])
        .run(tauri::generate_context!())
        .expect("error while running AudioShelf");
}
