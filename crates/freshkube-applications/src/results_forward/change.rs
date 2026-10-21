//! The change page borrows the shared session. Keep its subscription here,
//! keyed by the page and removed on release, without changing its layout or
//! storing a forward guard on the page. Observer callbacks hold weak entities.
use super::*;
use crate::results_forward::{Binding, Target};
use freshkube_delivery::entry::pool::ResultsRoute;
use gpui_kit::component::{Disableable, Sizable, button::Button};
use std::collections::HashMap;

#[derive(Default)]
struct Bindings(HashMap<EntityId, Entry>);
impl Global for Bindings {}
struct Entry {
    binding: Binding,
    _release: Subscription,
}

impl ChangePage {
    pub(super) fn configure_results(&mut self, cx: &mut Context<Self>) {
        if cx.try_global::<Bindings>().is_none() {
            cx.set_global(Bindings::default());
        }
        let id = cx.entity_id();
        let mut entry = cx
            .global_mut::<Bindings>()
            .0
            .remove(&id)
            .unwrap_or_else(|| Entry {
                binding: Binding::default(),
                _release: cx.on_release(move |_, cx| {
                    if cx.try_global::<Bindings>().is_some() {
                        cx.global_mut::<Bindings>().0.remove(&id);
                    }
                }),
            });
        let changed = if let Fetch::Live {
            runtime,
            access,
            place,
            ..
        } = &self.fetch
        {
            let source = crate::resources::ClusterSource {
                id: place.cluster.clone(),
                context: place.label.clone(),
                access: access.clone(),
            };
            let target = Target::of(&place.sources, place.mapping.open.as_deref(), &source);
            entry
                .binding
                .configure(target, &source, runtime.clone(), Self::results_changed, cx)
        } else {
            entry.binding = Binding::default();
            false
        };
        cx.global_mut::<Bindings>().0.insert(id, entry);
        if changed {
            self.loader.reset();
            self.stale = None;
            self.hide_outcomes();
            self.show(self.fetch.unread());
        }
    }

    fn results_changed(&mut self, cx: &mut Context<Self>) {
        let id = cx.entity_id();
        let Some(mut entry) = cx.global_mut::<Bindings>().0.remove(&id) else {
            return;
        };
        entry.binding.sync(cx);
        cx.global_mut::<Bindings>().0.insert(id, entry);
        self.loader.reset();
        self.stale = None;
        self.hide_outcomes();
        self.show(self.fetch.unread());
        if self.visible {
            self.read(None, cx);
        }
        cx.notify();
    }

    pub(super) fn results_route(&self, cx: &Context<Self>) -> ResultsRoute {
        cx.try_global::<Bindings>()
            .and_then(|bindings| bindings.0.get(&cx.entity_id()))
            .map_or(ResultsRoute::Address, |entry| entry.binding.route())
    }

    pub(super) fn results_identity(&self, cx: &Context<Self>) -> String {
        let identity = self.fetch.identity();
        let epoch = cx
            .try_global::<Bindings>()
            .and_then(|bindings| bindings.0.get(&cx.entity_id()))
            .and_then(|entry| entry.binding.epoch());
        match epoch {
            Some(epoch) => format!("{identity}/forward/{}/{}", epoch.id, epoch.start),
            None => identity,
        }
    }

    pub(crate) fn results_action(&self, cx: &mut Context<Self>) -> Option<Button> {
        let hop = self.change.hop(self.selected.as_ref()?)?;
        let Shows::Hop(detail) = &hop.shows else {
            return None;
        };
        if detail.kind != "Builds" && detail.kind != "PipelineRun" {
            return None;
        }
        let session = cx
            .try_global::<Bindings>()?
            .0
            .get(&cx.entity_id())?
            .binding
            .session
            .clone()?;
        let session = session.read(cx);
        Some(
            Button::new("change-forward-results")
                .outline()
                .xsmall()
                .label(session.label.clone())
                .tooltip(session.words.clone())
                .disabled(!session.enabled)
                .on_click(cx.listener(|this, _, _, cx| this.forward_results(cx))),
        )
    }

    fn forward_results(&mut self, cx: &mut Context<Self>) {
        let session = cx
            .try_global::<Bindings>()
            .and_then(|bindings| bindings.0.get(&cx.entity_id()))
            .and_then(|entry| entry.binding.session.clone());
        let Some(session) = session else {
            return;
        };
        if let Some((entry, link)) = session.update(cx, |session, cx| session.act(cx)) {
            cx.emit(ChangeEvent::Switch(entry, Box::new(link)));
        }
    }
}
