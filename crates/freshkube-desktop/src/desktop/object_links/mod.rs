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
            ResourceLink::Node(name, tab) => {
                self.unless_shell(window, cx, move |this, window, cx| {
                    this.resources
                        .update(cx, |resources, cx| resources.close_for_link(cx));
                    this.open_node_by_name(&name, tab, window, cx);
                })
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
                self.object_open_job = None;
                self.object_open_task = None;
                self.object_open_sequence = self.object_open_sequence.wrapping_add(1);
                let sequence = self.object_open_sequence;
                let epoch = self.epoch;
                let origin = self.resources.read(cx).detail_identity(cx).cloned();
                let (job, receiver) = backend::spawn_job(
                    &self.runtime,
                    Duration::from_secs(15),
                    "Resolving the owner's kind timed out".into(),
                    async move {
                        let client = source.access.client().await?;
                        freshkube_core::resources::resolve_owner_kind(&client, &api_version, &kind)
                            .await
                            .map_err(|error| error.to_string())
                    },
                );
                self.object_open_job = Some(job);
                self.object_open_task = Some(cx.spawn_in(window, async move |this, cx| {
                    let result = receiver
                        .await
                        .unwrap_or_else(|_| Err("The owner worker stopped".into()));
                    _ = this.update_in(cx, |this, window, cx| {
                        if this.epoch != epoch
                            || this.object_open_sequence != sequence
                            || this.resources.read(cx).detail_identity(cx) != origin.as_ref()
                        {
                            return;
                        }
                        this.object_open_job = None;
                        match result {
                            Ok(kind) => {
                                this.open_object(kind, object, resources::Tab::Overview, window, cx)
                            }
                            Err(error) => {
                                window.push_notification(format!("Can't open owner: {error}"), cx)
                            }
                        }
                    });
                }));
            }
        }
    }
    pub(super) fn push_node_rows(&self, cx: &mut Context<Self>) {
        let rows = self.node_workspace.rows.clone();
        self.resources
            .update(cx, |resources, cx| resources.set_node_rows(rows, cx));
    }
}

#[cfg(test)]
mod tests;
