//! Example data for the build namespace finder: every run of the example's
//! builds entry is in one namespace.
use super::{BuildFindings, BuildRead, NamespaceCounts};

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
