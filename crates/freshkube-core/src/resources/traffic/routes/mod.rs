//! The HTTPRoute trace and the Routes list (#949, slice N2a). Read-only, and
//! no UI types: the Routes page and the Resources pane draw what this
//! answers. A route is traced per parent through the class, the Gateway, a
//! listener, the namespace, the references, the Service and its endpoints,
//! stopping at the first break and saying whose problem it is. Configuration
//! and the controller's status are separate lines, and a read that was
//! refused or not served is Unknown with its reason, never "not serving".
pub mod example;
mod facts;
mod grant;
mod read;
mod trace;

pub use facts::{
    AllowedNamespaces, BackendRef, ClassFacts, Cond, GATEWAY_GROUP, GatewayFacts, GrantFacts,
    Listener, ListenerStatus, ParentRef, ParentStatus, RouteFacts, Rule, Selector, Tri,
};
pub use grant::{GrantProposal, proposal};
pub use read::{
    ApiState, BACKEND_LIMIT, CLASS_LIMIT, GATEWAY_LIMIT, GRANT_LIMIT, GatewayKinds,
    NAMESPACE_LIMIT, ROUTE_LIMIT, RouteEntry, RoutesRead, discover, read_routes,
};
pub use trace::{
    BackendRead, Branch, BranchKind, BranchVerdict, Inputs, Listing, NamespaceLabels,
    OtherController, Owner, ParentTrace, RouteTrace, RouteVerdict, StepId, StepVerdict, Summary,
    TraceStep, trace_route,
};

#[cfg(test)]
mod tests;
