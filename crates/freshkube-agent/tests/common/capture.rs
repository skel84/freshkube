//! Captures what `tracing` prints in this test binary, which installs the one
//! global subscriber.
#![allow(dead_code)]

use std::io::Write;
use std::sync::{Mutex, OnceLock};

static LOG: Mutex<Vec<u8>> = Mutex::new(Vec::new());
static INSTALLED: OnceLock<()> = OnceLock::new();

struct Sink;

impl Write for Sink {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        LOG.lock().unwrap().extend_from_slice(bytes);
        Ok(bytes.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

pub fn install() {
    INSTALLED.get_or_init(|| {
        tracing_subscriber::fmt()
            .with_writer(|| Sink)
            .with_ansi(false)
            .with_max_level(tracing::Level::TRACE)
            .init();
    });
}

pub fn captured() -> String {
    String::from_utf8_lossy(&LOG.lock().unwrap()).into_owned()
}
