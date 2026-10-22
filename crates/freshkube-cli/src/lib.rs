//! The read commands of the `freshkube` binary: `check` and `get`.
//!
//! They never start the window. They read through the same core functions
//! the pages call (`list_table` for the lists, the status tones of the
//! Resources page for the states) and change nothing: every request is a GET.
//! The cluster is always one the person named; the current kubeconfig or
//! talosconfig context is never used. [docs/features/cli.md] has the rules.

mod args;
mod check;
mod cluster;
mod exit;
mod get;
mod read;

pub use args::{CheckArgs, ClusterArgs, Command, FailOn, GetArgs};
pub use cluster::{Connector, Kubeconfig, Request, Stop, Target, resolve};
pub use exit::{FAILED, Findings, OK, REFUSED, SIGN_IN, UNKNOWN, USAGE};

use std::io::Write;
use std::path::PathBuf;

use serde_json::{Map, Value, json};

/// The contract's version. Fields are only added within a version.
pub const JSON_VERSION: u32 = 1;

/// What a command found, before the shared envelope is put around it.
pub(crate) struct Report {
    pub json: Map<String, Value>,
    pub text: String,
    pub findings: Findings,
}

/// What a run needs from outside: where the app keeps freshkube.yaml, and
/// how a context becomes a client.
pub struct Env<'a> {
    pub default_setup: Option<PathBuf>,
    pub connector: &'a dyn Connector,
}

/// Runs a command to its exit code. `stdout` gets the result (only JSON with
/// `--json`); `stderr` gets errors.
pub async fn execute(
    command: &Command,
    env: &Env<'_>,
    stdout: &mut dyn Write,
    stderr: &mut dyn Write,
) -> i32 {
    let args = command.cluster();
    // The options that need no cluster come first, so a usage error is
    // reported before anything is contacted.
    if let Command::Get(get_args) = command
        && let Err(stop) = get::wanted(get_args)
    {
        return stopped(stop, stderr);
    }
    let target = match resolve(args, env.default_setup.as_deref())
        .and_then(|request| env.connector.open(&request, args.allow_login))
    {
        Ok(target) => target,
        Err(stop) => return stopped(stop, stderr),
    };
    let report = match command {
        Command::Check(check_args) => check::run(check_args, &target).await,
        Command::Get(get_args) => get::run(get_args, &target).await,
    };
    let report = match report {
        Ok(report) => report,
        Err(stop) => return stopped(stop, stderr),
    };
    let code = report.findings.code();
    let written = if args.json {
        let mut object = Map::new();
        object.insert("version".into(), json!(JSON_VERSION));
        object.insert("command".into(), json!(command.name()));
        object.insert(
            "cluster".into(),
            json!({"entry": target.entry, "context": target.context}),
        );
        object.insert("exit".into(), json!(code));
        object.extend(report.json);
        writeln!(stdout, "{}", Value::Object(object))
    } else {
        write!(stdout, "{}", report.text)
    };
    if written.is_err() {
        // A closed pipe is the reader's choice; the code still says what was found.
        return code;
    }
    code
}

fn stopped(stop: Stop, stderr: &mut dyn Write) -> i32 {
    match stop {
        Stop::Usage(message) => {
            let _ = writeln!(stderr, "freshkube: {message}");
            USAGE
        }
        Stop::SignIn(message) => {
            let _ = writeln!(stderr, "freshkube: {message}");
            SIGN_IN
        }
    }
}

/// Runs a command for the binary: builds the runtime, reads the real
/// kubeconfig files and writes to the terminal.
pub fn run(command: &Command, default_setup: Option<PathBuf>) -> i32 {
    let runtime = match tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
    {
        Ok(runtime) => runtime,
        Err(error) => {
            eprintln!("freshkube: can't start the runtime: {error}");
            return FAILED;
        }
    };
    let env = Env {
        default_setup,
        connector: &Kubeconfig,
    };
    runtime.block_on(execute(
        command,
        &env,
        &mut std::io::stdout(),
        &mut std::io::stderr(),
    ))
}

#[cfg(test)]
mod tests;
