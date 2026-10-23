//! The ReferenceGrant a refused cross-namespace reference would need, as text
//! to copy. Nothing in the app applies it: applying a grant is a later slice
//! through the write model.

#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct GrantProposal {
    /// The namespace the grant belongs in: the Service's.
    pub namespace: String,
    pub name: String,
    pub from_namespace: String,
    pub service: String,
    pub yaml: String,
}

/// A grant scoped to this exact pair: HTTPRoutes in `from` may refer to the
/// Service `service` in `to`, and to nothing else there. The caller has
/// checked that every name is a DNS name, so none can add a line.
pub fn proposal(version: &str, from: &str, to: &str, service: &str) -> GrantProposal {
    let name = format!("allow-{from}-to-{service}");
    let name = name.chars().take(253).collect::<String>();
    let yaml = format!(
        "apiVersion: gateway.networking.k8s.io/{version}\n\
         kind: ReferenceGrant\n\
         metadata:\n  name: {name}\n  namespace: {to}\n\
         spec:\n  from:\n    - group: gateway.networking.k8s.io\n      kind: HTTPRoute\n      namespace: {from}\n  \
         to:\n    - group: \"\"\n      kind: Service\n      name: {service}\n"
    );
    GrantProposal {
        namespace: to.to_owned(),
        name,
        from_namespace: from.to_owned(),
        service: service.to_owned(),
        yaml,
    }
}
