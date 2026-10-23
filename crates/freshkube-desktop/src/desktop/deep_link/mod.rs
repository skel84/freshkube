//! Deep links (#873): Copy link, and opening a link.
//!
//! A link names a place with identities only (`freshkube_nav::ViewState`),
//! so opening one only navigates: it never opens a dialog, starts a shell or
//! a forward, reaches Operations or maintenance, or moves focus onto a
//! button. The object is read with the reader's own access; when the link
//! can't be followed the pane's slot says why (`LinkOutcome`).
use super::*;
use crate::debug_options::LinkShape;
use crate::presentation::attention::Destination;
use freshkube_nav::{
    KindKey, Link, LinkError, LinkOutcome, LinkPage, Name, Namespace, ObjectUid, ViewObject,
    ViewState, ViewTab,
};
use gpui_kit::ClipboardItem;

mod cluster;
#[cfg(test)]
mod cluster_tests;
mod describe;
mod example;
mod identity;
mod picker;

pub(super) use describe::describe;

/// What the shell keeps for links.
pub(super) struct State {
    /// Debug: forces one outcome on every example link, or reshapes how the
    /// example clusters identify themselves.
    pub(in crate::desktop) shape: Option<LinkShape>,
    /// What is known of each cluster's identity, and the dialog that asks.
    identities: cluster::Identities,
    /// Links from the operating system or the command line, waiting for the
    /// window to have a cluster to read.
    waiting: Vec<String>,
    /// The view being applied is an Agent tab's move, not the person's: a
    /// log read then shows its pod in the tab that following owns. Set per
    /// `apply_view`, so an open still being read is the latest one's.
    followed: bool,
}

/// The most links held while the window has no cluster yet.
const WAITING_LIMIT: usize = 4;

impl State {
    /// Forces one outcome on every example link, as `FRESHKUBE_LINKS` does.
    #[cfg(test)]
    pub(in crate::desktop) fn force(&mut self, shape: Option<LinkShape>) {
        self.shape = shape;
    }

    pub(super) fn new(shape: Option<LinkShape>, preferences: Option<&std::path::Path>) -> Self {
        Self {
            shape,
            identities: cluster::Identities::open(preferences),
            waiting: Vec::new(),
            followed: false,
        }
    }
}

/// The page a link names, for the pages that are plain places.
pub(super) fn page_of(page: LinkPage) -> Page {
    match page {
        LinkPage::Overview => Page::Overview,
        LinkPage::Nodes => Page::Nodes,
        LinkPage::Health => Page::Health,
        LinkPage::Databases => Page::Databases,
        LinkPage::Resources => Page::Resources,
        LinkPage::Etcd => Page::Etcd,
        LinkPage::SystemServices => Page::SystemServices,
        LinkPage::Security => Page::Security,
        LinkPage::Lifecycle => Page::Lifecycle,
        LinkPage::Monitoring => Page::Monitoring,
        LinkPage::Observability => Page::Observability,
        LinkPage::Applications => Page::Applications,
    }
}

/// The link page a shell page is, if it is a plain place: not Settings or
/// Operations.
pub(super) fn link_page(page: Page) -> Option<LinkPage> {
    LinkPage::ALL
        .into_iter()
        .find(|candidate| page_of(*candidate) == page)
}

/// What a kind key names here.
enum Lookup {
    Known(ResourceKind),
    /// This cluster doesn't serve it.
    Missing,
    /// Not known yet: discovery has to be asked.
    Discover,
}

/// The answer of a read a link waits on: a thing, or why not.
enum Looked<T> {
    Found(T),
    Outcome(LinkOutcome),
}

fn outcome_of(failure: &freshkube_core::resources::Failure) -> Option<LinkOutcome> {
    use freshkube_core::resources::FailureKind;
    match failure.kind {
        FailureKind::Forbidden | FailureKind::Unauthorized => Some(LinkOutcome::CantRead),
        FailureKind::NotFound => Some(LinkOutcome::NotFound),
        _ => None,
    }
}

impl Pilot {
    /// Where the window is, as identities, or `None` on a page a link can't
    /// name (Settings, Operations). Reads the shell's state; builds nothing
    /// in `render`.
    pub(super) fn current_view(&self, cx: &App) -> Option<ViewState> {
        let mut view = ViewState::page(link_page(self.navigation.page)?);
        match view.page {
            LinkPage::Resources => self.resources_view(&mut view, cx),
            LinkPage::Nodes => {
                let nodes = self.pages.node_workspace.read(cx);
                let name = nodes
                    .selected
                    .as_ref()
                    .filter(|_| nodes.open)
                    .and_then(|key| key.kubernetes.as_ref().or(key.talos.as_ref()))
                    .and_then(|name| Name::parse(name).ok());
                if name.is_some() {
                    view.node = name;
                    view.node_tab = nodes.tab;
                }
            }
            _ => {}
        }
        view.check().ok()?;
        Some(view)
    }

    fn resources_view(&self, view: &mut ViewState, cx: &App) {
        let kind = &self.navigation.resource_kind;
        let Ok(key) = KindKey::parse(&kind.key()) else {
            return;
        };
        view.kind = Some(key);
        let resources = self.pages.resources.read(cx);
        if kind.namespaced {
            view.ns = resources
                .namespace()
                .and_then(|namespace| Namespace::parse(namespace).ok());
        }
        let Some(identity) = resources.detail_identity(cx) else {
            return;
        };
        // A name a link can't carry (a ClusterRole's colon) falls back to
        // the kind's list; the toast says what the link opens.
        let Ok(name) = Name::parse(&identity.name) else {
            return;
        };
        if kind.namespaced {
            let Ok(namespace) = Namespace::parse(&identity.namespace) else {
                return;
            };
            view.ns = Some(namespace);
        }
        view.object = Some(ViewObject {
            name,
            uid: ObjectUid::parse(&identity.uid).ok(),
        });
        view.tab = ViewTab::from_tab(resources.detail_tab(cx));
    }

    /// Copy link: the current view's address, to the clipboard, with a toast
    /// that says what it opens and that it opens with the reader's access.
    pub(super) fn copy_link(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let view = self.current_view(cx);
        self.copy_view(view, window, cx);
    }

    /// Copy link on a row of Needs attention: the link to where Open goes.
    pub(super) fn copy_link_to(
        &mut self,
        destination: &Destination,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let view = Self::view_of(destination);
        self.copy_view(view, window, cx);
    }

    /// The view an attention row's destination names.
    fn view_of(destination: &Destination) -> Option<ViewState> {
        let view = match destination {
            Destination::Page(page) => ViewState::page(link_page(*page)?),
            Destination::Node(key, tab) => {
                let name = key.kubernetes.as_ref().or(key.talos.as_ref())?;
                ViewState {
                    node: Some(Name::parse(name).ok()?),
                    node_tab: *tab,
                    ..ViewState::page(LinkPage::Nodes)
                }
            }
            Destination::Service { node, logs, .. } => ViewState {
                node: Some(Name::parse(node).ok()?),
                node_tab: if *logs {
                    freshkube_nav::NodeTab::Logs
                } else {
                    freshkube_nav::NodeTab::Services
                },
                ..ViewState::page(LinkPage::Nodes)
            },
            Destination::Object(key, object, tab) => {
                let kind = freshkube_core::resources::builtin(key)?;
                ViewState {
                    kind: Some(KindKey::parse(key).ok()?),
                    ns: kind
                        .namespaced
                        .then(|| Namespace::parse(&object.namespace).ok())
                        .flatten(),
                    object: Some(ViewObject {
                        name: Name::parse(&object.name).ok()?,
                        uid: ObjectUid::parse(&object.uid).ok(),
                    }),
                    tab: ViewTab::from_tab(*tab),
                    ..ViewState::page(LinkPage::Resources)
                }
            }
            // Alerts has no place in a link yet: Copy link says so.
            Destination::Alerts | Destination::CopyLink(_) => return None,
        };
        view.check().ok()?;
        Some(view)
    }

    fn copy_view(&mut self, view: Option<ViewState>, window: &mut Window, cx: &mut Context<Self>) {
        let Some(view) = view else {
            self.tell("This page can’t be linked.".into(), window, cx);
            return;
        };
        let link = self.link_for(view.clone(), cx);
        let tied = link.cluster != freshkube_nav::ClusterRef::default();
        cx.write_to_clipboard(ClipboardItem::new_string(link.to_url()));
        let tail = if tied {
            ""
        } else {
            " It isn’t tied to a cluster yet, so it opens in whichever cluster the reader has open."
        };
        self.tell(
            format!(
                "Link copied. It opens {}, read with the reader’s own access.{tail}",
                describe(&view)
            ),
            window,
            cx,
        );
    }

    /// Opens a link the operating system or the command line gave: at once
    /// when the window has a cluster to read (example data always has), else
    /// once it does.
    pub(in crate::desktop) fn open_external_link(
        &mut self,
        text: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let ready =
            (self.mode().is_example() || self.source().kube.is_some()) && !self.identifying();
        if ready {
            self.open_link_text(&text, window, cx);
        } else if self.deep_link.waiting.len() < WAITING_LIMIT {
            self.deep_link.waiting.push(text);
        }
    }

    /// Opens the links that waited for a cluster, once the window has one.
    pub(in crate::desktop) fn open_waiting_links(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.deep_link.waiting.is_empty() || self.source().kube.is_none() || self.identifying() {
            return;
        }
        for text in std::mem::take(&mut self.deep_link.waiting) {
            self.open_link_text(&text, window, cx);
        }
    }

    /// Opens a link given as text: a click, a paste or the command line. A
    /// link that isn't valid, or was made by a newer Freshkube, opens
    /// nothing and says so.
    pub(super) fn open_link_text(
        &mut self,
        text: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match Link::parse(text) {
            Ok(link) => self.open_link(link, window, cx),
            Err(LinkError::NewerVersion(_)) => self.tell(
                "This link was made by a newer Freshkube. Update to open it.".into(),
                window,
                cx,
            ),
            Err(error) => self.tell(format!("Can’t open this link: {error}."), window, cx),
        }
    }

    /// Opens a view in the cluster that is open. Takes only the closed type,
    /// never a string, and only navigates.
    pub(super) fn apply_view(
        &mut self,
        view: &ViewState,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.deep_link.followed = false;
        self.apply_view_now(view, window, cx);
    }

    /// `apply_view` for a move an Agent tab made, which only differs in
    /// where a log read shows its pod.
    pub(super) fn apply_followed_view(
        &mut self,
        view: &ViewState,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.deep_link.followed = true;
        self.apply_view_now(view, window, cx);
    }

    fn apply_view_now(&mut self, view: &ViewState, window: &mut Window, cx: &mut Context<Self>) {
        self.target.cancel_object_open();
        match view.page {
            LinkPage::Resources => self.apply_resources(view, window, cx),
            LinkPage::Nodes => self.apply_nodes(view, window, cx),
            page => self.navigate_from_keyboard(page_of(page), window, cx),
        }
    }

    fn apply_nodes(&mut self, view: &ViewState, window: &mut Window, cx: &mut Context<Self>) {
        let Some(name) = &view.node else {
            self.navigate_from_keyboard(Page::Nodes, window, cx);
            return;
        };
        let listed = self.pages.node_workspace.read(cx).rows.iter().any(|row| {
            row.key.talos.as_deref() == Some(name.as_str())
                || row.key.kubernetes.as_deref() == Some(name.as_str())
        });
        if listed {
            self.open_node_by_name(name.as_str(), view.node_tab, window, cx);
        } else {
            self.navigate_from_keyboard(Page::Nodes, window, cx);
            self.tell(
                format!(
                    "Can’t open node {}: {}",
                    name.as_str(),
                    LinkOutcome::NotFound.why()
                ),
                window,
                cx,
            );
        }
    }

    fn lookup_kind(&self, key: &KindKey, cx: &App) -> Lookup {
        if self.deep_link.shape == Some(LinkShape::NotServed) && self.mode().is_example() {
            return Lookup::Missing;
        }
        if let Some(kind) = freshkube_core::resources::builtin(key.as_str()) {
            return Lookup::Known(kind);
        }
        if self.mode().is_example() {
            return resources::example::kind(key.as_str()).map_or(Lookup::Missing, Lookup::Known);
        }
        match self.pages.custom.read(cx).kind(key.as_str()) {
            Some(kind) => Lookup::Known(kind),
            None => Lookup::Discover,
        }
    }

    fn apply_resources(&mut self, view: &ViewState, window: &mut Window, cx: &mut Context<Self>) {
        let Some(key) = view.kind.clone() else {
            self.navigate_from_keyboard(Page::Resources, window, cx);
            return;
        };
        match self.lookup_kind(&key, cx) {
            Lookup::Known(kind) => self.apply_kind(kind, view, window, cx),
            Lookup::Missing => {
                self.show_outcome(LinkOutcome::NotServed, key.as_str(), None, window, cx)
            }
            Lookup::Discover => self.discover_kind(key, view.clone(), window, cx),
        }
    }

    /// Asks discovery which kind a key names, then opens the view.
    fn discover_kind(
        &mut self,
        key: KindKey,
        view: ViewState,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(source) = self.source().kube else {
            self.show_outcome(LinkOutcome::NotServed, key.as_str(), None, window, cx);
            return;
        };
        let connection = source.id.clone();
        let asked = key.as_str().to_owned();
        self.start_object_open(
            "Looking up the kind timed out",
            async move {
                let client = source
                    .access
                    .client()
                    .await
                    .map_err(|failure| failure.message)?;
                match freshkube_core::resources::resolve_kind_key(&client, &asked).await {
                    Ok(Some(kind)) => Ok(Looked::Found(kind)),
                    Ok(None) => Ok(Looked::Outcome(LinkOutcome::NotServed)),
                    Err(failure) => match outcome_of(&failure) {
                        Some(LinkOutcome::NotFound) | None => Err(failure.message),
                        Some(outcome) => Ok(Looked::Outcome(outcome)),
                    },
                }
            },
            move |this, _| {
                this.source()
                    .kube
                    .is_none_or(|source| source.id != connection)
            },
            move |this, result, window, cx| match result {
                Ok(Looked::Found(kind)) => this.apply_kind(kind, &view, window, cx),
                Ok(Looked::Outcome(outcome)) => {
                    this.show_outcome(outcome, key.as_str(), None, window, cx)
                }
                Err(error) => this.tell(format!("Can’t open this link: {error}"), window, cx),
            },
            window,
            cx,
        );
    }

    /// The kind is served: show its list in the namespace the link names,
    /// then the object, if it names one.
    fn apply_kind(
        &mut self,
        kind: ResourceKind,
        view: &ViewState,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.open_kind(kind.clone(), window, cx);
        // A tab that comes back lists where it was, not in its object's
        // namespace.
        let list_ns = self.tabs.list_override.take();
        if kind.namespaced {
            let namespace =
                list_ns.unwrap_or_else(|| view.ns.as_ref().map(|ns| ns.as_str().to_owned()));
            self.pages.resources.update(cx, |resources, cx| {
                resources.set_namespace(namespace, window, cx)
            });
        }
        if let Some(object) = &view.object {
            self.open_view_object(kind, view, object, window, cx);
        }
    }

    /// Reads the object with the reader's access, then opens it, or says why
    /// not in the pane's slot.
    fn open_view_object(
        &mut self,
        kind: ResourceKind,
        view: &ViewState,
        object: &ViewObject,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let namespace = if kind.namespaced {
            view.ns
                .as_ref()
                .map(|ns| ns.as_str().to_owned())
                .unwrap_or_default()
        } else {
            String::new()
        };
        let subject = subject(&kind, &namespace, object.name.as_str());
        let Some(source) = self.source().kube else {
            return;
        };
        if kind.namespaced && namespace.is_empty() {
            self.show_outcome(LinkOutcome::NotFound, &subject, None, window, cx);
            return;
        }
        let identity = resources::model::ResourceIdentity {
            connection: source.id.clone(),
            resource: kind.key(),
            namespace: namespace.clone(),
            name: object.name.as_str().to_owned(),
            uid: String::new(),
        };
        let wanted = object.uid.clone();
        let tab = view.tab;
        if self.mode().is_example() {
            let found = self.example_object(&source, &kind, identity);
            self.finish_object(found, wanted, kind, tab, subject, window, cx);
            return;
        }
        let connection = source.id.clone();
        let read_kind = kind.clone();
        let (read_namespace, name) = (namespace, object.name.as_str().to_owned());
        self.start_object_open(
            "Reading the object timed out",
            async move {
                let client = source
                    .access
                    .client()
                    .await
                    .map_err(|failure| failure.message)?;
                let found = freshkube_core::resources::get_metadata(
                    &client,
                    &read_kind,
                    (!read_namespace.is_empty()).then_some(read_namespace.as_str()),
                    &name,
                )
                .await;
                match found {
                    Ok(metadata) => match metadata.uid {
                        Some(uid) => Ok(Looked::Found(uid)),
                        None => Err("The API returned no UID".to_owned()),
                    },
                    Err(failure) => match outcome_of(&failure) {
                        Some(outcome) => Ok(Looked::Outcome(outcome)),
                        None => Err(failure.message),
                    },
                }
            },
            move |this, _| {
                this.source()
                    .kube
                    .is_none_or(|source| source.id != connection)
            },
            {
                let mut identity = identity;
                let subject = subject.clone();
                move |this, result, window, cx| match result {
                    Ok(Looked::Found(uid)) => {
                        identity.uid = uid;
                        this.finish_object(Ok(identity), wanted, kind, tab, subject, window, cx);
                    }
                    Ok(Looked::Outcome(outcome)) => {
                        this.finish_object(Err(outcome), wanted, kind, tab, subject, window, cx)
                    }
                    Err(error) => this.tell(format!("Can’t open {subject}: {error}"), window, cx),
                }
            },
            window,
            cx,
        );
    }

    /// An example object by name from the example rows, as the live read
    /// would answer it; a debug shape forces one outcome.
    fn example_object(
        &self,
        source: &resources::ClusterSource,
        kind: &ResourceKind,
        mut identity: resources::model::ResourceIdentity,
    ) -> Result<resources::model::ResourceIdentity, LinkOutcome> {
        match self.deep_link.shape {
            Some(LinkShape::Refused) => return Err(LinkOutcome::CantRead),
            Some(LinkShape::NotFound) => return Err(LinkOutcome::NotFound),
            _ => {}
        }
        let rows = resources::example::read(
            &source.context,
            &kind.key(),
            (!identity.namespace.is_empty()).then_some(identity.namespace.as_str()),
            chrono::Utc::now().timestamp(),
        )
        .map(|(_, rows)| rows)
        .unwrap_or_default();
        let row = rows
            .into_iter()
            .find(|row| {
                row.identity.name == identity.name && row.identity.namespace == identity.namespace
            })
            .ok_or(LinkOutcome::NotFound)?;
        identity.uid = row.identity.uid;
        Ok(identity)
    }

    /// Opens the object once it is read, or shows what the read said.
    #[allow(clippy::too_many_arguments)]
    fn finish_object(
        &mut self,
        found: Result<resources::model::ResourceIdentity, LinkOutcome>,
        wanted: Option<ObjectUid>,
        kind: ResourceKind,
        tab: ViewTab,
        subject: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let identity = match found {
            Ok(identity) => identity,
            Err(outcome) => return self.show_outcome(outcome, &subject, None, window, cx),
        };
        let replaced = match &wanted {
            Some(uid) => uid.as_str() != identity.uid,
            None => false,
        } || self.deep_link.shape == Some(LinkShape::Replaced)
            && self.mode().is_example();
        if replaced {
            self.show_outcome(LinkOutcome::Replaced, &subject, Some(identity), window, cx);
            return;
        }
        // Logs are the dock's, and the pane opens on Details beside them.
        let pane = match tab {
            ViewTab::Logs => resources::Tab::Overview,
            other => other.tab(),
        };
        self.pages.resources.update(cx, |resources, cx| {
            resources.open_identity_on(identity.clone(), pane, window, cx)
        });
        if tab == ViewTab::Logs {
            let request = resources::LogsRequest {
                target: resources::detail::DetailTarget { identity, kind },
                at: None,
            };
            if self.deep_link.followed {
                self.pages
                    .dock
                    .update(cx, |dock, cx| dock.open_followed_logs(request, window, cx));
            } else {
                self.resource_link(resources::ResourceLink::Logs(request), window, cx);
            }
        }
    }

    /// Puts `outcome` in the Resources pane's slot, over the list.
    fn show_outcome(
        &mut self,
        outcome: LinkOutcome,
        subject: &str,
        current: Option<resources::model::ResourceIdentity>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.navigation.page != Page::Resources {
            self.navigate_from_keyboard(Page::Resources, window, cx);
        }
        let card = resources::LinkOutcomeCard {
            outcome,
            subject: subject.to_owned(),
            current,
        };
        self.pages
            .resources
            .update(cx, |resources, cx| resources.show_link_outcome(card, cx));
    }
}

/// `deployments.apps payments/checkout`: what a card says the link named.
fn subject(kind: &ResourceKind, namespace: &str, name: &str) -> String {
    if namespace.is_empty() {
        format!("{} {name}", kind.key())
    } else {
        format!("{} {namespace}/{name}", kind.key())
    }
}

#[cfg(test)]
mod tests;
