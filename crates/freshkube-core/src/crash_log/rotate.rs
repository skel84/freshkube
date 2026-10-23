//! Keeps the log files of the sessions before this one instead of wiping
//! them: `freshkube.log` becomes `freshkube.previous.log`, that one
//! `freshkube.previous-2.log`, and the oldest is dropped.

use std::fs::{self, File};
use std::io::{self, Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};

/// How many earlier sessions' logs are kept beside the current one.
pub const KEPT: usize = 2;

/// The most a kept log holds. A longer one keeps its last bytes, which are the
/// ones that end with the crash.
pub const MAX_KEPT_BYTES: u64 = 8 * 1024 * 1024;

/// The file `n` sessions back: 0 is `path` itself, 1 is `<stem>.previous.<ext>`
/// and 2 is `<stem>.previous-2.<ext>`.
pub fn generation(path: &Path, n: usize) -> PathBuf {
    if n == 0 {
        return path.to_path_buf();
    }
    let stem = path
        .file_stem()
        .map_or_else(|| "freshkube".into(), |s| s.to_string_lossy().into_owned());
    let tag = if n == 1 {
        "previous".to_owned()
    } else {
        format!("previous-{n}")
    };
    let name = match path.extension() {
        Some(ext) => format!("{stem}.{tag}.{}", ext.to_string_lossy()),
        None => format!("{stem}.{tag}"),
    };
    path.with_file_name(name)
}

/// Moves each log one generation back, dropping the oldest, and trims the one
/// that becomes `previous` to [`MAX_KEPT_BYTES`]. An empty or missing current
/// log moves nothing: a launch that wrote nothing must not push a real log out.
pub fn rotate(path: &Path) -> io::Result<()> {
    if fs::metadata(path).map_or(true, |meta| meta.len() == 0) {
        return Ok(());
    }
    for n in (1..=KEPT).rev() {
        let from = generation(path, n - 1);
        let to = generation(path, n);
        if n == KEPT {
            match fs::remove_file(&to) {
                Err(e) if e.kind() != io::ErrorKind::NotFound => return Err(e),
                _ => {}
            }
        }
        match fs::rename(&from, &to) {
            Err(e) if e.kind() != io::ErrorKind::NotFound => return Err(e),
            _ => {}
        }
    }
    trim(&generation(path, 1), MAX_KEPT_BYTES)
}

/// Cuts a file to its last `max` bytes.
fn trim(path: &Path, max: u64) -> io::Result<()> {
    let len = fs::metadata(path)?.len();
    if len <= max {
        return Ok(());
    }
    let mut file = File::open(path)?;
    file.seek(SeekFrom::Start(len - max))?;
    let mut tail = Vec::with_capacity(max as usize);
    file.read_to_end(&mut tail)?;
    // Start at a line, not in the middle of one.
    let from = tail.iter().position(|b| *b == b'\n').map_or(0, |i| i + 1);
    File::create(path)?.write_all(&tail[from..])
}

/// Rotates, then creates the current log.
pub fn open(path: &Path) -> io::Result<File> {
    if let Some(parent) = path.parent().filter(|p| !p.as_os_str().is_empty()) {
        fs::create_dir_all(parent)?;
    }
    // A log that can't be moved still has to be written: the session's own
    // log matters more than the last one's.
    if let Err(error) = rotate(path) {
        eprintln!("freshkube: could not keep the previous log: {error}");
    }
    File::create(path)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write(path: &Path, text: &str) {
        fs::write(path, text).unwrap();
    }

    fn read(path: &Path) -> String {
        fs::read_to_string(path).unwrap()
    }

    #[test]
    fn generations_are_named_beside_the_log() {
        let log = Path::new("/t/freshkube.log");
        assert_eq!(generation(log, 0), log);
        assert_eq!(generation(log, 1), Path::new("/t/freshkube.previous.log"));
        assert_eq!(generation(log, 2), Path::new("/t/freshkube.previous-2.log"));
        assert_eq!(
            generation(Path::new("/t/custom"), 1),
            Path::new("/t/custom.previous")
        );
    }

    #[test]
    fn a_second_launch_keeps_the_first_runs_log() {
        let dir = tempfile::tempdir().unwrap();
        let log = dir.path().join("freshkube.log");
        write(&log, "first run\n");
        drop(open(&log).unwrap());
        assert_eq!(read(&generation(&log, 1)), "first run\n");
        assert_eq!(read(&log), "");
    }

    #[test]
    fn only_so_many_sessions_are_kept() {
        let dir = tempfile::tempdir().unwrap();
        let log = dir.path().join("freshkube.log");
        for run in 1..=4 {
            let mut file = open(&log).unwrap();
            writeln!(file, "run {run}").unwrap();
        }
        assert_eq!(read(&log), "run 4\n");
        assert_eq!(read(&generation(&log, 1)), "run 3\n");
        assert_eq!(read(&generation(&log, 2)), "run 2\n");
        assert!(!generation(&log, 3).exists());
        let files = fs::read_dir(dir.path()).unwrap().count();
        assert_eq!(files, KEPT + 1);
    }

    #[test]
    fn an_empty_launch_does_not_push_a_real_log_out() {
        let dir = tempfile::tempdir().unwrap();
        let log = dir.path().join("freshkube.log");
        write(&log, "real\n");
        drop(open(&log).unwrap());
        drop(open(&log).unwrap());
        assert_eq!(read(&generation(&log, 1)), "real\n");
        assert!(!generation(&log, 2).exists());
    }

    #[test]
    fn a_long_log_keeps_its_end_from_a_whole_line() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("long.log");
        write(&path, "aaaa\nbbbb\ncccc\n");
        trim(&path, 8).unwrap();
        assert_eq!(read(&path), "cccc\n");
        write(&path, "aaaa\n");
        trim(&path, 8).unwrap();
        assert_eq!(read(&path), "aaaa\n");
    }

    #[test]
    fn a_missing_log_is_made_with_its_folder() {
        let dir = tempfile::tempdir().unwrap();
        let log = dir.path().join("sub/freshkube.log");
        drop(open(&log).unwrap());
        assert!(log.exists());
    }
}
