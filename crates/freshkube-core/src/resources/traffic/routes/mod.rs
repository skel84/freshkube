//! The HTTPRoute trace and the Routes list (#949, slice N2a). Read-only, and
//! no UI types: the Routes page and the Resources pane draw what this
//! answers. A route is traced per parent through the class, the Gateway, a
//! listener, the namespace, the references, the Service and its endpoints,
//! stopping at the first break and saying whose problem it is. Configuration
//! and the controller's status are separate lines, and a read that was
//! refused or not served is Unknown with its reason, never "not serving".
pub mod example;
mod facts;
mod gateways;
mod grant;
mod read;
mod topology;
mod trace;

pub use facts::{
    AllowedNamespaces, BackendRef, ClassFacts, Cond, GATEWAY_GROUP, GatewayFacts, GrantFacts,
    Listener, ListenerStatus, ParentRef, ParentStatus, RouteFacts, Rule, Selector, Tri,
};
pub use gateways::{
    AttachedRoute, ClassEntry, ClassState, Contested, GatewayEntry, GatewayState, GatewaysView,
    ListenerRow, ListenerState,
};
pub use grant::{GrantProposal, proposal};
pub use read::{
    ApiState, BACKEND_LIMIT, CLASS_LIMIT, GATEWAY_LIMIT, GRANT_LIMIT, GatewayKinds,
    NAMESPACE_LIMIT, ROUTE_LIMIT, RouteEntry, RoutesRead, discover, read_routes,
};
pub use topology::{
    Edge, Health, ListCuts, MAX_BOXES, Node, NodeKind, Target, Topology, WORKLOAD_LIMIT,
    WORKLOAD_NAMESPACES, WorkloadRead, WorkloadRef, read_workloads, resolve_workloads,
};
pub use trace::{
    BackendRead, Branch, BranchKind, BranchVerdict, Cause, Inputs, Listing, NamespaceLabels,
    OtherController, Owner, ParentTrace, RouteTrace, RouteVerdict, StepId, StepVerdict, Summary,
    TraceStep, trace_route,
};

#[cfg(test)]
mod gateway_tests;
#[cfg(test)]
mod tests;
