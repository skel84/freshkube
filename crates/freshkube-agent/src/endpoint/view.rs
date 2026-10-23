//! Where the window would go to show a read, as the closed `ViewState` a
//! link carries. It stays in the app: it is never put on the wire, and what
//! the agent writes never becomes a navigation.

use freshkube_core::agent_reads::{Place, Tab};
use freshkube_nav::{
    KindKey, LinkPage, Name, Namespace, ObjectUid, ViewObject, ViewState, ViewTab,
};

/// The Resources view of a read's place. `None` when a part of it does not
/// fit the closed types (a kind key or a name a link could not carry), or the
/// parts do not check as one view.
pub(super) fn view_of(place: &Place) -> Option<ViewState> {
    let mut view = ViewState::page(LinkPage::Resources);
    view.kind = Some(KindKey::parse(&place.kind).ok()?);
    view.ns = match &place.namespace {
        Some(namespace) => Some(Namespace::parse(namespace).ok()?),
        None => None,
    };
    view.object = match &place.name {
        Some(name) => Some(ViewObject {
            name: Name::parse(name).ok()?,
            uid: place
                .uid
                .as_deref()
                .and_then(|uid| ObjectUid::parse(uid).ok()),
        }),
        None => None,
    };
    view.tab = match place.tab {
        Tab::Details => ViewTab::Details,
        Tab::Events => ViewTab::Events,
        Tab::Logs => ViewTab::Logs,
    };
    view.check().ok()?;
    Some(view)
}
