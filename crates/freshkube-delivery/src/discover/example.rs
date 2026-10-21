//! Example data for the destination finder, against
//! `workspace::example()`: `prod-fra` is suggested, `prod-ams` is already
//! mapped, `prod-lon` has a context but no workspace entry, a server shared
//! by `dev-fra` and `stage-fra` is ambiguous, and Argo CD's own cluster
//! goes to `core-fra`.

use super::{
    BuildFindings, BuildRead, ClusterSecret, DestinationFindings, Field, Found, NamespaceCounts,
    NamespaceRead, Read,
};

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

/// What a find of the example's builds, on `cicd-fra`, comes back with.
pub fn build_findings() -> BuildFindings {
    BuildFindings {
        entry: "cicd-fra".into(),
        read: BuildRead::Done(NamespaceCounts {
            counts: [("ci-shared".to_owned(), 23)].into(),
            capped: false,
        }),
    }
}

/// Runs in three namespaces, for the item that lists them and suggests none.
pub fn several_build_findings() -> BuildFindings {
    BuildFindings {
        entry: "cicd-fra".into(),
        read: BuildRead::Done(NamespaceCounts {
            counts: [
                ("acme".to_owned(), 14),
                ("shop".to_owned(), 6),
                ("ci-shared".to_owned(), 3),
            ]
            .into(),
            capped: false,
        }),
    }
}
