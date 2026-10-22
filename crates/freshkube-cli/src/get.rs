//! `freshkube get`: a kind's rows as the Resources page lists them.

use freshkube_core::resources::{StatusTone, find_kind};
use serde_json::{Map, Value, json};

use crate::Report;
use crate::args::GetArgs;
use crate::cluster::{Stop, Target};
use crate::exit::Findings;
use crate::read::{self, Class, Listing, Problem, Read, Row, list, view};

const NEVER: &str = "Secret values are never requested or printed.";

/// The usage errors that need no cluster: the options, the kind and the
/// Secret rule.
pub struct Wanted {
    pub kind: freshkube_core::resources::ResourceKind,
    pub name: Option<String>,
    pub wide: bool,
}

pub fn wanted(args: &GetArgs) -> Result<Wanted, Stop> {
    let (kind_text, name) = match args.kind.split_once('/') {
        Some((kind, name)) if args.name.is_none() => (kind, Some(name.to_owned())),
        Some(_) => {
            return Err(Stop::Usage(
                "give the name once: `kind/name` or `kind name`".into(),
            ));
        }
        None => (args.kind.as_str(), args.name.clone()),
    };
    let kind = find_kind(kind_text).ok_or_else(|| {
        Stop::Usage(format!(
            "unknown kind \"{kind_text}\". Use a kubectl key such as pods, nodes or deployments.apps."
        ))
    })?;
    let asks_for_a_value = args.show_values || args.reveal || args.decode || args.key.is_some();
    let secret = kind.is_secret();
    if asks_for_a_value {
        return Err(Stop::Usage(if secret {
            format!("{NEVER} Secrets are listed by their metadata only.")
        } else {
            "that option reads values, which this command never does".into()
        }));
    }
    if name.as_deref().is_some_and(|name| name.contains('/')) {
        return Err(Stop::Usage(if secret {
            format!("{NEVER} A Secret's keys are values.")
        } else {
            format!("\"{}\" is not an object name", name.unwrap_or_default())
        }));
    }
    let wide = match args.output.as_deref() {
        None | Some("table") => false,
        Some("wide") => true,
        Some(other) => {
            return Err(Stop::Usage(if secret {
                format!("{NEVER} -o {other} would print them; only table and wide are offered.")
            } else {
                format!("-o {other} is not offered; use --json for JSON, or -o wide.")
            }));
        }
    };
    Ok(Wanted { kind, name, wide })
}

pub async fn run(args: &GetArgs, target: &Target) -> Result<Report, Stop> {
    let Wanted { kind, name, wide } = wanted(args)?;
    let namespace = args.namespace.as_deref().filter(|_| kind.namespaced);
    let selector = name.as_ref().map(|name| format!("metadata.name={name}"));
    // A pod on a node that is not ready warns, as in the pods list; the
    // nodes are read for that alone, and a node list that fails changes nothing.
    let (result, nodes) = if kind.is_pod() {
        let nodes_kind = find_kind("nodes").expect("a built-in kind");
        let (result, nodes) = futures::join!(
            list(target, &kind, namespace, selector.as_deref(), wide),
            list(target, &nodes_kind, None, None, false)
        );
        (result, nodes.ok())
    } else {
        (
            list(target, &kind, namespace, selector.as_deref(), wide).await,
            None,
        )
    };
    let result = result.map(|mut listing| {
        if let Some(nodes) = &nodes {
            listing.note_nodes(&nodes.not_ready_nodes());
        }
        listing
    });
    let mut json = Map::new();
    json.insert(
        "kind".into(),
        json!({"key": kind.key(), "kind": kind.kind, "namespaced": kind.namespaced}),
    );
    json.insert("namespace".into(), json!(namespace));
    let mut findings = Findings::default();
    let text = match result {
        Ok(listing) => {
            json.insert("columns".into(), json!(listing.columns));
            json.insert("items".into(), Value::Array(items(&kind, &listing)));
            match name.as_ref().filter(|_| listing.rows.is_empty()) {
                Some(name) => {
                    let problem = Problem {
                        class: Class::NotFound,
                        read: Box::new(Read {
                            verb: "get",
                            kind: kind.key(),
                            namespaced: kind.namespaced,
                            namespace: namespace.map(str::to_owned),
                            context: target.context.clone(),
                        }),
                        message: format!("{} \"{name}\" not found", kind.key()),
                    };
                    findings = problem.findings();
                    json.insert("problems".into(), json!([problem.json()]));
                    format!("{}\n", problem.sentence())
                }
                None => {
                    json.insert("problems".into(), json!([]));
                    table(&kind, &listing, namespace)
                }
            }
        }
        Err(problem) => {
            findings = problem.findings();
            json.insert("columns".into(), json!([]));
            json.insert("items".into(), json!([]));
            json.insert("problems".into(), json!([problem.json()]));
            // An unreachable cluster shows `?`, not an empty list.
            let mark = if problem.class == read::Class::Unknown {
                "? "
            } else {
                ""
            };
            format!("{mark}{}\n", problem.sentence())
        }
    };
    Ok(Report {
        json,
        text,
        findings,
    })
}

fn items(kind: &freshkube_core::resources::ResourceKind, listing: &Listing) -> Vec<Value> {
    listing
        .rows
        .iter()
        .map(|row| {
            let cells: Map<String, Value> = listing
                .columns
                .iter()
                .zip(&row.cells)
                .map(|(column, cell)| (column.clone(), json!(cell)))
                .collect();
            json!({
                "namespace": row.namespace,
                "name": row.name,
                "cells": cells,
                "state": row.state(),
                "reason": row.tone.as_ref().map(|(_, reason)| reason),
                "view": view(kind, row),
            })
        })
        .collect()
}

fn glyph(row: &Row) -> &'static str {
    match row.tone.as_ref().map(|(tone, _)| tone) {
        Some(StatusTone::Success) => "+",
        Some(StatusTone::Warning) => "!",
        Some(StatusTone::Danger) => "x",
        Some(StatusTone::Neutral) => "?",
        None => " ",
    }
}

fn table(
    kind: &freshkube_core::resources::ResourceKind,
    listing: &Listing,
    namespace: Option<&str>,
) -> String {
    if listing.rows.is_empty() {
        return match namespace {
            Some(namespace) => format!("No {} in {namespace}.\n", kind.key()),
            None => format!("No {} found.\n", kind.key()),
        };
    }
    let with_namespace = kind.namespaced && namespace.is_none();
    let mut header = vec![String::new()];
    if with_namespace {
        header.push("NAMESPACE".into());
    }
    header.extend(listing.columns.iter().map(|c| c.to_uppercase()));
    let lines: Vec<Vec<String>> = listing
        .rows
        .iter()
        .map(|row| {
            let mut line = vec![glyph(row).to_owned()];
            if with_namespace {
                line.push(row.namespace.clone().unwrap_or_default());
            }
            line.extend(row.cells.iter().cloned());
            line
        })
        .collect();
    let widths: Vec<usize> = (0..header.len())
        .map(|ix| {
            std::iter::once(&header)
                .chain(&lines)
                .map(|line| line.get(ix).map_or(0, |cell| cell.chars().count()))
                .max()
                .unwrap_or(0)
        })
        .collect();
    let mut out = String::new();
    for line in std::iter::once(&header).chain(&lines) {
        let cells: Vec<String> = line
            .iter()
            .zip(&widths)
            .map(|(cell, width)| format!("{cell:<width$}"))
            .collect();
        out.push_str(cells.join("  ").trim_end());
        out.push('\n');
    }
    out
}
