//! Relationship navigation uses the same object entry point as cards and search.
use super::*;
use crate::resources::ResourceLink;
use gpui_kit::component::WindowExt;

impl Pilot {
    pub(super) fn resource_link(
        &mut self,
        intent: ResourceLink,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match intent {
            ResourceLink::Object(kind, object, tab) => {
                self.open_object(kind, object, tab, window, cx)
            }
            ResourceLink::Logs(request) => self
                .dock
                .update(cx, |dock, cx| dock.open_logs(request, window, cx)),
            ResourceLink::Shell(request) => self
                .dock
                .update(cx, |dock, cx| dock.open_shell(request, window, cx)),
            ResourceLink::Node(name, tab) => {
                self.resources
                    .update(cx, |resources, cx| resources.close_for_link(cx));
                self.open_node_by_name(&name, tab, window, cx);
            }
            ResourceLink::Owner {
                api_version,
                kind,
                object,
            } => {
                if let Some(kind) = freshkube_core::resources::builtin_by_gvk(&api_version, &kind) {
                    self.open_object(kind, object, resources::Tab::Overview, window, cx);
                    return;
                }
                if self.fixture {
                    let found = resources::example::custom_groups()
                        .into_iter()
                        .filter_map(|group| resources::example::group_kinds(&group.name).ok())
                        .flat_map(|group| group.kinds)
                        .find(|resource| {
                            resource.kind == kind && resource.api_version() == api_version
                        });
                    if let Some(kind) = found {
                        self.open_object(kind, object, resources::Tab::Overview, window, cx);
                    } else {
                        window.push_notification(
                            format!("Example data does not serve {api_version} {kind}"),
                            cx,
                        );
                    }
                    return;
                }
                let Some(source) = self.kube_source() else {
                    return;
                };
                self.resolve_owner_remote(source, api_version, kind, object, window, cx);
            }
        }
    }

    /// Reads which kind an owner's API version and kind name, then opens the
    /// owner. A link made in another cluster is refused first: this read
    /// would otherwise go to the cluster that is open.
    pub(super) fn resolve_owner_remote(
        &mut self,
        source: resources::KubeSource,
        api_version: String,
        kind: String,
        object: resources::model::ObjectRef,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.refuse_foreign_link(&object, window, cx) {
            return;
        }
        self.cancel_object_open();
        let origin = self.resources.read(cx).detail_identity(cx).cloned();
        self.start_object_open(
            "Resolving the owner's kind timed out",
            async move {
                let client = source.access.client().await?;
                freshkube_core::resources::resolve_owner_kind(&client, &api_version, &kind)
                    .await
                    .map_err(|error| error.to_string())
            },
            move |this, cx| this.resources.read(cx).detail_identity(cx) != origin.as_ref(),
            move |this, result, window, cx| match result {
                Ok(kind) => this.open_object(kind, object, resources::Tab::Overview, window, cx),
                Err(error) => window.push_notification(format!("Can't open owner: {error}"), cx),
            },
            window,
            cx,
        );
    }

    /// Starts the one read an object link waits on, replacing none: callers
    /// `cancel_object_open` first. Its answer is applied only if the shell's
    /// epoch and this request's sequence still hold and `superseded` (the
    /// caller's own check, for example that another object opened) says no.
    pub(super) fn start_object_open<T: Send + 'static>(
        &mut self,
        timeout_message: &str,
        work: impl Future<Output = Result<T, String>> + Send + 'static,
        superseded: impl FnOnce(&Self, &App) -> bool + 'static,
        done: impl FnOnce(&mut Self, Result<T, String>, &mut Window, &mut Context<Self>) + 'static,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let sequence = self.object_open_sequence;
        let epoch = self.epoch;
        let (job, receiver) = backend::spawn_job(
            &self.runtime,
            Duration::from_secs(15),
            timeout_message.into(),
            work,
        );
        self.object_open_job = Some(job);
        self.object_open_task = Some(cx.spawn_in(window, async move |this, cx| {
            let result = receiver
                .await
                .unwrap_or_else(|_| Err("The read stopped".into()));
            _ = this.update_in(cx, |this, window, cx| {
                if this.epoch != epoch
                    || this.object_open_sequence != sequence
                    || superseded(this, cx)
                {
                    return;
                }
                this.object_open_job = None;
                done(this, result, window, cx);
            });
        }));
    }

    pub(super) fn push_node_rows(&self, cx: &mut Context<Self>) {
        let rows = self.node_workspace.rows.clone();
        self.resources
            .update(cx, |resources, cx| resources.set_node_rows(rows, cx));
    }
}

#[cfg(test)]
mod tests;
