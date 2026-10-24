//! Which GitOps tool manages an object, read from what it stamps on it. Every
//! write to a managed object says so in its dialog, because the tool's
//! self-heal may put the change back.

use serde_json::Value;

/// Which GitOps tool manages an object, read from what it stamps on it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GitOps {
    ArgoCd,
    Flux,
}

impl GitOps {
    /// The sentence added to a write to a managed object: self-heal undoes it.
    pub fn warning(self) -> &'static str {
        match self {
            Self::ArgoCd => {
                " Argo CD manages this object: with self-heal on it will put the change back; with it off the object shows OutOfSync."
            }
            Self::Flux => {
                " Flux manages this object: its next reconcile will put the change back unless the source changes too."
            }
        }
    }

    /// The tool that manages `object`, when it says so.
    pub fn of(object: &Value) -> Option<Self> {
        let meta = object.get("metadata")?;
        let has = |field: &str, prefix: &str| {
            meta.get(field)
                .and_then(Value::as_object)
                .is_some_and(|map| map.keys().any(|key| key.starts_with(prefix)))
        };
        let managed_by = |manager: &str| {
            meta.get("managedFields")
                .and_then(Value::as_array)
                .is_some_and(|fields| {
                    fields
                        .iter()
                        .any(|entry| entry["manager"].as_str() == Some(manager))
                })
        };
        // Argo CD stamps an annotation when it tracks by annotation, and a
        // label when it tracks by label; its controller shows as a field
        // manager either way.
        if has("annotations", "argocd.argoproj.io/") || managed_by("argocd-controller") {
            Some(Self::ArgoCd)
        } else if managed_by("kustomize-controller")
            || managed_by("helm-controller")
            || has("labels", "kustomize.toolkit.fluxcd.io/")
            || has("labels", "helm.toolkit.fluxcd.io/")
        {
            Some(Self::Flux)
        } else {
            None
        }
    }
}
