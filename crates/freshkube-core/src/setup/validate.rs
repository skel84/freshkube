//! The rules a setup must meet beyond its shape, after the workspace's own.
//! A fault names a field or an entry by id and never repeats a value: a
//! value could be a server, a path or, by mistake, a key.

use std::path::Path;

use super::{
    Coroot, Delivery, EntryCoroot, EntryMetrics, Fault, Metrics, Mode, Results, Source, Store,
    SupplyChain, types::MetricsService,
};
use crate::{base_url::parse_base_url, workspace::Entry};

pub(super) fn entry(entry: &Entry) -> Result<(), Fault> {
    let at = |field: &str| format!("the cluster “{}” {field}", entry.id);
    entry_at(entry, &at)
}

pub(super) fn entry_fields(entry: &Entry) -> Result<(), Fault> {
    entry_at(entry, &str::to_owned)
}

fn entry_at(entry: &Entry, at: &dyn Fn(&str) -> String) -> Result<(), Fault> {
    if let Some(fingerprint) = &entry.fingerprint
        && !crate::fingerprint::valid_override(fingerprint)
    {
        return Err(Fault::Fingerprint(at("fingerprint")));
    }
    if let Some(results) = &entry.results {
        results_of(results, at)?;
    }
    if let Some(coroot) = &entry.coroot {
        coroot_of(coroot, at)?;
    }
    if let Some(metrics) = &entry.metrics {
        entry_metrics(metrics, at)?;
    }
    Ok(())
}

fn results_of(results: &Results, at: &dyn Fn(&str) -> String) -> Result<(), Fault> {
    if let Some(path) = &results.ca_file {
        absolute(path, at("results.ca_file"))?;
    }
    match (&results.address, &results.forward) {
        (Some(address), None) => results_address(address, at("results.address")),
        (None, Some(forward)) => {
            non_empty(&forward.namespace, at("results.forward.namespace"))?;
            non_empty(&forward.service, at("results.forward.service"))?;
            port(forward.port, at("results.forward.port"))
        }
        _ => Err(Fault::OneOf {
            at: at("results"),
            of: "address or forward",
        }),
    }
}

/// Typed Results addresses use the same literal-loopback HTTP rule as the
/// client. Other sources retain their existing HTTP(S) validation.
fn results_address(text: &str, at: String) -> Result<(), Fault> {
    let address = parse_base_url(text.trim()).map_err(|_| Fault::NotUrl(at.clone()))?;
    let loopback = match address.host() {
        Some(url::Host::Ipv4(ip)) => ip.is_loopback(),
        Some(url::Host::Ipv6(ip)) => ip.is_loopback(),
        _ => false,
    };
    if address.scheme() == "http" && !loopback {
        return Err(Fault::NonLoopbackHttp(at));
    }
    Ok(())
}

fn coroot_of(coroot: &EntryCoroot, at: &dyn Fn(&str) -> String) -> Result<(), Fault> {
    non_empty(&coroot.cluster, at("coroot.cluster"))?;
    if let Some(project) = &coroot.project {
        non_empty(project, at("coroot.project"))?;
    }
    Ok(())
}

/// Each linked entry's project is its own, else the connection's. Two entries
/// may not link one (project, Coroot cluster): a Coroot application would
/// belong to both, and no answer could say whose it is.
pub(super) fn coroot_links(entries: &[Entry], connection: Option<&str>) -> Result<(), Fault> {
    let mut seen: Vec<(&str, &str, &str)> = Vec::new();
    for entry in entries {
        let Some(link) = &entry.coroot else { continue };
        let Some(project) = link.project.as_deref().or(connection) else {
            return Err(Fault::CorootWithoutProject(entry.id.clone()));
        };
        let cluster = link.cluster.as_str();
        if let Some((first, ..)) = seen.iter().find(|(_, p, c)| *p == project && *c == cluster) {
            return Err(Fault::CorootTwice {
                first: (*first).to_owned(),
                second: entry.id.clone(),
                project: project.to_owned(),
                cluster: cluster.to_owned(),
            });
        }
        seen.push((&entry.id, project, cluster));
    }
    Ok(())
}

fn entry_metrics(metrics: &EntryMetrics, at: &dyn Fn(&str) -> String) -> Result<(), Fault> {
    if let Some(value) = &metrics.value {
        non_empty(value, at("metrics.value"))?;
    }
    if let Some(source) = &metrics.source {
        source_of(source, &at("metrics.source"))?;
    }
    Ok(())
}

/// An entry's own store: a Service or a URL.
fn source_of(source: &Source, at: &str) -> Result<(), Fault> {
    match (&source.service, &source.url) {
        (Some(service), None) => service_of(service, at)?,
        (None, Some(address)) => url(address, format!("{at}.url"))?,
        _ => {
            return Err(Fault::OneOf {
                at: at.into(),
                of: "service or url",
            });
        }
    }
    token_of(source.token.is_some(), source.url.is_some(), at)
}

pub(super) fn metrics(metrics: &Metrics) -> Result<(), Fault> {
    if metrics.mode != Mode::Central {
        // Kept, unread: switching modes back and forth loses nothing.
        return Ok(());
    }
    let Some(store) = &metrics.store else {
        return Err(Fault::CentralWithoutStore);
    };
    store_of(store)?;
    let label = metrics.label_name();
    if !is_label_name(label) {
        return Err(Fault::LabelName("metrics.label".into()));
    }
    Ok(())
}

fn store_of(store: &Store) -> Result<(), Fault> {
    let at = "metrics.store";
    match (&store.entry, &store.service, &store.url) {
        (Some(entry), Some(service), None) => {
            non_empty(entry, format!("{at}.entry"))?;
            service_of(service, at)?;
        }
        (None, None, Some(address)) => url(address, format!("{at}.url"))?,
        _ => {
            return Err(Fault::OneOf {
                at: at.into(),
                of: "entry with service, or url",
            });
        }
    }
    token_of(store.token.is_some(), store.url.is_some(), at)
}

/// A Service needs no token: the hosting entry's credentials reach it.
fn token_of(token: bool, url: bool, at: &str) -> Result<(), Fault> {
    if token && !url {
        return Err(Fault::Only {
            at: format!("{at}.token"),
            with: "url",
        });
    }
    Ok(())
}

fn service_of(service: &MetricsService, at: &str) -> Result<(), Fault> {
    non_empty(&service.namespace, format!("{at}.service.namespace"))?;
    non_empty(&service.name, format!("{at}.service.name"))?;
    port(service.port, format!("{at}.service.port"))
}

/// A Prometheus label name: `[a-zA-Z_][a-zA-Z0-9_]*`.
pub(crate) fn is_label_name(name: &str) -> bool {
    let mut chars = name.chars();
    chars
        .next()
        .is_some_and(|c| c.is_ascii_alphabetic() || c == '_')
        && chars.all(|c| c.is_ascii_alphanumeric() || c == '_')
}

pub(super) fn coroot(coroot: &Coroot) -> Result<(), Fault> {
    url(&coroot.server, "coroot.server".into())?;
    if coroot.key.is_some() && coroot.api_key_file.is_some() {
        return Err(Fault::KeyTwice);
    }
    if let Some(file) = &coroot.api_key_file {
        absolute(file, "coroot.api_key_file".into())?;
    }
    if let Some(project) = &coroot.project {
        non_empty(project, "coroot.project".into())?;
    }
    Ok(())
}

pub(super) fn supply_chain(chain: &SupplyChain) -> Result<(), Fault> {
    if let Some(address) = &chain.transparency_url {
        let at = "supply_chain.transparency_url";
        let parsed = parse_base_url(address).map_err(|_| Fault::NotHttpsUrl(at.into()))?;
        if parsed.scheme() != "https" {
            return Err(Fault::NotHttpsUrl(at.into()));
        }
    }
    if let Some(file) = &chain.public_key_file {
        absolute(file, "supply_chain.public_key_file".into())?;
    }
    if let Some(evidence) = &chain.evidence {
        let at = "supply_chain.evidence";
        non_empty(&evidence.result, format!("{at}.result"))?;
        pointer(&evidence.commit, format!("{at}.commit"))?;
        pointer(&evidence.digest, format!("{at}.digest"))?;
    }
    Ok(())
}

pub(super) fn delivery(delivery: &Delivery) -> Result<(), Fault> {
    if let Some(namespace) = &delivery.build_namespace {
        non_empty(namespace, "delivery.build_namespace".into())?;
        namespace_name(
            &namespace.replace("{project}", "p"),
            "delivery.build_namespace".into(),
        )?;
    }
    if let Some(naming) = &delivery.stage_naming {
        let at = "delivery.stage_naming";
        non_empty(&naming.name_template, format!("{at}.name_template"))?;
        if !naming.name_template.contains("{stage}") {
            return Err(Fault::TemplateWithoutStage(format!("{at}.name_template")));
        }
        if let Some(key) = &naming.project_key {
            non_empty(key, format!("{at}.project_key"))?;
        }
    }
    Ok(())
}

/// Resolves a build namespace for one Kargo project and checks the result.
/// A bad result refuses this change's build reads, not the setup file.
pub fn resolve_build_namespace(template: &str, project: &str) -> Result<String, String> {
    let name = template.replace("{project}", project);
    namespace_name(&name, "delivery.build_namespace".into()).map_err(|_| {
        format!(
            "delivery.build_namespace resolves to \"{name}\" for project {project}, which isn't a namespace name"
        )
    })?;
    Ok(name)
}

/// A DNS-1123 label: lowercase alphanumerics and `-`, at most 63, starting
/// and ending alphanumeric.
fn namespace_name(text: &str, at: String) -> Result<(), Fault> {
    let alnum = |c: char| c.is_ascii_lowercase() || c.is_ascii_digit();
    let fine = text.len() <= 63
        && text.chars().all(|c| alnum(c) || c == '-')
        && text.chars().next().is_some_and(alnum)
        && text.chars().last().is_some_and(alnum);
    if fine {
        Ok(())
    } else {
        Err(Fault::NotNamespace(at))
    }
}

fn non_empty(text: &str, at: String) -> Result<(), Fault> {
    if text.trim().is_empty() {
        return Err(Fault::Empty(at));
    }
    Ok(())
}

fn port(port: u16, at: String) -> Result<(), Fault> {
    if port == 0 {
        return Err(Fault::ZeroPort(at));
    }
    Ok(())
}

fn url(text: &str, at: String) -> Result<(), Fault> {
    parse_base_url(text.trim())
        .map(drop)
        .map_err(|_| Fault::NotUrl(at))
}

fn absolute(path: &Path, at: String) -> Result<(), Fault> {
    if !path.is_absolute() {
        return Err(Fault::RelativePath(at));
    }
    Ok(())
}

fn pointer(text: &str, at: String) -> Result<(), Fault> {
    if !text.starts_with('/') {
        return Err(Fault::Pointer(at));
    }
    Ok(())
}
