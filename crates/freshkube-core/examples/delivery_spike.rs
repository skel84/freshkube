//! Read-only delivery join (#158): from one commit, or one pull request, to
//! the pods running its digest, with every link's confidence printed.
//!
//! Every cluster read goes through `delivery::ReadOnlyClient`, which only
//! GETs; pull requests come from `gh api --method GET`. Nothing here writes.
//! Contexts are named explicitly as `alias=context` and only the alias is
//! printed; kubeconfig contents (servers, users, tokens) are never output.
//!
//! ```sh
//! FRESHKUBE_KUBECONFIG=<file> cargo run -p freshkube-core --example delivery_spike -- \
//!     --kargo kargo=<ctx> --argocd argocd=<ctx> --tekton tekton=<ctx> --env env=<ctx> \
//!     --kargo-project <ns> --argocd-namespace <ns> --build-namespace <ns> \
//!     [--known-as alias=server] \
//!     [--evidence-result NAME --evidence-commit POINTER --evidence-digest POINTER] \
//!     [--commit-param NAME]... [--commit-result NAME]... \
//!     [--stage-annotation KEY --stage-name 'TEMPLATE with {project} and {stage}'] \
//!     [--repo owner/name] (--sha <sha> | --pr <number>) [--discover] [--probe]
//! ```
//!
//! The optional settings describe conventions that differ between setups,
//! so none has a default:
//! - `--evidence-*`: a build result holding JSON in which the pipeline
//!   records what it built, and the JSON pointers of its commit and image
//!   digest (for example `/source/commit`).
//! - `--commit-param`, `--commit-result`: further parameter and result names
//!   a build records its commit under, besides upstream's `revision`
//!   parameter and `CHAINS-GIT_COMMIT` and `commit` results.
//! - `--stage-*`: for Applications without Kargo's authorized-stage
//!   annotation, the kustomize annotation that names the Kargo project and a
//!   template for the Application's name.

use std::collections::BTreeMap;

use freshkube_core::delivery::{
    Clusters, CommitNames, EvidenceResult, GhCli, Plan, ReadOnlyClient, Reader, Scope, StageNaming,
    collect, commit_of_pull_request, join, render,
};
use freshkube_core::delivery::{ListRequest, Resource, Source};
use freshkube_core::resources::{connect, kubeconfig_sources};

/// `alias=context`.
struct Named {
    alias: String,
    context: String,
}

#[derive(Default)]
struct Args {
    kargo: Option<Named>,
    argocd: Option<Named>,
    tekton: Option<Named>,
    env: Option<Named>,
    kargo_project: Option<String>,
    argocd_namespace: Option<String>,
    build_namespace: Option<String>,
    repo: Option<String>,
    sha: Option<String>,
    pr: Option<u64>,
    /// `alias=server`: another address an Application's destination may use
    /// for a context; a context may be reachable at more than one address.
    known_as: Vec<(String, String)>,
    evidence_result: Option<String>,
    evidence_commit: Option<String>,
    evidence_digest: Option<String>,
    commit_names: CommitNames,
    stage_annotation: Option<String>,
    stage_name: Option<String>,
    discover: bool,
    probe: bool,
}

fn named(value: &str) -> Named {
    match value.split_once('=') {
        Some((alias, context)) => Named {
            alias: alias.into(),
            context: context.into(),
        },
        None => fail("a context is written alias=context"),
    }
}

fn fail(message: &str) -> ! {
    eprintln!("delivery_spike: {message}");
    std::process::exit(2);
}

fn parse() -> Args {
    let mut args = Args::default();
    let mut it = std::env::args().skip(1);
    while let Some(flag) = it.next() {
        let mut value = || {
            it.next()
                .unwrap_or_else(|| fail("a flag is missing its value"))
        };
        match flag.as_str() {
            "--kargo" => args.kargo = Some(named(&value())),
            "--argocd" => args.argocd = Some(named(&value())),
            "--tekton" => args.tekton = Some(named(&value())),
            "--evidence-result" => args.evidence_result = Some(value()),
            "--evidence-commit" => args.evidence_commit = Some(value()),
            "--evidence-digest" => args.evidence_digest = Some(value()),
            "--commit-param" => args.commit_names.params.push(value()),
            "--commit-result" => args.commit_names.results.push(value()),
            "--stage-annotation" => args.stage_annotation = Some(value()),
            "--stage-name" => args.stage_name = Some(value()),
            "--env" => args.env = Some(named(&value())),
            "--known-as" => {
                let Named { alias, context } = named(&value());
                args.known_as.push((alias, context));
            }
            "--kargo-project" => args.kargo_project = Some(value()),
            "--argocd-namespace" => args.argocd_namespace = Some(value()),
            "--build-namespace" => args.build_namespace = Some(value()),
            "--repo" => args.repo = Some(value()),
            "--sha" => args.sha = Some(value()),
            "--pr" => args.pr = value().parse().ok().or_else(|| fail("--pr takes a number")),
            "--discover" => args.discover = true,
            "--probe" => args.probe = true,
            other => fail(&format!("unknown flag {other}")),
        }
    }
    args
}

struct Cluster {
    alias: String,
    server: String,
    client: ReadOnlyClient,
}

/// Connects to exactly the named context. The server URL stays in memory for
/// destination matching and is never printed.
async fn open(sources: &[std::path::PathBuf], named: &Named) -> Cluster {
    let connection = match connect(sources.to_vec(), named.context.clone()).await {
        Ok(connection) => connection,
        Err(failure) => fail(&format!("{}: {}", named.alias, failure.message)),
    };
    let server = server_of(sources, &named.context).unwrap_or_default();
    Cluster {
        alias: named.alias.clone(),
        server,
        client: ReadOnlyClient::new(connection.client),
    }
}

fn server_of(sources: &[std::path::PathBuf], context: &str) -> Option<String> {
    let mut merged: Option<kube::config::Kubeconfig> = None;
    for path in sources {
        if let Ok(config) = kube::config::Kubeconfig::read_from(path) {
            merged = Some(match merged {
                Some(first) => first.merge(config).ok()?,
                None => config,
            });
        }
    }
    let config = merged?;
    let cluster_name = config
        .contexts
        .iter()
        .find(|c| c.name == context)?
        .context
        .as_ref()?
        .cluster
        .clone();
    config
        .clusters
        .iter()
        .find(|c| c.name == cluster_name)?
        .cluster
        .as_ref()?
        .server
        .clone()
}

async fn discover(label: &str, reader: &ReadOnlyClient) {
    const GROUPS: [&str; 7] = [
        "kargo.akuity.io",
        "argoproj.io",
        "tekton.dev",
        "pipelinesascode.tekton.dev",
        "results.tekton.dev",
        "triggers.tekton.dev",
        "operator.tekton.dev",
    ];
    match reader.groups().await {
        Err(failure) => println!("  [{label}] discovery: {failure}"),
        Ok(served) => {
            for group in GROUPS {
                match served.iter().find(|s| s.name == group) {
                    Some(s) => println!("  [{label}] {group}: {}", s.versions.join(", ")),
                    None => println!("  [{label}] {group}: not served"),
                }
            }
        }
    }
}

/// One namespaced list of one kind, to see which kinds are readable.
async fn probe(
    label: &str,
    reader: &ReadOnlyClient,
    (group, version, plural): (&str, &str, &str),
    namespace: &str,
) {
    let request = ListRequest {
        resource: Resource::new(group, version, plural, true),
        scope: Scope::Namespace(namespace.to_owned()),
    };
    let outcome = match reader.list(&request).await {
        Ok(listing) => format!(
            "readable, {} item(s){}",
            listing.items.len(),
            if listing.truncated.is_some() {
                " (truncated)"
            } else {
                ""
            }
        ),
        Err(failure) => format!("{failure}"),
    };
    println!("  [{label}] list {plural}.{group}/{version} in a namespace: {outcome}");
}

#[tokio::main]
async fn main() {
    let args = parse();
    let sources = kubeconfig_sources(None);
    let need = |value: &Option<Named>, what: &str| -> Named {
        value
            .as_ref()
            .map(|n| Named {
                alias: n.alias.clone(),
                context: n.context.clone(),
            })
            .unwrap_or_else(|| fail(&format!("--{what} is required")))
    };
    let kargo = open(&sources, &need(&args.kargo, "kargo")).await;
    let argocd = open(&sources, &need(&args.argocd, "argocd")).await;
    let tekton = open(&sources, &need(&args.tekton, "tekton")).await;
    let env = open(&sources, &need(&args.env, "env")).await;
    let project = args
        .kargo_project
        .clone()
        .unwrap_or_else(|| fail("--kargo-project is required"));
    let argocd_ns = args
        .argocd_namespace
        .clone()
        .unwrap_or_else(|| fail("--argocd-namespace is required"));
    let build_ns = args
        .build_namespace
        .clone()
        .unwrap_or_else(|| fail("--build-namespace is required"));

    if args.discover {
        println!("served API versions");
        for cluster in [&kargo, &argocd, &tekton, &env] {
            discover(&cluster.alias, &cluster.client).await;
        }
    }
    if args.probe {
        println!("which kinds are readable (one namespace each)");
        probe(
            &kargo.alias,
            &kargo.client,
            ("kargo.akuity.io", "v1alpha1", "freights"),
            &project,
        )
        .await;
        probe(
            &kargo.alias,
            &kargo.client,
            ("kargo.akuity.io", "v1alpha1", "stages"),
            &project,
        )
        .await;
        probe(
            &kargo.alias,
            &kargo.client,
            ("kargo.akuity.io", "v1alpha1", "promotions"),
            &project,
        )
        .await;
        probe(
            &argocd.alias,
            &argocd.client,
            ("argoproj.io", "v1alpha1", "applications"),
            &argocd_ns,
        )
        .await;
        probe(
            &tekton.alias,
            &tekton.client,
            ("tekton.dev", "v1", "pipelineruns"),
            &build_ns,
        )
        .await;
        probe(
            &tekton.alias,
            &tekton.client,
            ("pipelinesascode.tekton.dev", "v1alpha1", "repositories"),
            &build_ns,
        )
        .await;
    }

    let github = GhCli;
    let sha = match (&args.sha, args.pr, &args.repo) {
        (Some(sha), _, _) => sha.clone(),
        (None, Some(number), Some(repo)) => {
            match commit_of_pull_request(&github, repo, number).await {
                Source::Read((sha, merged)) => {
                    println!(
                        "pull request #{number}: {} commit {}",
                        if merged { "merge" } else { "head (not merged)" },
                        sha.chars().take(12).collect::<String>()
                    );
                    sha
                }
                other => fail(&format!("pull request: {:?}", other.why_not_read())),
            }
        }
        _ => {
            if args.discover || args.probe {
                return;
            }
            fail("give --sha, or --pr with --repo")
        }
    };

    // Destinations are matched to the contexts named here, and nothing else.
    // Only aliases leave this function.
    let mut contexts: Vec<(String, String)> = [&kargo, &argocd, &tekton, &env]
        .iter()
        .map(|c| (c.alias.clone(), c.server.clone()))
        .collect::<BTreeMap<_, _>>()
        .into_iter()
        .collect();
    contexts.extend(args.known_as.iter().cloned());
    let plan = Plan {
        sha,
        kargo_project: project,
        argocd_namespace: argocd_ns,
        build_namespace: build_ns,
        github_repo: args.repo.clone(),
        environment: Some(env.alias.clone()),
        evidence_result: match (
            &args.evidence_result,
            &args.evidence_commit,
            &args.evidence_digest,
        ) {
            (Some(result), Some(commit), Some(digest)) => Some(EvidenceResult {
                result: result.clone(),
                commit_pointer: commit.clone(),
                digest_pointer: digest.clone(),
            }),
            (None, None, None) => None,
            _ => fail("--evidence-result, --evidence-commit and --evidence-digest go together"),
        },
        commit_names: args.commit_names.clone(),
        stage_naming: match (&args.stage_annotation, &args.stage_name) {
            (Some(annotation_key), Some(name_template)) => Some(StageNaming {
                annotation_key: annotation_key.clone(),
                name_template: name_template.clone(),
            }),
            (None, None) => None,
            _ => fail("--stage-annotation and --stage-name go together"),
        },
        contexts,
    };
    let clusters = Clusters {
        kargo: &kargo.client,
        argocd: &argocd.client,
        tekton: &tekton.client,
        environment: &env.client,
        github: &github,
    };
    let evidence = collect(&clusters, &plan).await;
    print!("{}", render(&join(&evidence)));
}
