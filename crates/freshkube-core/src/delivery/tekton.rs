//! The CI/CD side: the PipelineRuns Pipelines as Code starts (labelled with
//! the commit SHA), their TaskRuns' results, Tekton Chains' signing
//! annotation and Conforma's verdict.

use std::collections::BTreeMap;

use serde_json::Value;

use super::digest::{Digest, is_full_sha, text};
use super::observation::{Meta, ObjectRef};
use super::read::{ListRequest, Reader, Resource, Scope, label_equals};
use super::source::{Source, Truncation, printable};
use super::versions::resolve_each;
use crate::resources::Failure;

pub const TEKTON_GROUP: &str = "tekton.dev";
const TEKTON_VERSIONS: &[&str] = &["v1", "v1beta1"];

/// The label PaC puts on every PipelineRun it starts.
pub const SHA_LABEL: &str = "pipelinesascode.tekton.dev/sha";
const REPOSITORY_LABEL: &str = "pipelinesascode.tekton.dev/repository";
const PIPELINE_RUN_LABEL: &str = "tekton.dev/pipelineRun";
/// Chains writes `"true"` here once it has signed a run.
pub const CHAINS_SIGNED: &str = "chains.tekton.dev/signed";

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PipelineRun {
    pub namespace: String,
    pub name: String,
    /// `metadata.uid` and `metadata.resourceVersion`.
    pub meta: Meta,
    /// From the PaC label: a claim until something else says the same.
    pub sha: Option<String>,
    pub repository: Option<String>,
    pub event: Option<String>,
    /// The pull request number PaC recorded, a claim.
    pub pull_request: Option<u64>,
    pub branch: Option<String>,
    /// The run's parameters, where it may name its revision: a second
    /// witness to the SHA.
    pub params: BTreeMap<String, String>,
    /// `Succeeded` condition status and reason.
    pub succeeded: Option<String>,
    pub reason: Option<String>,
    /// `status.completionTime`, as written: shown, never compared.
    pub completed: Option<String>,
    pub results: BTreeMap<String, String>,
    pub task_runs: Vec<String>,
    /// `chains.tekton.dev/signed`: `true`, `failed`, …
    pub chains_state: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TaskRun {
    pub namespace: String,
    pub name: String,
    /// `metadata.uid` and `metadata.resourceVersion`.
    pub meta: Meta,
    pub pipeline_run: Option<String>,
    pub results: BTreeMap<String, String>,
    pub chains_state: Option<String>,
}

impl PipelineRun {
    /// The object these facts were read from.
    pub fn object_ref(&self) -> ObjectRef {
        ObjectRef::new(
            TEKTON_GROUP,
            "PipelineRun",
            Some(&self.namespace),
            &self.name,
            &self.meta,
        )
    }
}

impl TaskRun {
    /// The object these facts were read from.
    pub fn object_ref(&self) -> ObjectRef {
        ObjectRef::new(
            TEKTON_GROUP,
            "TaskRun",
            Some(&self.namespace),
            &self.name,
            &self.meta,
        )
    }
}

/// An image a build reported, by repository and digest.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BuiltImage {
    pub url: String,
    pub digest: Digest,
}

/// Conforma's verdict from a `TEST_OUTPUT` result.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Conforma {
    pub outcome: String,
    pub failures: u64,
    pub warnings: u64,
}

fn results_of(value: &Value) -> BTreeMap<String, String> {
    ["/status/results", "/status/taskResults"]
        .into_iter()
        .filter_map(|pointer| value.pointer(pointer).and_then(Value::as_array))
        .flatten()
        .filter_map(|result| Some((text(result, "/name")?, text(result, "/value")?)))
        .collect()
}

fn chains_state(value: &Value) -> Option<String> {
    annotation(value, CHAINS_SIGNED)
}

fn label(value: &Value, key: &str) -> Option<String> {
    value
        .pointer("/metadata/labels")
        .and_then(|labels| labels.get(key))
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty())
        .map(str::to_owned)
}

fn annotation(value: &Value, key: &str) -> Option<String> {
    value
        .pointer("/metadata/annotations")
        .and_then(|annotations| annotations.get(key))
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty())
        .map(str::to_owned)
}

pub fn parse_pipeline_run(value: &Value) -> Option<PipelineRun> {
    let condition = value
        .pointer("/status/conditions")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .find(|condition| text(condition, "/type").as_deref() == Some("Succeeded"));
    let params = value
        .pointer("/spec/params")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|param| Some((text(param, "/name")?, text(param, "/value")?)))
        .collect();
    Some(PipelineRun {
        namespace: text(value, "/metadata/namespace")?,
        name: text(value, "/metadata/name")?,
        meta: Meta::parse(value),
        sha: label(value, SHA_LABEL),
        repository: label(value, REPOSITORY_LABEL),
        event: label(value, "pipelinesascode.tekton.dev/event-type"),
        pull_request: annotation(value, "pipelinesascode.tekton.dev/pull-request")
            .or_else(|| label(value, "pipelinesascode.tekton.dev/pull-request"))
            .and_then(|number| number.parse().ok()),
        branch: label(value, "pipelinesascode.tekton.dev/branch"),
        params,
        succeeded: condition.and_then(|condition| text(condition, "/status")),
        reason: condition.and_then(|condition| text(condition, "/reason")),
        completed: text(value, "/status/completionTime"),
        results: results_of(value),
        task_runs: value
            .pointer("/status/childReferences")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter(|child| text(child, "/kind").as_deref() == Some("TaskRun"))
            .filter_map(|child| text(child, "/name"))
            .collect(),
        chains_state: chains_state(value),
    })
}

pub fn parse_task_run(value: &Value) -> Option<TaskRun> {
    Some(TaskRun {
        namespace: text(value, "/metadata/namespace")?,
        name: text(value, "/metadata/name")?,
        meta: Meta::parse(value),
        pipeline_run: label(value, PIPELINE_RUN_LABEL),
        results: results_of(value),
        chains_state: chains_state(value),
    })
}

/// The images a set of results names: a `IMAGE_URL` result paired with the
/// `IMAGE_DIGEST` that has the same prefix, as Chains' type hinting does.
pub fn built_images(results: &BTreeMap<String, String>) -> Vec<BuiltImage> {
    results
        .iter()
        .filter_map(|(name, url)| {
            let prefix = name.strip_suffix("IMAGE_URL")?;
            let digest = results.get(&format!("{prefix}IMAGE_DIGEST"))?;
            Some(BuiltImage {
                url: url.trim().to_owned(),
                digest: Digest::parse(digest)?,
            })
        })
        .collect()
}

/// The parameter upstream names the revision a run builds: the git-clone
/// task's `revision`, which Pipelines as Code's templates fill.
const UPSTREAM_REVISION_PARAMS: &[&str] = &["revision"];
/// The results upstream names the commit a run built: Chains' type hint
/// `CHAINS-GIT_COMMIT` and the git-clone task's `commit`.
const UPSTREAM_COMMIT_RESULTS: &[&str] = &["CHAINS-GIT_COMMIT", "commit"];

/// Other parameter and result names a setup's pipelines record the commit
/// under, besides the upstream ones. Empty unless configured.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct CommitNames {
    pub params: Vec<String>,
    pub results: Vec<String>,
}

/// The first of the upstream names, then the configured ones, that `values`
/// holds.
fn named_commit(
    values: &BTreeMap<String, String>,
    upstream: &[&str],
    configured: &[String],
) -> Option<String> {
    upstream
        .iter()
        .copied()
        .chain(configured.iter().map(String::as_str))
        .find_map(|key| values.get(key))
        .map(|commit| commit.trim().to_owned())
}

/// A commit the results themselves name.
pub fn result_commit(results: &BTreeMap<String, String>, names: &CommitNames) -> Option<String> {
    named_commit(results, UPSTREAM_COMMIT_RESULTS, &names.results)
}

/// Where a pipeline writes its own record of what it built, for platforms
/// whose pipelines write one: the name of a result that holds JSON, and the
/// JSON pointers (RFC 6901, such as `/commit`) of the commit and the
/// image digest in it. Every part is configured; none has a default.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EvidenceResult {
    pub result: String,
    pub commit_pointer: String,
    pub digest_pointer: String,
}

/// The commit and digest a pipeline's own record names, to be checked
/// against the other sources.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EvidenceRecord {
    pub commit: Option<String>,
    pub image_digest: Option<Digest>,
}

pub fn evidence_record(
    results: &BTreeMap<String, String>,
    config: &EvidenceResult,
) -> Option<EvidenceRecord> {
    let json: Value = serde_json::from_str(results.get(&config.result)?).ok()?;
    Some(EvidenceRecord {
        commit: text(&json, &config.commit_pointer),
        image_digest: text(&json, &config.digest_pointer).and_then(|digest| Digest::parse(&digest)),
    })
}

/// `TEST_OUTPUT` is the result name upstream Conforma tasks write their
/// verdict under.
pub fn conforma(results: &BTreeMap<String, String>) -> Option<Conforma> {
    let output: Value = serde_json::from_str(results.get("TEST_OUTPUT")?).ok()?;
    Some(Conforma {
        outcome: text(&output, "/result")?,
        failures: output.get("failures").and_then(Value::as_u64).unwrap_or(0),
        warnings: output.get("warnings").and_then(Value::as_u64).unwrap_or(0),
    })
}

/// Where a build's own witness to its commit was read.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WitnessSource {
    /// A parameter of the run: declared.
    Param,
    /// A result of the run: reported.
    RunResult,
    /// A result of the task at this index of [`Build::tasks`]: reported.
    TaskResult(usize),
}

/// A commit a build names besides the PaC label, and where.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Witness {
    pub commit: String,
    pub source: WitnessSource,
}

impl Witness {
    /// Whether a result reported the commit, as against a parameter declaring it.
    pub fn reported(&self) -> bool {
        self.source != WitnessSource::Param
    }
}

/// A PipelineRun with the TaskRuns that belong to it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Build {
    pub run: PipelineRun,
    pub tasks: Vec<TaskRun>,
    /// Why the run's TaskRuns could not be read, when they could not. The
    /// run's own results still count; whatever only a task carries is
    /// unknown.
    pub tasks_unread: Option<String>,
}

impl Build {
    /// Every image this build reported, from the run's and its tasks' results.
    pub fn images(&self) -> Vec<BuiltImage> {
        let mut images = built_images(&self.run.results);
        for task in &self.tasks {
            for image in built_images(&task.results) {
                if !images.contains(&image) {
                    images.push(image);
                }
            }
        }
        images
    }

    /// Conforma's verdict from the first result that has one.
    pub fn conforma(&self) -> Option<Conforma> {
        conforma(&self.run.results)
            .or_else(|| self.tasks.iter().find_map(|task| conforma(&task.results)))
    }

    /// What Chains says: `true` if it signed the run or any task, else the
    /// first other state it wrote (`failed`, …); `None` when none carries the
    /// annotation, which is not the same as not signed.
    pub fn chains_state(&self) -> Option<String> {
        let states: Vec<&String> = self
            .run
            .chains_state
            .iter()
            .chain(
                self.tasks
                    .iter()
                    .filter_map(|task| task.chains_state.as_ref()),
            )
            .collect();
        states
            .iter()
            .find(|state| state.as_str() == "true")
            .or(states.first())
            .map(|state| (*state).clone())
    }

    /// The pipeline's own evidence record, from the configured result.
    pub fn evidence_record(&self, config: &EvidenceResult) -> Option<EvidenceRecord> {
        evidence_record(&self.run.results, config).or_else(|| {
            self.tasks
                .iter()
                .find_map(|task| evidence_record(&task.results, config))
        })
    }

    /// The commit the build names besides the PaC label: a commit result of
    /// the run or one of its tasks, else a revision parameter. `prefer` is the
    /// commit being asked about.
    pub fn witnessed_commit(&self, names: &CommitNames, prefer: &str) -> Option<String> {
        self.witness(names, prefer).map(|witness| witness.commit)
    }

    /// Every commit a result of the run, then of each of its tasks, names.
    pub fn reported_witnesses(&self, names: &CommitNames) -> Vec<Witness> {
        let named = |commit, source| Witness { commit, source };
        let mut found: Vec<Witness> = result_commit(&self.run.results, names)
            .map(|commit| named(commit, WitnessSource::RunResult))
            .into_iter()
            .collect();
        for (index, task) in self.tasks.iter().enumerate() {
            if let Some(commit) = result_commit(&task.results, names) {
                found.push(named(commit, WitnessSource::TaskResult(index)));
            }
        }
        found
    }

    /// [`Build::witnessed_commit`], with where the commit was read. What was
    /// reported (the run's results, then its TaskRuns') wins over what the
    /// run declared (its parameters); of several reported commits, one equal
    /// to `prefer` (a pipeline can clone more than one repository) wins,
    /// else the first.
    pub fn witness(&self, names: &CommitNames, prefer: &str) -> Option<Witness> {
        let reported = self.reported_witnesses(names);
        let chosen = reported
            .iter()
            .position(|witness| witness.commit.eq_ignore_ascii_case(prefer))
            .unwrap_or(0);
        reported.into_iter().nth(chosen).or_else(|| {
            named_commit(&self.run.params, UPSTREAM_REVISION_PARAMS, &names.params).map(|commit| {
                Witness {
                    commit,
                    source: WitnessSource::Param,
                }
            })
        })
    }
}

/// Builds are found by the full SHA only: PaC labels a run with the full
/// one, and a shortened one could name another commit.
fn check_sha(sha: &str) -> Result<(), Failure> {
    if is_full_sha(sha) {
        Ok(())
    } else {
        Err(Failure::new(
            crate::resources::FailureKind::Other,
            "a commit SHA is 40 or 64 hexadecimal digits",
        ))
    }
}

async fn list_scoped<R: Reader, T>(
    reader: &R,
    resource: &Resource,
    scope: Scope,
    parse: fn(&Value) -> Option<T>,
) -> Result<(Vec<T>, Option<Truncation>), Failure> {
    let listing = reader
        .list(&ListRequest {
            resource: resource.clone(),
            scope,
        })
        .await?;
    Ok(listing.parse(parse))
}

/// The builds PaC started for one commit, with their TaskRuns, in one
/// namespace. The SHA is only ever sent as a label selector value. Capped
/// when any of the listings was.
///
/// Tekton's API is discovered once for the whole read: PipelineRuns and
/// TaskRuns are resolved together, and every build's TaskRuns are listed
/// at that version. When only TaskRuns can't be resolved, each build says
/// so and keeps its own results.
///
/// A SHA-256 commit (64 digits) is longer than a label value may be, so no
/// build is found by it: that read says so, and the commit joins Kargo
/// Freight on the commit alone.
pub async fn read_builds<R: Reader>(reader: &R, namespace: &str, sha: &str) -> Source<Vec<Build>> {
    async fn run<R: Reader>(
        reader: &R,
        namespace: &str,
        sha: &str,
    ) -> Result<(Vec<Build>, Option<Truncation>), Failure> {
        check_sha(sha)?;
        if sha.len() > 63 {
            return Err(Failure::new(
                crate::resources::FailureKind::Other,
                "a SHA-256 commit is longer than a label value, so no build can be found by its SHA",
            ));
        }
        let selector = label_equals(SHA_LABEL, sha)?;
        let [pipeline_runs, task_runs] = resolve_each(
            reader,
            TEKTON_GROUP,
            ["pipelineruns", "taskruns"],
            TEKTON_VERSIONS,
            true,
        )
        .await?;
        let (runs, mut truncated) = list_scoped(
            reader,
            &pipeline_runs?,
            Scope::Labels {
                namespace: Some(namespace.to_owned()),
                selector,
            },
            parse_pipeline_run,
        )
        .await?;
        let mut builds = Vec::new();
        for run in runs {
            // One run's TaskRuns that can't be read leave that build without
            // them; the other builds are still read.
            let tasks = match &task_runs {
                Ok(task_runs) => match label_equals(PIPELINE_RUN_LABEL, &run.name) {
                    Ok(selector) => {
                        list_scoped(
                            reader,
                            task_runs,
                            Scope::Labels {
                                namespace: Some(namespace.to_owned()),
                                selector,
                            },
                            parse_task_run,
                        )
                        .await
                    }
                    Err(failure) => Err(failure),
                },
                Err(failure) => Err(failure.clone()),
            };
            let (tasks, tasks_unread) = match tasks {
                Ok((tasks, tasks_truncated)) => {
                    truncated = truncated.or(tasks_truncated);
                    (tasks, None)
                }
                Err(failure) => (Vec::new(), Some(printable(&failure))),
            };
            builds.push(Build {
                run,
                tasks,
                tasks_unread,
            });
        }
        Ok((builds, truncated))
    }
    Source::from_listing(run(reader, namespace, sha).await)
}
