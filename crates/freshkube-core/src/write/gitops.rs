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

    /// The sentence an edit of the whole object adds: it names the source to
    /// change instead when the object says which it is.
    pub fn edit_warning(self, object: &Value) -> String {
        let source = self.source(object);
        match (self, source) {
            (Self::ArgoCd, source) => format!(
                " Argo CD manages this object{}: change the source it syncs from instead. With self-heal on it will put this edit back; with it off the object shows OutOfSync.",
                source
                    .map(|name| format!(" (application {name})"))
                    .unwrap_or_default()
            ),
            (Self::Flux, source) => format!(
                " Flux manages this object{}: change its source instead, or its next reconcile will put this edit back.",
                source.map(|name| format!(" ({name})")).unwrap_or_default()
            ),
        }
    }

    /// What the object names as its source: Argo CD's application, from the
    /// tracking id (`<app>:<group>/<Kind>:<namespace>/<name>`) or the
    /// instance stamp, or Flux's Kustomization or HelmRelease, from its labels.
    fn source(self, object: &Value) -> Option<String> {
        let meta = object.get("metadata")?;
        let stamp = |field: &str, key: &str| meta.get(field)?.get(key)?.as_str();
        let named = |value: &str| (!value.is_empty()).then(|| value.to_owned());
        match self {
            Self::ArgoCd => stamp("annotations", "argocd.argoproj.io/tracking-id")
                .and_then(|id| id.split(':').next())
                .and_then(named)
                .or_else(|| {
                    ["labels", "annotations"]
                        .iter()
                        .find_map(|field| stamp(field, "argocd.argoproj.io/instance"))
                        .and_then(named)
                }),
            Self::Flux => [
                ("kustomize.toolkit.fluxcd.io", "Kustomization"),
                ("helm.toolkit.fluxcd.io", "HelmRelease"),
            ]
            .iter()
            .find_map(|(domain, kind)| {
                let name = stamp("labels", &format!("{domain}/name"))?;
                let namespace = stamp("labels", &format!("{domain}/namespace"))?;
                Some(format!("{kind} {namespace}/{name}"))
            }),
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

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn an_edit_names_the_source_the_object_says_it_has() {
        let argo = json!({"metadata": {"annotations": {
            "argocd.argoproj.io/tracking-id": "ledger:apps/Deployment:payments/ledger"}}});
        let warning = GitOps::of(&argo).unwrap().edit_warning(&argo);
        assert!(warning.contains("application ledger"), "{warning}");
        let by_label = json!({"metadata": {"labels": {"argocd.argoproj.io/instance": "checkout"}}});
        assert!(
            GitOps::ArgoCd
                .edit_warning(&by_label)
                .contains("application checkout")
        );
        let flux = json!({"metadata": {"labels": {
            "kustomize.toolkit.fluxcd.io/name": "apps", "kustomize.toolkit.fluxcd.io/namespace": "flux-system"}}});
        let warning = GitOps::of(&flux).unwrap().edit_warning(&flux);
        assert!(
            warning.contains("Kustomization flux-system/apps"),
            "{warning}"
        );
        let helm = json!({"metadata": {"labels": {
            "helm.toolkit.fluxcd.io/name": "podinfo", "helm.toolkit.fluxcd.io/namespace": "web"}}});
        assert!(
            GitOps::Flux
                .edit_warning(&helm)
                .contains("HelmRelease web/podinfo")
        );
        // An object that names no source still gets the warning.
        let bare = json!({"metadata": {"managedFields": [{"manager": "argocd-controller"}]}});
        let warning = GitOps::of(&bare).unwrap().edit_warning(&bare);
        assert!(
            warning.contains("Argo CD manages this object:"),
            "{warning}"
        );
    }
}
