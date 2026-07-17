//! Incremental JSONL ingestion. Tracks a byte offset per file so a growing
//! file only has its new bytes parsed, handles truncation/rotation by
//! re-reading from the start, and never chokes on a partial final line.

use std::collections::HashMap;
use std::io::{BufRead, BufReader, Seek, SeekFrom};
use std::path::{Path, PathBuf};

#[derive(Debug, Default, Clone)]
struct FileCursor {
    /// Byte offset of the first unconsumed byte.
    offset: u64,
    /// File length at the last read, used to detect truncation.
    len: u64,
}

/// Per-file read positions, keyed by path.
#[derive(Debug, Default)]
pub struct JsonlTail {
    cursors: HashMap<PathBuf, FileCursor>,
}

pub struct ReadOutcome {
    /// Complete new lines since the last read (without trailing newline).
    pub lines: Vec<String>,
    /// True when the file shrank or was replaced and state was reset.
    pub truncated: bool,
}

impl JsonlTail {
    pub fn new() -> Self {
        Self::default()
    }

    /// Whether a file has grown (or shrunk) since the last read, judged by
    /// length alone — cheap enough to call for every candidate file.
    pub fn needs_read(&self, path: &Path, current_len: u64) -> bool {
        match self.cursors.get(path) {
            Some(c) => current_len != c.len,
            None => true,
        }
    }

    /// Read all complete new lines from `path`. A trailing line without a
    /// newline is left unconsumed and will be re-read once complete, so a
    /// writer caught mid-line never produces a corrupt record.
    pub fn read_new_lines(&mut self, path: &Path) -> std::io::Result<ReadOutcome> {
        let file = std::fs::File::open(path)?;
        let len = file.metadata()?.len();
        let cursor = self.cursors.entry(path.to_path_buf()).or_default();

        let mut truncated = false;
        if len < cursor.offset {
            // File shrank: rotation or replacement. Start over.
            cursor.offset = 0;
            truncated = true;
        }

        let mut reader = BufReader::new(file);
        reader.seek(SeekFrom::Start(cursor.offset))?;

        let mut lines = Vec::new();
        let mut consumed = cursor.offset;
        let mut buf = Vec::new();
        loop {
            buf.clear();
            let n = reader.read_until(b'\n', &mut buf)?;
            if n == 0 {
                break;
            }
            if buf.last() != Some(&b'\n') {
                // Partial final line: leave it for the next scan.
                break;
            }
            consumed += n as u64;
            let text = String::from_utf8_lossy(&buf);
            let trimmed = text.trim_end_matches(['\n', '\r']);
            if !trimmed.is_empty() {
                lines.push(trimmed.to_string());
            }
        }
        cursor.offset = consumed;
        cursor.len = len;
        Ok(ReadOutcome { lines, truncated })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    #[test]
    fn reads_incrementally_and_skips_partial_line() {
        let dir = std::env::temp_dir().join(format!("lmtop-jsonl-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("a.jsonl");

        std::fs::write(&path, "{\"a\":1}\n{\"a\":2}\n{\"par").unwrap();
        let mut tail = JsonlTail::new();
        let out = tail.read_new_lines(&path).unwrap();
        assert_eq!(out.lines, vec!["{\"a\":1}", "{\"a\":2}"]);
        assert!(!out.truncated);

        // Complete the partial line and append another.
        let mut f = std::fs::OpenOptions::new()
            .append(true)
            .open(&path)
            .unwrap();
        write!(f, "tial\":3}}\n{{\"a\":4}}\n").unwrap();
        drop(f);
        let out = tail.read_new_lines(&path).unwrap();
        assert_eq!(out.lines, vec!["{\"partial\":3}", "{\"a\":4}"]);

        // Nothing new -> no lines, and needs_read is false.
        let len = std::fs::metadata(&path).unwrap().len();
        assert!(!tail.needs_read(&path, len));
        let out = tail.read_new_lines(&path).unwrap();
        assert!(out.lines.is_empty());

        // Truncation resets and re-reads from the start.
        std::fs::write(&path, "{\"fresh\":1}\n").unwrap();
        let out = tail.read_new_lines(&path).unwrap();
        assert!(out.truncated);
        assert_eq!(out.lines, vec!["{\"fresh\":1}"]);

        std::fs::remove_dir_all(&dir).ok();
    }
}
