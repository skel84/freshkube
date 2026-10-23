//! The reads behind the route trace: API discovery for the Gateway API group,
//! then bounded lists of routes, Gateways, classes, grants and (only when a
//! listener selects by label) namespaces, then each distinct backend Service.
//! All of them are lists or gets; none writes, and none reads a Secret. A
//! refused read lands in the trace as Unknown with its reason.
use super::facts::*;
use super::trace::{BackendRead, Inputs, Listing, RouteTrace, trace_route};
use crate::failure::{Failure, FailureKind};
use crate::resources::object::{JSON, json_get};
use crate::resources::traffic::{ServiceFacts, read_service};
use crate::resources::{ResourceKind, list_custom_groups, list_group_kinds};
use futures::StreamExt;
use k8s_openapi::api::core::v1::{Namespace, Service};
use kube::{Api, Client};
use serde_json::Value;
use std::collections::BTreeMap;

pub const ROUTE_LIMIT: u32 = 500;
pub const GATEWAY_LIMIT: u32 = 200;
pub const CLASS_LIMIT: u32 = 100;
pub const GRANT_LIMIT: u32 = 500;
pub const NAMESPACE_LIMIT: u32 = 500;
/// Distinct backend Services one read follows; the rest are counted.
pub const BACKEND_LIMIT: usize = 100;
/// Backend reads in flight at once.
const BACKEND_PARALLEL: usize = 10;

/// The kinds the trace reads, each at the version the cluster serves it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GatewayKinds {
    pub route: ResourceKind,
    pub gateway: ResourceKind,
    pub class: ResourceKind,
    /// Absent when ReferenceGrant is not served: no grant can exist.
    pub grant: Option<ResourceKind>,
}

/// What the cluster serves of the Gateway API.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ApiState {
    /// No `gateway.networking.k8s.io` group at all.
    NotInstalled,
    /// The group is there, but not every kind the trace needs.
    Incomplete { missing: Vec<&'static str> },
    /// Usable. `mixed` says so when HTTPRoute, Gateway and GatewayClass are
    /// not served at one version, which is a bundle installed in pieces.
    Ready { mixed: Option<String> },
}

impl ApiState {
    /// The plain reading of an API that can't be traced, for the page.
    pub fn problem(&self) -> Option<String> {
        match self {
            Self::NotInstalled => Some(
                "The Gateway API is not installed in this cluster, so there are no Gateways or HTTPRoutes to trace."
                    .into(),
            ),
            Self::Incomplete { missing } => Some(format!(
                "The Gateway API is only partly installed: {} not served.",
                missing.join(", ")
            )),
            Self::Ready { .. } => None,
        }
    }

    pub fn mixed(&self) -> Option<&str> {
        match self {
            Self::Ready { mixed } => mixed.as_deref(),
            _ => None,
        }
    }
}

#[derive(Clone, Debug)]
#[non_exhaustive]
pub struct RouteEntry {
    pub facts: RouteFacts,
    pub trace: RouteTrace,
}

/// Everything one read of the Gateway API found.
#[derive(Clone, Debug)]
#[non_exhaustive]
pub struct RoutesRead {
    pub api: ApiState,
    pub routes: Vec<RouteEntry>,
    /// HTTPRoute at the version the cluster serves it, for a link to one.
    pub route_kind: Option<ResourceKind>,
    /// The route list stopped at its limit.
    pub routes_cut: bool,
    /// Distinct backends past the limit, which read Unknown.
    pub backends_cut: usize,
    /// Reads that were refused or cut, in words, so a banner can say what the
    /// verdicts rest on.
    pub notes: Vec<String>,
}

impl RoutesRead {
    pub fn new(api: ApiState) -> Self {
        Self {
            api,
            routes: Vec::new(),
            route_kind: None,
            routes_cut: false,
            backends_cut: 0,
            notes: Vec::new(),
        }
    }

    pub fn with_routes(mut self, routes: Vec<RouteEntry>) -> Self {
        self.routes = routes;
        self
    }

    pub fn with_route_kind(mut self, kind: ResourceKind) -> Self {
        self.route_kind = Some(kind);
        self
    }

    /// The route list stopped at `limit`.
    pub(super) fn set_cut(&mut self, limit: u32) {
        self.routes_cut = true;
        self.notes
            .push(format!("Routes were cut at {limit}; more may exist"));
    }

    pub fn with_notes(mut self, notes: Vec<String>) -> Self {
        self.notes = notes;
        self
    }
}

impl RouteEntry {
    pub fn new(facts: RouteFacts, trace: RouteTrace) -> Self {
        Self { facts, trace }
    }
}

/// Resolves the API: the group's kinds and whether they come from one bundle.
pub async fn discover(client: &Client) -> Result<(ApiState, Option<GatewayKinds>), Failure> {
    let group = list_custom_groups(client)
        .await?
        .into_iter()
        .find(|g| g.name == GATEWAY_GROUP);
    let Some(group) = group else {
        return Ok((ApiState::NotInstalled, None));
    };
    let served = list_group_kinds(client, &group).await?;
    let find = |kind: &str| served.kinds.iter().find(|k| k.kind == kind).cloned();
    let (route, gateway, class) = (find("HTTPRoute"), find("Gateway"), find("GatewayClass"));
    let missing = [
        ("HTTPRoute", route.is_none()),
        ("Gateway", gateway.is_none()),
        ("GatewayClass", class.is_none()),
    ]
    .into_iter()
    .filter_map(|(name, absent)| absent.then_some(name))
    .collect::<Vec<_>>();
    let (Some(route), Some(gateway), Some(class)) = (route, gateway, class) else {
        return Ok((ApiState::Incomplete { missing }, None));
    };
    let mixed = (route.version != gateway.version || gateway.version != class.version).then(|| {
        format!(
            "HTTPRoute {}, Gateway {}, GatewayClass {}",
            route.version, gateway.version, class.version
        )
    });
    let kinds = GatewayKinds {
        route,
        gateway,
        class,
        grant: find("ReferenceGrant"),
    };
    Ok((ApiState::Ready { mixed }, Some(kinds)))
}

async fn list_json(client: &Client, kind: &ResourceKind, limit: u32) -> Result<Listing<Value>, Failure> {
    let path = format!("{}?limit={limit}", kind.collection_path(None));
    let list: Value = client
        .request(json_get(path, JSON)?)
        .await
        .map_err(Failure::from_kube)?;
    let cut = list["metadata"]["continue"]
        .as_str()
        .is_some_and(|token| !token.is_empty());
    let items = list["items"].as_array().cloned().unwrap_or_default();
    Ok(Listing::new(items, cut))
}

async fn list_as<T>(
    client: &Client,
    kind: &ResourceKind,
    limit: u32,
    parse: fn(&Value) -> Option<T>,
) -> Result<Listing<T>, Failure> {
    let list = list_json(client, kind, limit).await?;
    Ok(Listing::new(list.items.iter().filter_map(parse).collect(), list.cut))
}

async fn read_backend(client: &Client, namespace: &str, name: &str) -> BackendRead {
    let service = Api::<Service>::namespaced(client.clone(), namespace)
        .get(name)
        .await
        .map_err(Failure::from_kube);
    match service {
        Ok(service) => Ok(Some(
            read_service(client, ServiceFacts::from_service(&service), false).await,
        )),
        Err(failure) if failure.kind == FailureKind::NotFound => Ok(None),
        Err(failure) => Err(failure),
    }
}

fn note<T>(what: &str, result: &Result<Listing<T>, Failure>, notes: &mut Vec<String>) {
    match result {
        Err(failure) => notes.push(format!(
            "{what} could not be read ({}: {}); the steps that need them read Unknown",
            failure.kind.label(),
            failure.message
        )),
        Ok(list) if list.cut => notes.push(format!("{what} were cut at their limit")),
        Ok(_) => {}
    }
}

/// A grant version safe to write into text to copy.
pub(super) fn grant_version(kinds: &GatewayKinds) -> String {
    let version = kinds.grant.as_ref().map_or("v1beta1", |k| k.version.as_str());
    let digits = |s: &str| !s.is_empty() && s.bytes().all(|b| b.is_ascii_digit());
    let plain = version.strip_prefix('v').is_some_and(|rest| {
        let (major, rest) = rest.split_at(rest.bytes().take_while(u8::is_ascii_digit).count());
        digits(major)
            && (rest.is_empty()
                || rest
                    .strip_prefix("alpha")
                    .or_else(|| rest.strip_prefix("beta"))
                    .is_some_and(digits))
    });
    if plain { version.to_owned() } else { "v1beta1".into() }
}

/// Reads and traces every HTTPRoute, or, with `only`, the one named
/// `(namespace, name)`. An error is the discovery or the route read failing;
/// everything else lands in the traces.
pub async fn read_routes(client: &Client, only: Option<(&str, &str)>) -> Result<RoutesRead, Failure> {
    let (api, kinds) = discover(client).await?;
    let Some(kinds) = kinds else {
        return Ok(RoutesRead::new(api));
    };
    let routes = async {
        match only {
            Some((namespace, name)) => {
                let object: Value = client
                    .request(json_get(kinds.route.object_path(Some(namespace), name), JSON)?)
                    .await
                    .map_err(Failure::from_kube)?;
                Ok(Listing::new(vec![object], false))
            }
            None => list_json(client, &kinds.route, ROUTE_LIMIT).await,
        }
    };
    let grants = async {
        match &kinds.grant {
            Some(kind) => list_as(client, kind, GRANT_LIMIT, GrantFacts::parse).await,
            // No ReferenceGrant API means no grant can exist.
            None => Ok(Listing::new(Vec::new(), false)),
        }
    };
    let (routes, gateways, classes, grants) = futures::join!(
        routes,
        list_as(client, &kinds.gateway, GATEWAY_LIMIT, GatewayFacts::parse),
        list_as(client, &kinds.class, CLASS_LIMIT, ClassFacts::parse),
        grants,
    );
    let routes = routes?;
    let facts = routes
        .items
        .iter()
        .filter_map(RouteFacts::parse)
        .collect::<Vec<_>>();
    let mut notes = Vec::new();
    note("Gateways", &gateways, &mut notes);
    note("GatewayClasses", &classes, &mut notes);
    note("ReferenceGrants", &grants, &mut notes);

    let selects = gateways.as_ref().is_ok_and(|g| {
        g.items.iter().any(|g| {
            g.listeners
                .iter()
                .any(|l| matches!(l.namespaces, AllowedNamespaces::Selector(_)))
        })
    });
    let namespaces = if selects {
        let list = Api::<Namespace>::all(client.clone())
            .list(&kube::api::ListParams::default().limit(NAMESPACE_LIMIT))
            .await
            .map_err(Failure::from_kube)
            .map(|list| {
                let cut = list.metadata.continue_.as_deref().is_some_and(|t| !t.is_empty());
                Listing::new(
                    list.items
                        .into_iter()
                        .map(|ns| {
                            (
                                ns.metadata.name.unwrap_or_default(),
                                ns.metadata.labels.unwrap_or_default(),
                            )
                        })
                        .collect(),
                    cut,
                )
            });
        note("Namespaces", &list, &mut notes);
        list
    } else {
        Ok(Listing::new(Vec::new(), false))
    };

    let mut wanted: Vec<(String, String)> = Vec::new();
    for route in &facts {
        for backend in route.rules.iter().flat_map(|r| &r.backends) {
            let key = (
                backend.namespace.clone().unwrap_or_else(|| route.namespace.clone()),
                backend.name.clone(),
            );
            if backend.is_service() && backend.weight > 0 && !wanted.contains(&key) {
                wanted.push(key);
            }
        }
    }
    let backends_cut = wanted.len().saturating_sub(BACKEND_LIMIT);
    wanted.truncate(BACKEND_LIMIT);
    if backends_cut > 0 {
        notes.push(format!(
            "{backends_cut} backend Services past {BACKEND_LIMIT} were not read; their steps read Unknown"
        ));
    }
    let backends = futures::stream::iter(wanted.into_iter().map(|(namespace, name)| async move {
        let read = read_backend(client, &namespace, &name).await;
        ((namespace, name), read)
    }))
    .buffer_unordered(BACKEND_PARALLEL)
    .collect::<BTreeMap<_, _>>()
    .await;

    let version = grant_version(&kinds);
    let inputs = Inputs {
        gateways: &gateways,
        classes: &classes,
        grants: &grants,
        namespaces: &namespaces,
        backends: &backends,
        grant_version: &version,
    };
    let routes_cut = routes.cut;
    let route_kind = Some(kinds.route.clone());
    let mut notes_out = notes;
    if routes_cut {
        notes_out.push(format!("Routes were cut at {ROUTE_LIMIT}; more may exist"));
    }
    let entries = facts
        .into_iter()
        .map(|facts| {
            let trace = trace_route(&facts, &inputs);
            RouteEntry { facts, trace }
        })
        .collect();
    Ok(RoutesRead {
        api,
        routes: entries,
        route_kind,
        routes_cut,
        backends_cut,
        notes: notes_out,
    })
}
