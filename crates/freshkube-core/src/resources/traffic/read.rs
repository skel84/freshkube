//! The reads behind the chain: Services, EndpointSlices, the legacy Endpoints
//! and a Service's pods. All of them are lists or gets; none writes, and none
//! reads a Secret.
use super::chain::{ServiceReading, WorkloadServices};
use super::endpoints::{Published, SERVICE_LABEL, UnpublishedPod, unpublished_pods};
use super::service::ServiceFacts;
use crate::failure::{Failure, FailureKind};
use k8s_openapi::api::core::v1::{Endpoints, Pod, Service};
use k8s_openapi::api::discovery::v1::EndpointSlice;
use kube::{Api, Client, api::ListParams};

/// Slices read for one Service, and pods read for the unpublished list.
pub const SLICE_LIMIT: u32 = 200;
pub const POD_LIMIT: u32 = 500;
pub const SERVICE_LIMIT: u32 = 500;
/// Services a workload's chain follows; the rest are counted by the caller.
pub const CHAIN_SERVICES: usize = 10;

/// What a Service publishes: its EndpointSlices. The legacy Endpoints are
/// read only when the slice API is not served or its list is refused; an
/// empty slice list is an answer, "publishes nothing", from EndpointSlices.
pub async fn read_published(client: &Client, service: &ServiceFacts) -> Result<Published, Failure> {
    let slices = Api::<EndpointSlice>::namespaced(client.clone(), &service.namespace)
        .list(
            &ListParams::default()
                .labels(&format!("{SERVICE_LABEL}={}", service.name))
                .limit(SLICE_LIMIT),
        )
        .await
        .map_err(Failure::from_kube);
    match slices {
        Ok(list) => {
            let cut = list
                .metadata
                .continue_
                .as_deref()
                .is_some_and(|token| !token.is_empty());
            Ok(Published::from_slices(service, list.items, cut))
        }
        // Endpoints answer only where slices can't: not served, or refused.
        // A timeout, an unreachable cluster or a server error is reported.
        Err(failure)
            if matches!(
                failure.kind,
                FailureKind::NotFound | FailureKind::Forbidden | FailureKind::Unauthorized
            ) =>
        {
            Api::<Endpoints>::namespaced(client.clone(), &service.namespace)
                .get(&service.name)
                .await
                .map(|endpoints| Published::from_endpoints(service, &endpoints))
                .map_err(|_| failure)
        }
        Err(failure) => Err(failure),
    }
}

/// The pods a Service selects that nothing publishes.
async fn read_unpublished(
    client: &Client,
    service: &ServiceFacts,
    published: &Published,
) -> Result<(Vec<UnpublishedPod>, bool), Failure> {
    if service.selector.is_empty() {
        return Ok((Vec::new(), false));
    }
    let pods = Api::<Pod>::namespaced(client.clone(), &service.namespace)
        .list(
            &ListParams::default()
                .labels(&service.selector_query())
                .limit(POD_LIMIT),
        )
        .await
        .map_err(Failure::from_kube)?;
    let cut = pods
        .metadata
        .continue_
        .as_deref()
        .is_some_and(|token| !token.is_empty());
    Ok((unpublished_pods(service, published, &pods.items), cut))
}

/// One Service as read. `with_pods` adds the unpublished-pod read, which a
/// Service's own pane wants and a workload's chain does not.
pub async fn read_service(client: &Client, facts: ServiceFacts, with_pods: bool) -> ServiceReading {
    let published = read_published(client, &facts).await;
    let mut pods_cut = false;
    let unpublished = match (&published, with_pods) {
        (Ok(published), true) => Some(read_unpublished(client, &facts, published).await.map(
            |(pods, cut)| {
                pods_cut = cut;
                pods
            },
        )),
        // Without the endpoints, "unpublished" has nothing to compare to.
        (Err(failure), true) => Some(Err(failure.clone())),
        (_, false) => None,
    };
    let mut reading = ServiceReading::new(facts, published, unpublished);
    reading.pods_cut = pods_cut;
    reading
}

/// A Service by name, with its published endpoints and unpublished pods.
pub async fn read_service_by_name(
    client: &Client,
    namespace: &str,
    name: &str,
) -> Result<ServiceReading, Failure> {
    let service = Api::<Service>::namespaced(client.clone(), namespace)
        .get(name)
        .await
        .map_err(Failure::from_kube)?;
    Ok(read_service(client, ServiceFacts::from_service(&service), true).await)
}

/// The Services in `namespace` that select pods with `labels`, each with what
/// it publishes. The list failing, or being cut at its limit, is one error; a
/// Service whose endpoints fail is a reading with its own error.
pub async fn read_workload_services(
    client: &Client,
    namespace: &str,
    labels: &std::collections::BTreeMap<String, String>,
) -> Result<WorkloadServices, Failure> {
    let list = Api::<Service>::namespaced(client.clone(), namespace)
        .list(&ListParams::default().limit(SERVICE_LIMIT))
        .await
        .map_err(Failure::from_kube)?;
    if list
        .metadata
        .continue_
        .as_deref()
        .is_some_and(|token| !token.is_empty())
    {
        return Err(Failure::new(
            FailureKind::Other,
            format!("Services list cut at {SERVICE_LIMIT}"),
        ));
    }
    let mut selecting = list
        .items
        .iter()
        .map(ServiceFacts::from_service)
        .filter(|facts| facts.selects(labels))
        .collect::<Vec<_>>();
    let total = selecting.len();
    selecting.truncate(CHAIN_SERVICES);
    let readings = futures::future::join_all(
        selecting
            .into_iter()
            .map(|facts| read_service(client, facts, false)),
    )
    .await;
    Ok(WorkloadServices::new(readings, total))
}
