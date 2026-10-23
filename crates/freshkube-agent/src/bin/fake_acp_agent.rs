//! The scripted ACP agent as a process, for the tests that need one: spawning,
//! the process group, the environment it is given, cancel. It is never part
//! of the app bundle. See `freshkube_agent::scripted` for what it does.
//!
//! `--idle` makes it a process that does nothing (the grandchild the agent
//! starts for `grandchild`); `--never-initialize` makes it never answer.

use freshkube_agent::scripted::{Script, serve};
use tokio_util::compat::{TokioAsyncReadCompatExt, TokioAsyncWriteCompatExt};

#[tokio::main]
async fn main() -> agent_client_protocol::Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.iter().any(|arg| arg == "--idle") {
        loop {
            tokio::time::sleep(std::time::Duration::from_secs(3600)).await;
        }
    }
    let result = serve(
        tokio::io::stdin().compat(),
        tokio::io::stdout().compat_write(),
        Script::from_args(args),
    )
    .await;
    // Stdin is read on a blocking thread that only ends when the client
    // closes it; a crash must not wait for that.
    std::process::exit(i32::from(result.is_err()))
}
