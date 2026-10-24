//! What a Service publishes and the traffic chain from a workload down to it
//! (#949, slice N1). Read-only, and no UI types: the pane draws what this
//! answers. Configuration (a Service's spec) and status (its slices) are
//! separate evidence, a refused read is unknown and never zero, and the chain
//! is a list of steps that later slices put routes and gateways in front of.
mod chain;
mod endpoints;
mod read;
pub mod routes;
mod service;

pub use chain::{
    Evidence, ServiceReading, StepKind, Subject, TrafficChain, TrafficStep, Verdict,
    WorkloadServices, service_chain, workload_chain,
};
pub use endpoints::{
    Endpoint, EndpointSource, EndpointState, Published, SERVICE_LABEL, UnpublishedPod,
    UnpublishedReason, unpublished_pods,
};
pub use read::{
    CHAIN_SERVICES, POD_LIMIT, SERVICE_LIMIT, SLICE_LIMIT, read_published, read_service,
    read_service_by_name, read_workload_services,
};
pub use service::{LoadBalancerState, ServiceFacts, ServiceKind, ServicePort, ServiceShape};

#[cfg(test)]
mod tests;
