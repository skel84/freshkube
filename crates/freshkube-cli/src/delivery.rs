//! The delivery reads the commands share: the Changes list's and Waiting to
//! ship's, through the same functions the pages call, over the cluster the
//! person named.
//!
//! The setup file, when `--cluster` read one, gives the workspace the way the
//! app does (where Kargo, the builds and Argo CD are read, the destinations,
//! the supply chain's key). Other entries connect read-only through
//! delivery's pool. GitHub is read anonymously and sparingly: a command
//! never runs `gh`, never opens the keychain and never prints a token, so a
//! token source the setup names is not used here and the command says so.

use std::path::PathBuf;
use std::sync::Mutex;

use chrono::{DateTime, Utc};
use freshkube_core::failure::{Failure, FailureKind};
use freshkube_core::resources::kubeconfig_sources;
use freshkube_core::workspace::{Chosen, Entry, Part, Sources};
use freshkube_delivery::change::live::Mapping;
use freshkube_delivery::changes::Read as ChangesRead;
use freshkube_delivery::changes::live::{BUDGET, Where};
use freshkube_delivery::entry::pool::{Open, Pool};
use freshkube_delivery::entry::{self, EntryReader};
use freshkube_delivery::github::Handle;
use freshkube_delivery::waiting::Read as WaitingRead;

use crate::cluster::{Target, context_signs_in};
use crate::read::{Class, Problem, Read};

/// Where a delivery read goes: the settings the page would hand it.
pub(crate) fn place(target: &Target, github: bool) -> (Where, Vec<PathBuf>) {
    let setup = target.setup.as_ref();
    let (mapping, sources, kubeconfig) = match setup {
        Some(setup) => (
            Mapping::of(&setup.workspace, target.entry.as_deref()),
            Sources::of(&setup.workspace),
            setup.workspace.kubeconfig.clone(),
        ),
        None => (Mapping::default(), Sources::default(), None),
    };
    let chain = setup.and_then(|setup| setup.supply_chain.as_ref());
    let delivery = setup.and_then(|setup| setup.delivery.as_ref());
    let at = Where {
        cluster: target.source.id.clone(),
        label: target.context.clone(),
        argocd_namespace: "argocd".into(),
        mapping,
        sources,
        public_key_file: chain.and_then(|chain| chain.public_key_file.clone()),
        transparency_log: chain.is_some_and(|chain| chain.transparency_log),
        transparency_url: chain.and_then(|chain| chain.transparency_url.clone()),
        evidence_result: chain.and_then(|chain| chain.evidence.clone()),
        build_namespace: delivery.and_then(|delivery| delivery.build_namespace.clone()),
        stage_naming: delivery.and_then(|delivery| delivery.stage_naming.clone()),
        github: github
            .then(|| target.github.clone().or_else(|| Handle::anonymous(0).ok()))
            .flatten(),
    };
    let kubeconfigs = kubeconfig_sources(kubeconfig.as_deref());
    (at, kubeconfigs)
}

/// Words for a setup that names a GitHub token source the command won't use.
pub(crate) fn github_note(target: &Target) -> Option<String> {
    target
        .setup
        .as_ref()
        .and_then(|setup| setup.delivery.as_ref())
        .and_then(|delivery| delivery.github.as_ref())
        .map(|_| {
            "GitHub is read anonymously from the command line; the token the setup names is \
             never read here, so the rate limit is low."
                .to_owned()
        })
}

fn problem(target: &Target, what: &str, failure: Failure) -> Problem {
    Problem {
        class: Class::of(failure.kind),
        read: Box::new(Read {
            verb: "connect",
            kind: what.to_owned(),
            namespaced: false,
            namespace: None,
            context: target.context.clone(),
        }),
        message: failure.message,
    }
}

/// The open connection's reader. Where Kargo is read on this connection, one
/// that can't be made is the command's own problem, classified (a rejected
/// credential needs a sign-in); where Kargo is another entry's, the pool
/// takes the failure and says it as the page does.
async fn open(
    target: &Target,
    at: &Where,
    what: &str,
) -> Result<Result<EntryReader, Failure>, Problem> {
    match target.source.access.client().await {
        Ok(client) => Ok(Ok(entry::reader(client))),
        Err(failure) if matches!(at.sources.get(Part::Kargo), Chosen::Open) => {
            Err(problem(target, what, failure))
        }
        Err(failure) => Ok(Err(failure)),
    }
}

/// What a delivery read came back with, and the entries it did not connect
/// to because their sign-in needs a credential plugin and `--allow-login`
/// was not given.
pub(crate) struct Fetched<T> {
    pub read: T,
    pub sign_in: Vec<String>,
}

/// The connection to another entry, as the app makes it, except that an
/// entry whose context signs in with a credential plugin is not connected
/// unless `--allow-login` was given: the plugin may open a browser. Such an
/// entry is noted in `needed`, counted as a needed sign-in.
async fn connect(
    kubeconfigs: Vec<PathBuf>,
    allow_login: bool,
    needed: &Mutex<Vec<String>>,
    chosen: Entry,
) -> Result<EntryReader, Failure> {
    if !allow_login && context_signs_in(&kubeconfigs, &chosen.context) {
        needed.lock().expect("a list").push(chosen.id.clone());
        return Err(Failure::new(
            FailureKind::Unauthorized,
            format!(
                "entry {} signs in with a credential plugin, which may open a browser; run it \
                 again with --allow-login",
                chosen.id
            ),
        ));
    }
    entry::connect_entry(kubeconfigs, chosen).await
}

/// The Changes list's read, as the page makes it. Argo CD's namespace is
/// found as the page finds it, from its own labelled workloads.
pub(crate) async fn changes(
    target: &Target,
) -> Result<Fetched<(ChangesRead, DateTime<Utc>)>, Problem> {
    let (base, kubeconfigs) = place(target, true);
    let open = open(target, &base, "changes").await?;
    let needed = Mutex::new(Vec::new());
    let pool = Pool::new(
        &base.sources,
        Some(Open {
            entry: base.mapping.open.as_deref(),
            reader: open.as_ref().map_err(Clone::clone),
        }),
        |chosen| connect(kubeconfigs.clone(), target.allow_login, &needed, chosen),
        BUDGET,
    );
    let argocd_namespace = match pool.part(Part::ArgoCd).await {
        Ok((reader, _)) => freshkube_delivery::applications::read::argocd_namespace(&reader).await,
        Err(_) => base.argocd_namespace.clone(),
    };
    let at = Where {
        argocd_namespace,
        ..base.clone()
    };
    let now = Utc::now();
    let read = freshkube_delivery::changes::live::read_in(&pool, &at, now, BUDGET).await;
    let sign_in = needed.lock().expect("a list").clone();
    Ok(Fetched {
        read: (read, now),
        sign_in,
    })
}

/// Waiting to ship's read. `github` is whether pull requests are asked for.
pub(crate) async fn waiting(
    target: &Target,
    github: bool,
) -> Result<Fetched<WaitingRead>, Problem> {
    let (at, kubeconfigs) = place(target, github);
    let open = open(target, &at, "waiting-to-ship").await?;
    let needed = Mutex::new(Vec::new());
    let pool = Pool::new(
        &at.sources,
        Some(Open {
            entry: at.mapping.open.as_deref(),
            reader: open.as_ref().map_err(Clone::clone),
        }),
        |chosen| connect(kubeconfigs.clone(), target.allow_login, &needed, chosen),
        BUDGET,
    );
    let read = freshkube_delivery::waiting::live::read_in(&pool, &at, Utc::now(), BUDGET).await;
    let sign_in = needed.lock().expect("a list").clone();
    Ok(Fetched { read, sign_in })
}
