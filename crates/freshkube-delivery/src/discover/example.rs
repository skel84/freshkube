//! Example data for the destination finder, against
//! `workspace::example()`: `prod-fra` is suggested, `prod-ams` is already
//! mapped, `prod-lon` has a context but no workspace entry, a server shared
//! by `dev-fra` and `stage-fra` is ambiguous, and Argo CD's own cluster
//! goes to `core-fra`.

use super::{ClusterSecret, DestinationFindings, Field, Found, NamespaceRead, Read};

fn found(name: &str, contexts: &[&str]) -> Found {
    Found {
        secret: ClusterSecret {
            namespace: "argocd".into(),
            secret: format!("cluster-{name}"),
            server: Field::Text(format!("https://{name}.example.test:6443")),
            name: Field::Text(name.into()),
        },
        contexts: contexts.iter().map(|c| (*c).to_owned()).collect(),
    }
}

/// What a find of the example's Argo CD, on `core-fra`, comes back with.
pub fn destination_findings() -> DestinationFindings {
    DestinationFindings {
        entry: "core-fra".into(),
        read: Read::Done {
            namespaces: vec![(
                "argocd".into(),
                NamespaceRead::Found(vec![
                    found("prod-fra", &["prod-fra"]),
                    found("prod-ams", &["prod-ams"]),
                    found("prod-lon", &["prod-lon"]),
                    found("shared", &["dev-fra", "stage-fra"]),
                ]),
            )],
            skipped: Vec::new(),
            no_servers: None,
        },
    }
}
