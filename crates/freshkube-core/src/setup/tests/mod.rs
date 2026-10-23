use super::*;
use crate::write::Protection;

/// The example's paths are Unix ones; a Windows path is absolute only with a
/// drive, so the tests read them under one.
fn portable(text: &str) -> String {
    if cfg!(windows) {
        text.replace("C:/home/acme", "/home/acme")
            .replace("/home/acme", "C:/home/acme")
    } else {
        text.to_owned()
    }
}

fn home(path: &str) -> std::path::PathBuf {
    portable(path).into()
}

fn ok(text: &str) -> Setup {
    parse(portable(text).as_bytes()).unwrap_or_else(|invalid| panic!("{invalid}"))
}

fn refused(text: &str) -> Invalid {
    parse(portable(text).as_bytes()).expect_err("the file should be refused")
}

fn at(line: usize, column: usize) -> Option<Place> {
    Some(Place { line, column })
}

/// A file with one cluster and `rest` after it.
fn with_cluster(rest: &str) -> String {
    format!(
        "version: 1\nclusters:\n  - id: cluster-a\n    role: core\n    context: cluster-a\n{rest}"
    )
}

fn rule(text: &str) -> String {
    match refused(text) {
        Invalid::Rule(fault) => fault.to_string(),
        other => panic!("{other:?}"),
    }
}

mod clusters;
mod coroot;
mod credentials;
mod delivery;
mod identity;
mod monitoring;
mod parsing;
mod persistence;
mod results;
mod round_trip;
mod supply_chain;
