//! The session's token must not reach the log, even at debug level.

mod support;

use std::io;
use std::sync::{Arc, Mutex};

use freshkube_agent::QUIET_LOG_TARGETS;
use freshkube_agent::acp::Endpoint;
use freshkube_agent::investigation::Investigation;
use support::{start_in_process, until_ended};
use tracing_subscriber::EnvFilter;
use tracing_subscriber::fmt::MakeWriter;

#[derive(Clone, Default)]
struct Captured(Arc<Mutex<Vec<u8>>>);

impl io::Write for Captured {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.0.lock().unwrap().extend_from_slice(buf);
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

impl<'a> MakeWriter<'a> for Captured {
    type Writer = Captured;

    fn make_writer(&'a self) -> Self::Writer {
        self.clone()
    }
}

const TOKEN: &str = "log-test-token-9f8e7d6c5b4a";

/// Runs one prompt through the in-process session with debug logging, and
/// returns what was logged.
async fn logged(filter: EnvFilter) -> String {
    let captured = Captured::default();
    let subscriber = tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_writer(captured.clone())
        .finish();
    let _guard = tracing::subscriber::set_default(subscriber);
    let (session, mut rx) = start_in_process(Endpoint::new("http://127.0.0.1:1/mcp", TOKEN));
    session.prompt("model").unwrap();
    let mut inv = Investigation::default();
    until_ended(&mut rx, &mut inv).await;
    drop(session);
    let bytes = captured.0.lock().unwrap().clone();
    String::from_utf8_lossy(&bytes).into_owned()
}

#[tokio::test]
async fn the_token_does_not_reach_the_debug_log() {
    // Without the directives the libraries log the message that carries it,
    // so this test can tell the filter is what keeps it out.
    let open = logged(EnvFilter::new("debug")).await;
    assert!(
        open.contains(TOKEN),
        "the libraries no longer log the token at debug; this test proves nothing"
    );

    let filter = QUIET_LOG_TARGETS
        .iter()
        .fold(EnvFilter::new("debug"), |f, d| {
            f.add_directive(d.parse().unwrap())
        });
    let quiet = logged(filter).await;
    assert!(
        !quiet.contains(TOKEN),
        "the token reached the log:\n{quiet}"
    );
}
