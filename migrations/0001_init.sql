BEGIN;

CREATE TABLE books (
  id           INTEGER PRIMARY KEY,
  group_key    TEXT NOT NULL UNIQUE,          -- folder path: one folder = one book
  title        TEXT NOT NULL DEFAULT '',
  author       TEXT NOT NULL DEFAULT '',
  narrator     TEXT NOT NULL DEFAULT '',
  series       TEXT,
  series_index REAL,
  year         INTEGER,
  description  TEXT,
  codec        TEXT,
  bitrate_bps  INTEGER,
  duration_ms  INTEGER,
  cover_path   TEXT,
  missing      INTEGER NOT NULL DEFAULT 0,
  added_at     INTEGER NOT NULL,
  updated_at   INTEGER NOT NULL
);
CREATE INDEX books_author ON books(author);
CREATE INDEX books_series ON books(series, series_index);

CREATE TABLE book_files (
  id          INTEGER PRIMARY KEY,
  book_id     INTEGER NOT NULL REFERENCES books(id) ON DELETE CASCADE,
  path        TEXT NOT NULL UNIQUE,
  position    INTEGER NOT NULL,              -- playback order
  size_bytes  INTEGER NOT NULL,
  mtime_ms    INTEGER NOT NULL,
  duration_ms INTEGER,
  missing     INTEGER NOT NULL DEFAULT 0
);
CREATE INDEX book_files_book ON book_files(book_id, position);

CREATE TABLE chapters (
  book_id  INTEGER NOT NULL REFERENCES books(id) ON DELETE CASCADE,
  idx      INTEGER NOT NULL,
  start_ms INTEGER NOT NULL,                  -- relative to the book, not the file
  end_ms   INTEGER NOT NULL,
  title    TEXT NOT NULL,
  PRIMARY KEY (book_id, idx)
);

CREATE TABLE jobs (
  id          INTEGER PRIMARY KEY,
  book_id     INTEGER REFERENCES books(id) ON DELETE SET NULL,
  kind        TEXT NOT NULL,                 -- 'convert'
  status      TEXT NOT NULL,                 -- queued|running|done|failed|cancelled
  progress    REAL NOT NULL DEFAULT 0,
  output_path TEXT,
  error       TEXT,
  created_at  INTEGER NOT NULL,
  finished_at INTEGER
);

CREATE VIRTUAL TABLE books_fts USING fts5(
  title, author, narrator, series,
  content='books', content_rowid='id',
  tokenize='unicode61 remove_diacritics 2'
);
CREATE TRIGGER books_ai AFTER INSERT ON books BEGIN
  INSERT INTO books_fts(rowid, title, author, narrator, series)
  VALUES (new.id, new.title, new.author, new.narrator, COALESCE(new.series, ''));
END;
CREATE TRIGGER books_ad AFTER DELETE ON books BEGIN
  INSERT INTO books_fts(books_fts, rowid, title, author, narrator, series)
  VALUES ('delete', old.id, old.title, old.author, old.narrator, COALESCE(old.series, ''));
END;
CREATE TRIGGER books_au AFTER UPDATE ON books BEGIN
  INSERT INTO books_fts(books_fts, rowid, title, author, narrator, series)
  VALUES ('delete', old.id, old.title, old.author, old.narrator, COALESCE(old.series, ''));
  INSERT INTO books_fts(rowid, title, author, narrator, series)
  VALUES (new.id, new.title, new.author, new.narrator, COALESCE(new.series, ''));
END;

COMMIT;
