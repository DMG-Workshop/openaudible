//! Read-side queries: FTS search and single-book detail.

use rusqlite::{params, Connection, OptionalExtension};
use serde::Serialize;

#[derive(Debug, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct BookSummary {
    pub id: i64,
    pub title: String,
    pub author: String,
    pub narrator: String,
    pub duration_ms: Option<i64>,
    pub cover_path: Option<String>,
    pub missing: bool,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ChapterRow {
    pub idx: i64,
    pub start_ms: i64,
    pub end_ms: i64,
    pub title: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BookDetail {
    pub id: i64,
    pub title: String,
    pub author: String,
    pub narrator: String,
    pub series: Option<String>,
    pub series_index: Option<f64>,
    pub description: Option<String>,
    pub codec: Option<String>,
    pub bitrate_bps: Option<i64>,
    pub duration_ms: Option<i64>,
    pub chapters: Vec<ChapterRow>,
}

/// Turns free text into a safe FTS5 query: every token must match a word prefix.
pub fn fts_query(input: &str) -> String {
    input
        .split_whitespace()
        .map(|t| format!("\"{}\"*", t.replace('"', "\"\"")))
        .collect::<Vec<_>>()
        .join(" ")
}

pub fn search(
    c: &Connection,
    query: &str,
    offset: i64,
    limit: i64,
) -> rusqlite::Result<Vec<BookSummary>> {
    let fts = fts_query(query);
    let map = |r: &rusqlite::Row<'_>| {
        Ok(BookSummary {
            id: r.get(0)?,
            title: r.get(1)?,
            author: r.get(2)?,
            narrator: r.get(3)?,
            duration_ms: r.get(4)?,
            cover_path: r.get(5)?,
            missing: r.get::<_, i64>(6)? != 0,
        })
    };
    if fts.is_empty() {
        let mut st = c.prepare(
            "SELECT id, title, author, narrator, duration_ms, cover_path, missing
             FROM books ORDER BY title COLLATE NOCASE LIMIT ?1 OFFSET ?2",
        )?;
        let rows = st.query_map(params![limit, offset], map)?.collect();
        rows
    } else {
        let mut st = c.prepare(
            "SELECT b.id, b.title, b.author, b.narrator, b.duration_ms, b.cover_path, b.missing
             FROM books_fts f JOIN books b ON b.id = f.rowid
             WHERE books_fts MATCH ?3
             ORDER BY rank LIMIT ?1 OFFSET ?2",
        )?;
        let rows = st.query_map(params![limit, offset, fts], map)?.collect();
        rows
    }
}

pub fn detail(c: &Connection, id: i64) -> rusqlite::Result<Option<BookDetail>> {
    let book = c
        .query_row(
            "SELECT id, title, author, narrator, series, series_index, description,
                    codec, bitrate_bps, duration_ms
             FROM books WHERE id = ?1",
            [id],
            |r| {
                Ok(BookDetail {
                    id: r.get(0)?,
                    title: r.get(1)?,
                    author: r.get(2)?,
                    narrator: r.get(3)?,
                    series: r.get(4)?,
                    series_index: r.get(5)?,
                    description: r.get(6)?,
                    codec: r.get(7)?,
                    bitrate_bps: r.get(8)?,
                    duration_ms: r.get(9)?,
                    chapters: Vec::new(),
                })
            },
        )
        .optional()?;
    let Some(mut book) = book else {
        return Ok(None);
    };
    let mut st = c.prepare(
        "SELECT idx, start_ms, end_ms, title FROM chapters WHERE book_id = ?1 ORDER BY idx",
    )?;
    book.chapters = st
        .query_map([id], |r| {
            Ok(ChapterRow {
                idx: r.get(0)?,
                start_ms: r.get(1)?,
                end_ms: r.get(2)?,
                title: r.get(3)?,
            })
        })?
        .collect::<rusqlite::Result<_>>()?;
    Ok(Some(book))
}

#[cfg(test)]
mod tests {
    use super::fts_query;

    #[test]
    fn fts_query_quotes_and_prefixes_each_token() {
        assert_eq!(
            fts_query("  herbert \"dune\" "),
            "\"herbert\"* \"\"\"dune\"\"\"*"
        );
        assert_eq!(fts_query("   "), "");
    }
}
