//! AudioShelf core: library database, folder scanning, media probing and
//! FFmpeg conversion jobs. Contains no UI code so it can be tested headless.

pub mod db;
pub mod error;
pub mod jobs;
pub mod library;
pub mod media;
pub mod scan;

pub use error::AppError;
pub use jobs::{ConvertOpts, EventSink, JobEvent, JobManager, JobStatus};
pub use media::probe::Chapter;
