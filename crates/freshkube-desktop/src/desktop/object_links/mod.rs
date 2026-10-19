//! Relationship navigation uses the same object entry point as cards and search.
use super::switch::LinkWork;
use super::*;
use crate::resources::ResourceLink;
use gpui_kit::component::WindowExt;

impl Pilot {
    /// A Deployment's revision on Observability's Deployments, which finds
    /// it in what it reads for the open connection.
    pub(super) fn open_revision(
        &mut self,
        link: crate::observability::RevisionLink,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.pages
            .observability
            .update(cx, |page, cx| page.open_revision(link, cx));
        self.navigate_from_keyboard(Page::Observability, window, cx);
    }

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
                .pages
                .dock
                .update(cx, |dock, cx| dock.open_logs(request, window, cx)),
            ResourceLink::Shell(request) => self
                .pages
                .dock
                .update(cx, |dock, cx| dock.open_shell(request, window, cx)),
            ResourceLink::Node(name, tab) => {
                self.pages
                    .resources
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
                let work = LinkWork::Owner {
                    api_version: api_version.clone(),
                    kind: kind.clone(),
                };
                if self.divert_link(&object, work, window, cx) {
                    return;
                }
                if self.mode().is_example() {
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
                let Some(source) = self.source().kube else {
                    return;
                };
                self.resolve_owner_remote(source, api_version, kind, object, window, cx);
            }
        }
    }

    /// Reads which kind an owner's API version and kind name, then opens the
    /// owner. A link made in another cluster is sent to it or refused first.
    pub(super) fn resolve_owner_remote(
        &mut self,
        source: resources::ClusterSource,
        api_version: String,
        kind: String,
        object: resources::model::ObjectRef,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        // Also here, for a caller that has not routed it: this read goes to
        // the cluster that is open.
        let work = LinkWork::Owner {
            api_version: api_version.clone(),
            kind: kind.clone(),
        };
        if self.divert_link(&object, work, window, cx) {
            return;
        }
        self.cancel_object_open();
        let origin = self.pages.resources.read(cx).detail_identity(cx).cloned();
        self.start_object_open(
            "Resolving the owner's kind timed out",
            async move {
                let client = source
                    .access
                    .client()
                    .await
                    .map_err(|failure| failure.message)?;
                freshkube_core::resources::resolve_owner_kind(&client, &api_version, &kind)
                    .await
                    .map_err(|error| error.to_string())
            },
            move |this, cx| this.pages.resources.read(cx).detail_identity(cx) != origin.as_ref(),
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
    /// epoch and this loader request still hold and `superseded` (the
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
        let epoch = self.epoch;
        let request = self.object_open.start(Some(epoch));
        let (job, receiver) = backend::spawn_job(
            &self.runtime,
            Duration::from_secs(15),
            timeout_message.into(),
            work,
        );
        let task = cx.spawn_in(window, async move |this, cx| {
            let result = receiver
                .await
                .unwrap_or_else(|_| Err("The read stopped".into()));
            _ = this.update_in(cx, |this, window, cx| {
                if this.epoch != epoch
                    || superseded(this, cx)
                    || !this.object_open.land(&request, Ok(()), cx)
                {
                    return;
                }
                done(this, result, window, cx);
            });
            // A superseded origin intentionally keeps the read pending until
            // replacement, as before. Tests can still wait for its delivery.
            #[cfg(test)]
            probe::hit("object-open-delivered");
        });
        self.object_open.run(Some(job), task);
    }
}

#[cfg(test)]
mod tests;
