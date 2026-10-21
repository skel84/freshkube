//! Argo CD's destinations, found in its cluster Secrets and matched to the
//! kubeconfig's contexts.
//!
//! The Secret read is the sensitive part. Kubernetes returns a whole Secret
//! on a list, `config` (credentials) and `last-applied-configuration` (which
//! can repeat the Secret) included, so [`parse_secret_list`] parses the
//! answer into structs that have only the three fields kept, no `flatten`
//! and no `Value`, and the caller drops the bytes when it returns. Nothing
//! here formats those bytes, and a parse failure says fixed words, since
//! `serde_json`'s own message can quote a value.

use std::path::PathBuf;

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use serde::Deserialize;

use crate::applications::ArgoFound;
use crate::applications::read::{argo_namespaces, read_argo_workloads};
use crate::entry::connect_entry;
use crate::read::{Reader, SECRETS_LIMIT, secrets_unreadable};
use crate::source::printable;
use freshkube_core::resources::{Failure, FailureKind, context_servers, load_kubeconfigs};
use freshkube_core::workspace::{
    Chosen, IN_CLUSTER_NAME, IN_CLUSTER_SERVER, Key, Part, Sources, Workspace, normalize_server,
};

/// The most cluster Secrets one namespace gives a suggestion; a longer list
/// is capped and suggests nothing.
pub const MAX_CLUSTER_SECRETS: usize = SECRETS_LIMIT as usize;
/// The longest server or name read from a Secret.
const MAX_FIELD: usize = 2048;

/// One field of a cluster Secret, once decoded.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Field {
    /// The Secret has no such field, or it is empty.
    Absent,
    /// It has one that isn't base64, text or short enough. Its bytes are
    /// never kept.
    Unreadable,
    Text(String),
}

impl Field {
    pub fn text(&self) -> Option<&str> {
        match self {
            Field::Text(text) => Some(text),
            Field::Absent | Field::Unreadable => None,
        }
    }
}

/// What is kept of one cluster Secret. There is no other field.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ClusterSecret {
    pub namespace: String,
    pub secret: String,
    pub server: Field,
    pub name: Field,
}

/// One page of a namespace's cluster Secrets.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ClusterSecrets {
    pub items: Vec<ClusterSecret>,
    /// The page had a `continue` token: there are more than were asked for.
    /// Then `items` is empty, since nothing is suggested from a partial
    /// list.
    pub capped: bool,
}

// None of these derives `Debug`, so no path can print what they hold.
#[derive(Deserialize)]
struct RawList {
    #[serde(default)]
    metadata: Option<ListMeta>,
    #[serde(default)]
    items: Option<Vec<RawItem>>,
}

#[derive(Deserialize)]
struct ListMeta {
    #[serde(rename = "continue", default)]
    next: Option<String>,
}

#[derive(Deserialize)]
struct RawItem {
    #[serde(default)]
    metadata: Option<ItemMeta>,
    #[serde(default)]
    data: Option<Data>,
}

#[derive(Deserialize)]
struct ItemMeta {
    #[serde(default)]
    name: Option<String>,
}

#[derive(Deserialize)]
struct Data {
    #[serde(default)]
    server: Option<String>,
    #[serde(default)]
    name: Option<String>,
}

/// A Secret list's body as [`ClusterSecrets`]. The caller owns `body` and
/// drops it after; the only things copied out are the names and the two
/// fields, decoded. A body that doesn't parse fails with
/// [`secrets_unreadable`]'s fixed words.
pub(crate) fn parse_secret_list(namespace: &str, body: &[u8]) -> Result<ClusterSecrets, Failure> {
    let list: RawList = serde_json::from_slice(body).map_err(|_| secrets_unreadable())?;
    if list
        .metadata
        .and_then(|metadata| metadata.next)
        .is_some_and(|next| !next.is_empty())
    {
        return Ok(ClusterSecrets {
            items: Vec::new(),
            capped: true,
        });
    }
    let items = list
        .items
        .into_iter()
        .flatten()
        .filter_map(|item| {
            let secret = item.metadata?.name.filter(|name| !name.is_empty())?;
            let (server, name) = match item.data {
                Some(data) => (field(data.server), field(data.name)),
                None => (Field::Absent, Field::Absent),
            };
            Some(ClusterSecret {
                namespace: namespace.to_owned(),
                secret,
                server,
                name,
            })
        })
        .collect();
    Ok(ClusterSecrets {
        items,
        capped: false,
    })
}

/// A field's base64 as text. A failure keeps nothing of the bytes.
fn field(encoded: Option<String>) -> Field {
    let Some(encoded) = encoded else {
        return Field::Absent;
    };
    let Some(text) = STANDARD
        .decode(encoded.trim())
        .ok()
        .and_then(|bytes| String::from_utf8(bytes).ok())
    else {
        return Field::Unreadable;
    };
    let text = text.trim();
    if text.is_empty() {
        Field::Absent
    } else if text.len() > MAX_FIELD || text.chars().any(char::is_control) {
        Field::Unreadable
    } else {
        Field::Text(text.to_owned())
    }
}

// ---- the findings ---------------------------------------------------------

/// A cluster Secret and the kubeconfig contexts whose server it names. The
/// servers themselves stay in the job that matched them.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Found {
    pub secret: ClusterSecret,
    pub contexts: Vec<String>,
}

/// What the read of one namespace's Secrets came to.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum NamespaceRead {
    Found(Vec<Found>),
    /// More Secrets than one page holds.
    Capped,
    /// The identity may not list Secrets there.
    Refused(String),
    /// Anything else, such as a deadline or the fixed parse text.
    Failed(String),
}

/// Where the find stood.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Read {
    /// Nothing was read: Settings knows no entry Argo CD is read from, and
    /// the words say why.
    NotRead(String),
    /// The entry Argo CD is read from couldn't be reached.
    Unreached(String),
    Done {
        namespaces: Vec<(String, NamespaceRead)>,
        /// Argo CD namespaces past the cap, not read.
        skipped: Vec<String>,
        /// Why the kubeconfig's servers weren't known, when they weren't.
        no_servers: Option<String>,
    },
}

/// What a find kept: the entry Argo CD was read from, and what it read.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DestinationFindings {
    pub entry: String,
    pub read: Read,
}

/// Reads Argo CD's cluster Secrets through `entry` and matches each
/// server to `kubeconfigs`' contexts. Within the entry's connect deadline
/// and each read's own.
pub async fn find_destinations(
    kubeconfigs: Vec<PathBuf>,
    entry: freshkube_core::workspace::Entry,
) -> DestinationFindings {
    let id = entry.id.clone();
    let servers = servers_of(kubeconfigs.clone());
    let reader = match connect_entry(kubeconfigs, entry).await {
        Ok(reader) => reader,
        Err(failure) => {
            return DestinationFindings {
                entry: id,
                read: Read::Unreached(printable(&failure)),
            };
        }
    };
    DestinationFindings {
        entry: id,
        read: read_destinations(&reader, servers.await).await,
    }
}

/// The merged kubeconfig's context servers, read off the async workers. No
/// credential is loaded: the files are parsed and only addresses kept.
async fn servers_of(kubeconfigs: Vec<PathBuf>) -> Result<Vec<(String, String)>, String> {
    tokio::task::spawn_blocking(move || match load_kubeconfigs(&kubeconfigs).1 {
        Some(config) => Ok(context_servers(&config)),
        None => Err("no kubeconfig file could be read".to_owned()),
    })
    .await
    .unwrap_or_else(|_| Err("the kubeconfig read stopped".to_owned()))
}

/// The cluster Secrets of every namespace Argo CD runs in, one list each.
pub(super) async fn read_destinations<R: Reader>(
    reader: &R,
    servers: Result<Vec<(String, String)>, String>,
) -> Read {
    let (namespaces, found) = argo_namespaces(&read_argo_workloads(reader).await);
    let skipped = match found {
        ArgoFound::Labelled { skipped } => skipped,
        ArgoFound::Default => Vec::new(),
    };
    let (known, no_servers) = match servers {
        Ok(servers) => (servers, None),
        Err(why) => (Vec::new(), Some(why)),
    };
    let mut reads = Vec::new();
    for namespace in namespaces {
        let read = match reader.cluster_secrets(&namespace).await {
            Ok(secrets) if secrets.capped => NamespaceRead::Capped,
            Ok(secrets) => NamespaceRead::Found(
                secrets
                    .items
                    .into_iter()
                    .map(|secret| Found {
                        contexts: contexts_of(&known, &secret),
                        secret,
                    })
                    .collect(),
            ),
            Err(failure) => match failure.kind {
                FailureKind::Forbidden | FailureKind::Unauthorized => {
                    NamespaceRead::Refused(printable(&failure))
                }
                _ => NamespaceRead::Failed(printable(&failure)),
            },
        };
        reads.push((namespace, read));
    }
    Read::Done {
        namespaces: reads,
        skipped,
        no_servers,
    }
}

/// The contexts whose server is the Secret's, with both normalized as an
/// Application's destination is. Argo CD's own cluster address is never
/// compared: from inside a cluster it names no kubeconfig server.
pub(super) fn contexts_of(servers: &[(String, String)], secret: &ClusterSecret) -> Vec<String> {
    let Some(server) = secret.server.text() else {
        return Vec::new();
    };
    let wanted = normalize_server(server);
    if wanted == normalize_server(IN_CLUSTER_SERVER) {
        return Vec::new();
    }
    servers
        .iter()
        .filter(|(_, own)| normalize_server(own) == wanted)
        .map(|(context, _)| context.clone())
        .collect()
}

// ---- the suggestions ------------------------------------------------------

/// What a found destination comes to.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Verdict {
    /// Map `rows` to the entry.
    Suggest { entry: String, rows: Vec<Key> },
    /// The server is more than one entry's.
    Ambiguous(Vec<String>),
    /// Contexts have the server, and no workspace entry opens them.
    NotAnEntry(Vec<String>),
    /// No kubeconfig context has the server.
    NoMatch,
    /// It can't be said, with the reason.
    Unknown(String),
}

impl Verdict {
    /// Whether it can be accepted.
    pub fn is_suggestion(&self) -> bool {
        matches!(self, Verdict::Suggest { .. })
    }
}

/// One found destination, as the list shows it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Item {
    /// What identifies it: its server, else its name.
    pub key: Key,
    /// `server https://… · name prod-fra`.
    pub destination: String,
    /// What it was found from.
    pub found_from: String,
    pub verdict: Verdict,
}

/// A line about the read itself.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Note {
    pub warn: bool,
    pub text: String,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Suggestions {
    pub items: Vec<Item>,
    pub notes: Vec<Note>,
}

/// The suggestions `findings` make for `workspace`. Pure: the same findings
/// and setup give the same items, so a save, a Reload or a hand edit derives
/// them again without a read.
pub fn suggest_destinations(findings: &DestinationFindings, workspace: &Workspace) -> Suggestions {
    let mut out = Suggestions::default();
    let note = |out: &mut Suggestions, warn, text: String| out.notes.push(Note { warn, text });
    let entry = &findings.entry;
    let mapped = |key: &Key| workspace.destinations.iter().any(|row| row.key().same(key));
    let (namespaces, skipped, no_servers) = match &findings.read {
        Read::Done {
            namespaces,
            skipped,
            no_servers,
        } => (namespaces.as_slice(), skipped.as_slice(), no_servers),
        // Argo CD's own cluster needs no Secret, so it stays an item
        // whatever the read came to.
        Read::NotRead(why) => {
            note(&mut out, false, format!("Nothing was read: {why}"));
            out.items
                .extend(in_cluster_item(&[], workspace, findings, &mapped));
            return out;
        }
        Read::Unreached(why) => {
            note(
                &mut out,
                true,
                format!("{entry} couldn't be reached to list Argo CD's clusters: {why}"),
            );
            out.items
                .extend(in_cluster_item(&[], workspace, findings, &mapped));
            return out;
        }
    };
    if let Some(why) = no_servers {
        note(
            &mut out,
            true,
            format!("No kubeconfig server is known to compare: {why}"),
        );
    }
    if !skipped.is_empty() {
        note(
            &mut out,
            false,
            format!(
                "Argo CD also runs in {}, past the cap; its clusters weren't listed",
                skipped.join(", ")
            ),
        );
    }
    let mut secrets: Vec<&Found> = Vec::new();
    let mut failed = false;
    for (namespace, read) in namespaces {
        match read {
            NamespaceRead::Found(found) => secrets.extend(found),
            NamespaceRead::Capped => {
                failed = true;
                note(
                    &mut out,
                    true,
                    format!(
                        "More than {MAX_CLUSTER_SECRETS} cluster Secrets in {namespace}; nothing \
                         suggested from them"
                    ),
                );
            }
            NamespaceRead::Refused(why) => {
                failed = true;
                note(
                    &mut out,
                    true,
                    format!(
                        "Listing Secrets in {namespace} on {entry} was refused ({why}); map \
                         destinations by hand. RBAC can't narrow a Secret list by label, so \
                         this needs list on secrets there"
                    ),
                );
            }
            NamespaceRead::Failed(why) => {
                failed = true;
                note(
                    &mut out,
                    true,
                    format!("Listing Secrets in {namespace} on {entry} failed: {why}"),
                );
            }
        }
    }
    let mapped = |key: &Key| workspace.destinations.iter().any(|row| row.key().same(key));
    let in_cluster =
        |secret: &Found| {
            secret.secret.server.text().is_some_and(|server| {
                normalize_server(server) == normalize_server(IN_CLUSTER_SERVER)
            })
        };
    let (own, others): (Vec<&Found>, Vec<&Found>) = secrets
        .iter()
        .copied()
        .partition(|secret| in_cluster(secret));
    if let Some(item) = in_cluster_item(&own, workspace, findings, &mapped) {
        out.items.push(item);
    }
    let mut unreadable = 0;
    for found in &others {
        match secret_item(found, workspace, findings, no_servers.is_some(), &mapped) {
            Ok(Some(item)) => out.items.push(item),
            Ok(None) => {}
            Err(()) => unreadable += 1,
        }
    }
    if unreadable > 0 {
        note(
            &mut out,
            true,
            format!("{unreadable} cluster Secrets have no readable server or name"),
        );
    }
    if out.items.is_empty() && !failed {
        let text = match secrets.is_empty() {
            true => "No cluster Secrets found".to_owned(),
            false => "Every cluster Secret is mapped".to_owned(),
        };
        note(&mut out, false, text);
    }
    out
}

fn destination_words(server: Option<&str>, name: Option<&str>) -> String {
    match (server, name) {
        (Some(server), Some(name)) => format!("server {server} · name {name}"),
        (Some(server), None) => format!("server {server}"),
        (None, Some(name)) => format!("name {name}"),
        (None, None) => String::new(),
    }
}

/// Argo CD's own cluster: one item made from what Settings knows of where
/// Argo CD is read, and joined by any Secret that names the same address.
fn in_cluster_item(
    own: &[&Found],
    workspace: &Workspace,
    findings: &DestinationFindings,
    mapped: &dyn Fn(&Key) -> bool,
) -> Option<Item> {
    let key = Key::Server(IN_CLUSTER_SERVER.to_owned());
    if mapped(&key) {
        return None;
    }
    let found_from;
    let verdict;
    match Sources::of(workspace).get(Part::ArgoCd) {
        Chosen::Entry { entry, why } => {
            let mut rows = vec![key.clone(), Key::Name(IN_CLUSTER_NAME.to_owned())];
            for found in own {
                if let Some(name) = found.secret.name.text()
                    && name != IN_CLUSTER_NAME
                {
                    rows.push(Key::Name(name.to_owned()));
                }
            }
            rows.retain(|row| !mapped(row));
            found_from = format!("Argo CD is read from {} ({})", entry.id, why.words());
            verdict = Verdict::Suggest {
                entry: entry.id.clone(),
                rows,
            };
        }
        chosen => {
            found_from = format!("Argo CD is read from {}", findings.entry);
            verdict = Verdict::Unknown(chosen.missing().unwrap_or_else(|| {
                "Argo CD is read from the open cluster, which the workspace doesn't list".to_owned()
            }));
        }
    }
    Some(Item {
        key,
        destination: destination_words(Some(IN_CLUSTER_SERVER), Some(IN_CLUSTER_NAME)),
        found_from,
        verdict,
    })
}

/// One cluster Secret as an item; `Err` when it has neither a readable
/// server nor a readable name, and `Ok(None)` when it is already mapped.
fn secret_item(
    found: &Found,
    workspace: &Workspace,
    findings: &DestinationFindings,
    no_servers: bool,
    mapped: &dyn Fn(&Key) -> bool,
) -> Result<Option<Item>, ()> {
    let secret = &found.secret;
    let (server, name) = (secret.server.text(), secret.name.text());
    let key = match (server, name) {
        (Some(server), _) => Key::Server(server.to_owned()),
        (None, Some(name)) => Key::Name(name.to_owned()),
        (None, None) => return Err(()),
    };
    // A row that already maps the server (or, with no server, the name)
    // leaves nothing to suggest.
    if mapped(&key) {
        return Ok(None);
    }
    let mut from = format!(
        "Secret {} in {} on {}",
        secret.secret, secret.namespace, findings.entry
    );
    if !found.contexts.is_empty() {
        let noun = if found.contexts.len() == 1 {
            "context"
        } else {
            "contexts"
        };
        from.push_str(&format!("; {noun} {}", found.contexts.join(", ")));
    }
    let verdict = verdict_of(found, workspace, no_servers, server, name, mapped);
    Ok(Some(Item {
        destination: destination_words(server, name),
        key,
        found_from: from,
        verdict,
    }))
}

fn verdict_of(
    found: &Found,
    workspace: &Workspace,
    no_servers: bool,
    server: Option<&str>,
    name: Option<&str>,
    mapped: &dyn Fn(&Key) -> bool,
) -> Verdict {
    let Some(server) = server else {
        return Verdict::Unknown("its server couldn't be read from the Secret".to_owned());
    };
    if no_servers {
        return Verdict::Unknown(
            "no kubeconfig could be read, so its server can't be compared".to_owned(),
        );
    }
    let mut entries: Vec<String> = Vec::new();
    for context in &found.contexts {
        for entry in workspace.clusters.iter().filter(|e| &e.context == context) {
            if !entries.contains(&entry.id) {
                entries.push(entry.id.clone());
            }
        }
    }
    match entries.len() {
        0 if found.contexts.is_empty() => Verdict::NoMatch,
        0 => Verdict::NotAnEntry(found.contexts.clone()),
        1 => {
            let mut rows = vec![Key::Server(server.to_owned())];
            rows.extend(name.map(|name| Key::Name(name.to_owned())));
            rows.retain(|row| !mapped(row));
            Verdict::Suggest {
                entry: entries.remove(0),
                rows,
            }
        }
        _ => Verdict::Ambiguous(entries),
    }
}
