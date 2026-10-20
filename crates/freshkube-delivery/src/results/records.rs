//! The manifest carried by a Results record goes through the live parsers.
use base64::{Engine, engine::general_purpose::STANDARD};
use serde_json::Value;

use super::transport::segment;
use super::{Error, Record};
use crate::tekton::{PipelineRun, TaskRun, parse_pipeline_run, parse_task_run};

const MOST_RECORD_BYTES: usize = 4 * 1024 * 1024;

pub(super) fn result_parts(result: &str) -> Result<(&str, &str), Error> {
    let parts: Vec<_> = result.split('/').collect();
    match parts.as_slice() {
        [namespace, "results", name] if *namespace != "-" && *name != "-" => {
            segment(namespace)?;
            segment(name)?;
            Ok((namespace, name))
        }
        _ => Err(Error::Unreadable("invalid Results result name")),
    }
}

pub(super) fn manifest(raw: &Value, kind: &str) -> Result<(String, String, Value), Error> {
    let name = raw
        .get("name")
        .and_then(Value::as_str)
        .ok_or(Error::Unreadable("the record has no name"))?;
    let (result, record) = name
        .rsplit_once("/records/")
        .ok_or(Error::Unreadable("invalid Results record name"))?;
    let (namespace, _) = result_parts(result)?;
    segment(record)?;
    if record == "-" {
        return Err(Error::Unreadable("the record has a wildcard name"));
    }
    let data = raw
        .get("data")
        .ok_or(Error::Unreadable("the record has no data"))?;
    let (group, versions): (&str, &[&str]) = if kind == "Log" {
        ("results.tekton.dev", &["v1alpha2", "v1alpha3"])
    } else {
        ("tekton.dev", &["v1", "v1beta1"])
    };
    let version = versions
        .iter()
        .copied()
        .into_iter()
        .find(|version| {
            data.get("type").and_then(Value::as_str)
                == Some(format!("{group}/{version}.{kind}").as_str())
        })
        .ok_or(Error::Unreadable("the record has an unsupported run type"))?;
    let encoded = data
        .get("value")
        .and_then(Value::as_str)
        .ok_or(Error::Unreadable("the record has no encoded manifest"))?;
    if encoded.len() > MOST_RECORD_BYTES.div_ceil(3) * 4 {
        return Err(Error::Unreadable("the record exceeds 4 MiB"));
    }
    let bytes = STANDARD
        .decode(encoded)
        .map_err(|_| Error::Unreadable("the record manifest is not base64"))?;
    if bytes.len() > MOST_RECORD_BYTES {
        return Err(Error::Unreadable("the record exceeds 4 MiB"));
    }
    let value: Value = serde_json::from_slice(&bytes)
        .map_err(|_| Error::Unreadable("the record manifest is not JSON"))?;
    if value.get("kind").and_then(Value::as_str) != Some(kind)
        || value.get("apiVersion").and_then(Value::as_str)
            != Some(format!("{group}/{version}").as_str())
        || value.pointer("/metadata/namespace").and_then(Value::as_str) != Some(namespace)
        || value
            .pointer("/metadata/uid")
            .and_then(Value::as_str)
            .is_none_or(str::is_empty)
    {
        return Err(Error::Unreadable(
            "the record's type, namespace or UID does not match its manifest",
        ));
    }
    Ok((name.to_owned(), result.to_owned(), value))
}

pub(super) fn pipeline_run(
    raw: &Value,
    namespace: &str,
    sha: &str,
) -> Result<Record<PipelineRun>, Error> {
    let (name, result, value) = manifest(raw, "PipelineRun")?;
    let run = parse_pipeline_run(&value).ok_or(Error::Unreadable(
        "the PipelineRun manifest has no name or namespace",
    ))?;
    if (namespace != "-" && namespace != run.namespace) || run.sha.as_deref() != Some(sha) {
        return Err(Error::Unreadable(
            "the PipelineRun does not match the requested namespace and commit",
        ));
    }
    Ok(Record { name, result, run })
}

pub(super) fn task_run(raw: &Value, wanted: &str) -> Result<Record<TaskRun>, Error> {
    let (name, result, value) = manifest(raw, "TaskRun")?;
    if result != wanted {
        return Err(Error::Unreadable(
            "the TaskRun does not belong to the requested result",
        ));
    }
    let run = parse_task_run(&value).ok_or(Error::Unreadable(
        "the TaskRun manifest has no name or namespace",
    ))?;
    Ok(Record { name, result, run })
}
