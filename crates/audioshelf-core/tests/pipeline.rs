//! End-to-end tests against the real ffmpeg/ffprobe binaries on PATH.
//! Fixtures are generated on the fly (sine tones), so no audio files are committed.

use audioshelf_core::{
    db::{self, with_db},
    jobs::{ConvertOpts, EventSink, JobEvent, JobManager, JobStatus},
    library,
    media::{
        convert::ffmetadata,
        probe::{self, Chapter},
    },
    scan,
};
use parking_lot::Mutex;
use std::{
    path::{Path, PathBuf},
    process::Command,
    sync::Arc,
    time::Duration,
};

#[derive(Default)]
struct Recorder(Mutex<Vec<JobEvent>>);

impl EventSink for Recorder {
    fn job_event(&self, event: JobEvent) {
        self.0.lock().push(event);
    }
}

impl Recorder {
    fn wait_terminal(&self, id: i64) -> JobEvent {
        for _ in 0..600 {
            if let Some(e) =
                self.0.lock().iter().rev().find(|e| {
                    e.id == id && !matches!(e.status, JobStatus::Queued | JobStatus::Running)
                })
            {
                return e.clone();
            }
            std::thread::sleep(Duration::from_millis(100));
        }
        panic!("job {id} did not finish in time");
    }
}

fn ffmpeg(args: &[&str]) {
    let st = Command::new("ffmpeg")
        .args(["-hide_banner", "-loglevel", "error", "-y"])
        .args(args)
        .status()
        .expect("ffmpeg must be on PATH");
    assert!(st.success(), "ffmpeg failed: {args:?}");
}

/// M4A/M4B with a sine tone of `secs` seconds and chapter metadata.
fn make_m4a(path: &Path, secs: u32, title: &str, author: &str, chapters: &[Chapter]) {
    let meta = path.with_extension("ffmeta");
    std::fs::write(&meta, ffmetadata(title, author, chapters)).unwrap();
    ffmpeg(&[
        "-f",
        "lavfi",
        "-i",
        &format!("sine=frequency=330:duration={secs}"),
        "-i",
        meta.to_str().unwrap(),
        "-map",
        "0:a",
        "-map_metadata",
        "1",
        "-map_chapters",
        "1",
        "-c:a",
        "aac",
        "-b:a",
        "64k",
        path.to_str().unwrap(),
    ]);
}

fn make_mp3(path: &Path, secs: u32, album: &str, title: &str) {
    ffmpeg(&[
        "-f",
        "lavfi",
        "-i",
        &format!("sine=frequency=220:duration={secs}"),
        "-metadata",
        &format!("album={album}"),
        "-metadata",
        &format!("title={title}"),
        "-metadata",
        "artist=Test Author",
        "-c:a",
        "libmp3lame",
        "-b:a",
        "64k",
        path.to_str().unwrap(),
    ]);
}

fn ffprobe_path() -> PathBuf {
    PathBuf::from("ffprobe")
}

struct Harness {
    db: db::Db,
    jobs: JobManager,
    sink: Arc<Recorder>,
    _tmp: tempfile::TempDir,
    root: PathBuf,
    out: PathBuf,
}

fn harness() -> Harness {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().join("library");
    let out = tmp.path().join("converted");
    std::fs::create_dir_all(&root).unwrap();
    let db = db::open(&tmp.path().join("lib.sqlite")).unwrap();
    let sink = Arc::new(Recorder::default());
    let jobs = JobManager::new(
        db.clone(),
        sink.clone(),
        PathBuf::from("ffmpeg"),
        tmp.path().join("scratch"),
        2,
    );
    Harness {
        db,
        jobs,
        sink,
        _tmp: tmp,
        root,
        out,
    }
}

fn opts(h: &Harness) -> ConvertOpts {
    ConvertOpts {
        out_dir: h.out.clone(),
        bitrate_kbps: 48,
        overwrite: false,
    }
}

async fn ms_of(path: &Path) -> i64 {
    probe::probe(&ffprobe_path(), path)
        .await
        .unwrap()
        .duration_ms
        .unwrap()
}

async fn scan_root(h: &Harness) -> scan::ScanReport {
    scan::scan(&h.db, &ffprobe_path(), vec![h.root.clone()])
        .await
        .unwrap()
}

#[tokio::test(flavor = "multi_thread")]
async fn scan_then_convert_single_m4a_keeps_chapters() {
    let h = harness();
    let dir = h.root.join("Dune");
    std::fs::create_dir_all(&dir).unwrap();
    make_m4a(
        &dir.join("dune.m4a"),
        4,
        "Dune",
        "Frank Herbert",
        &[
            Chapter {
                start_ms: 0,
                end_ms: 2000,
                title: "Book I".into(),
            },
            Chapter {
                start_ms: 2000,
                end_ms: 4000,
                title: "Book II".into(),
            },
        ],
    );

    let report = scan::scan(&h.db, &ffprobe_path(), vec![h.root.clone()])
        .await
        .unwrap();
    assert_eq!(
        report,
        scan::ScanReport {
            files_seen: 1,
            files_probed: 1,
            books: 1
        }
    );

    let hits = with_db(&h.db, |c| library::search(c, "herbert", 0, 10))
        .await
        .unwrap();
    assert_eq!(hits.len(), 1);
    let book_id = hits[0].id;

    let ids = h.jobs.enqueue(vec![book_id], opts(&h)).await.unwrap();
    let done = tokio::task::spawn_blocking({
        let sink = h.sink.clone();
        move || sink.wait_terminal(ids[0])
    })
    .await
    .unwrap();
    assert_eq!(done.status, JobStatus::Done, "error: {:?}", done.error);

    let output = h.out.join("Frank Herbert").join("Dune.m4b");
    assert!(output.exists(), "expected {}", output.display());
    assert!(!PathBuf::from(format!("{}.part", output.display())).exists());

    let p = probe::probe(&ffprobe_path(), &output).await.unwrap();
    assert!(
        (p.duration_ms.unwrap() - 4000).abs() < 150,
        "duration {:?}",
        p.duration_ms
    );
    assert_eq!(p.chapters.len(), 2);
    assert_eq!(p.chapters[1].title, "Book II");
    assert!((p.chapters[1].start_ms - 2000).abs() < 150);
    assert_eq!(p.title.as_deref(), Some("Dune"));
}

#[tokio::test(flavor = "multi_thread")]
async fn multi_part_mp3_folder_becomes_one_book_with_offset_chapters() {
    let h = harness();
    let dir = h.root.join("Hobbit");
    std::fs::create_dir_all(&dir).unwrap();
    make_mp3(&dir.join("01 part.mp3"), 2, "The Hobbit", "Part 1");
    make_mp3(&dir.join("02 part.mp3"), 3, "The Hobbit", "Part 2");

    let report = scan_root(&h).await;
    assert_eq!(report.books, 1);
    assert_eq!(report.files_seen, 2);

    let book = with_db(&h.db, |c| library::search(c, "hobbit", 0, 5))
        .await
        .unwrap();
    let detail = with_db(&h.db, {
        let id = book[0].id;
        move |c| library::detail(c, id)
    })
    .await
    .unwrap()
    .unwrap();
    assert_eq!(detail.title, "The Hobbit");
    assert_eq!(detail.chapters.len(), 2);
    // The second file's chapter starts where the first file ends.
    assert!(
        (detail.chapters[1].start_ms - 2000).abs() < 150,
        "{:?}",
        detail.chapters
    );
    assert!((detail.duration_ms.unwrap() - 5000).abs() < 200);

    let ids = h.jobs.enqueue(vec![book[0].id], opts(&h)).await.unwrap();
    let done = tokio::task::spawn_blocking({
        let sink = h.sink.clone();
        move || sink.wait_terminal(ids[0])
    })
    .await
    .unwrap();
    assert_eq!(done.status, JobStatus::Done, "error: {:?}", done.error);
    let output = h.out.join("Test Author").join("The Hobbit.m4b");
    assert!((ms_of(&output).await - 5000).abs() < 250);
}

#[tokio::test(flavor = "multi_thread")]
async fn cancel_removes_partial_output() {
    let h = harness();
    let dir = h.root.join("Long");
    std::fs::create_dir_all(&dir).unwrap();
    // Long enough that the cancel lands mid-encode.
    make_m4a(&dir.join("long.m4a"), 120, "Long Book", "Tester", &[]);
    scan_root(&h).await;
    let book_id = with_db(&h.db, |c| library::search(c, "long", 0, 5))
        .await
        .unwrap()[0]
        .id;

    let ids = h.jobs.enqueue(vec![book_id], opts(&h)).await.unwrap();
    // Wait until ffmpeg has actually started, then cancel.
    for _ in 0..200 {
        if h.sink
            .0
            .lock()
            .iter()
            .any(|e| e.id == ids[0] && e.status == JobStatus::Running && e.progress > 0.0)
        {
            break;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    h.jobs.cancel(ids[0]).unwrap();
    let done = tokio::task::spawn_blocking({
        let sink = h.sink.clone();
        move || sink.wait_terminal(ids[0])
    })
    .await
    .unwrap();
    assert_eq!(done.status, JobStatus::Cancelled);

    let output = h.out.join("Tester").join("Long Book.m4b");
    assert!(!output.exists());
    assert!(!PathBuf::from(format!("{}.part", output.display())).exists());
}

#[tokio::test(flavor = "multi_thread")]
async fn rescan_is_incremental_and_flags_missing_files() {
    let h = harness();
    let dir = h.root.join("Stable");
    std::fs::create_dir_all(&dir).unwrap();
    let file = dir.join("a.m4a");
    make_m4a(&file, 2, "Stable", "Writer", &[]);

    assert_eq!(scan_root(&h).await.files_probed, 1);
    // Nothing changed: no probes at all.
    assert_eq!(scan_root(&h).await.files_probed, 0);

    std::fs::remove_file(&file).unwrap();
    let report = scan_root(&h).await;
    assert_eq!(report.files_seen, 0);
    // The file is flagged, not deleted, so the book and its user data survive.
    let hits = with_db(&h.db, |c| library::search(c, "stable", 0, 5))
        .await
        .unwrap();
    assert_eq!(hits.len(), 1);
    assert!(hits[0].missing);
}

#[tokio::test(flavor = "multi_thread")]
async fn empty_query_lists_all_books_alphabetically() {
    let h = harness();
    for name in ["Zed", "alpha"] {
        let d = h.root.join(name);
        std::fs::create_dir_all(&d).unwrap();
        make_m4a(&d.join("b.m4a"), 1, name, "Someone", &[]);
    }
    scan_root(&h).await;
    let all = with_db(&h.db, |c| library::search(c, "   ", 0, 10))
        .await
        .unwrap();
    let titles: Vec<_> = all.iter().map(|b| b.title.as_str()).collect();
    assert_eq!(titles, ["alpha", "Zed"]);
}
