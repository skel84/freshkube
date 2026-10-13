//! Overview: the cluster's cards and what needs attention, drawn from what
//! the shell hands it.
use super::shell::Connection;
use super::states::{self, ConfigAction, OnConfigAction, StaleTalos};
use super::switch::AgeLabel;
use crate::palette::palette;
use crate::presentation::attention::{Attention, Destination};
use crate::presentation::overview::{CardTarget, Overview};
use crate::ui::{self, dp};
use gpui_kit::component::tooltip::Tooltip;
use gpui_kit::prelude::*;
use gpui_kit::*;
use std::rc::Rc;
use std::sync::Arc;

pub(super) mod attention;
mod cards;

/// What Overview asks of the shell.
#[derive(Clone, Debug)]
pub(crate) enum OverviewEvent {
    /// A card's target: a page, a kind, a destination or System services.
    Card(CardTarget),
    /// An attention row's Open, Logs or Open node.
    Open(Destination),
    /// A state's button.
    Config(ConfigAction),
}

/// Where the Talos overview, the summary and the connection stand, as
/// Overview's states, banner and heading say. The shell hands it over
/// whenever it changes; an equal one redraws nothing.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct OverviewState {
    pub(crate) context: SharedString,
    pub(crate) config_error: Option<String>,
    pub(crate) kubernetes_only: bool,
    /// The Talos overview failed with nothing kept and isn't loading.
    pub(crate) unreachable: Option<String>,
    pub(crate) stale_talos: Option<StaleTalos>,
    /// The Talos overview or the summary is stale.
    pub(crate) stale: bool,
    /// Why the attention list may be out of date.
    pub(crate) attention_stale: Option<SharedString>,
    pub(crate) connection: Connection,
    /// The restored summary shows its age instead of the connection.
    pub(crate) last_known: bool,
}

pub(crate) struct OverviewPage {
    display: Arc<Overview>,
    attention: Arc<Attention>,
    state: OverviewState,
    /// Show all on the attention list.
    expanded: bool,
    age_label: Entity<AgeLabel>,
    on_config: OnConfigAction,
}

impl EventEmitter<OverviewEvent> for OverviewPage {}

impl OverviewPage {
    pub(super) fn new(age_label: Entity<AgeLabel>, cx: &mut Context<Self>) -> Self {
        let page = cx.weak_entity();
        let on_config: OnConfigAction = Rc::new(move |action, _, cx| {
            _ = page.update(cx, |_, cx| cx.emit(OverviewEvent::Config(action)));
        });
        Self {
            display: Default::default(),
            attention: Default::default(),
            state: Default::default(),
            expanded: false,
            age_label,
            on_config,
        }
    }

    /// The cards and heading texts, derived with the joined nodes.
    pub(super) fn set_display(&mut self, display: Arc<Overview>, cx: &mut Context<Self>) {
        self.display = display;
        cx.notify();
    }

    pub(super) fn set_attention(&mut self, attention: Arc<Attention>, cx: &mut Context<Self>) {
        self.attention = attention;
        cx.notify();
    }

    pub(super) fn set_state(&mut self, state: OverviewState, cx: &mut Context<Self>) {
        if self.state != state {
            self.state = state;
            cx.notify();
        }
    }

    /// Folds the attention list again, as a new context does.
    pub(super) fn collapse(&mut self, cx: &mut Context<Self>) {
        if self.expanded {
            self.expanded = false;
            cx.notify();
        }
    }

    /// The context as the title, with the connection state, any version
    /// drift, what the cluster runs and where its node list comes from in
    /// the meta line. The texts are derived with the cards.
    fn overview_header(&self, window: &mut Window, cx: &mut Context<Self>) -> Div {
        let p = palette(cx);
        let display = &self.display;
        let separator = || div().flex_none().px(dp(5.)).child("·");
        let drift = display.drift.clone().map(|drift| {
            let tip = display.drift_tip.clone();
            div()
                .id("version-drift")
                .test_support()
                .flex_none()
                .text_color(p.warn_ink)
                .aria_label(tip.clone())
                .tooltip(move |window, cx| Tooltip::new(tip.clone()).build(window, cx))
                .child(drift)
        });
        let subtitle = display.subtitle.clone();
        let roster = (!display.roster.is_empty()).then(|| {
            let tip = display.roster_tip.clone();
            div()
                .id("roster")
                .test_support()
                .flex_none()
                .aria_label(tip.clone())
                .tooltip(move |window, cx| Tooltip::new(tip.clone()).build(window, cx))
                .child(display.roster.clone())
        });
        // The header's switcher reads the same state, so the two agree.
        let (connection, ink) = match self.state.connection {
            _ if self.state.stale => ("Stale snapshot", Some(p.warn_ink)),
            Connection::Connected => ("Connected", None),
            Connection::Failed => ("Couldn't connect", Some(p.crit_ink)),
            Connection::Connecting => ("Connecting…", None),
        };
        let connection_part = if self.state.last_known {
            // The restored summary says how old it is, on its own timer.
            self.age_label.clone().into_any_element()
        } else {
            div()
                .id("overview-connection")
                .test_support()
                .flex_none()
                .when_some(ink, |this, ink| this.text_color(ink))
                .aria_label(connection)
                .child(connection)
                .into_any_element()
        };
        let mut meta = vec![connection_part];
        for part in [drift, roster].into_iter().flatten() {
            meta.push(separator().into_any_element());
            meta.push(part.into_any_element());
        }
        if !subtitle.is_empty() {
            meta.push(separator().into_any_element());
            meta.push(
                div()
                    .id("overview-runs")
                    .test_support()
                    .min_w_0()
                    .truncate()
                    .tooltip({
                        let subtitle = subtitle.clone();
                        move |window, cx| Tooltip::new(subtitle.clone()).build(window, cx)
                    })
                    .child(subtitle)
                    .into_any_element(),
            );
        }
        freshkube_ui::page::PageHeader::new("overview", self.state.context.clone())
            .meta(meta)
            .render(window, cx)
    }

    fn render_attention(&self, cx: &mut Context<Self>) -> AnyElement {
        let page = cx.weak_entity();
        let on_open: attention::OnOpen = Rc::new(move |destination, _, cx| {
            _ = page.update(cx, |_, cx| cx.emit(OverviewEvent::Open(destination)));
        });
        let page = cx.weak_entity();
        let on_show_all: attention::OnShowAll = Rc::new(move |_, cx| {
            _ = page.update(cx, |page, cx| {
                page.expanded = true;
                cx.notify();
            });
        });
        attention::render_attention(
            &self.attention,
            None,
            self.expanded,
            self.state.attention_stale.clone(),
            &on_open,
            Some(&on_show_all),
            cx,
        )
    }
}

impl Render for OverviewPage {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        super::probe::hit("overview");
        let state = &self.state;
        if let Some(error) = state.config_error.clone() {
            return states::config_error_state(error, state.kubernetes_only, &self.on_config, cx);
        }
        if let Some(error) = state.unreachable.clone() {
            return states::unreachable_state(&state.context, error, &self.on_config, cx);
        }
        let header = self.overview_header(window, cx);
        let cards = self.render_cards(window, cx);
        let warnings = self
            .display
            .warnings
            .iter()
            .map(|warning| ui::warning_banner(None, warning.clone(), None, cx))
            .collect::<Vec<_>>();
        let attention = self.render_attention(cx);
        // The shared frame, at its content's height inside the scrolling page.
        let body = freshkube_ui::page::padded("overview-frame")
            .h_auto()
            .children(states::stale_banner(
                self.state.stale_talos.as_ref(),
                &self.on_config,
                cx,
            ))
            .child(header)
            .children(warnings)
            .child(cards)
            .child(attention);
        states::page_scroll("overview-page")
            .test_support()
            .child(body)
            .into_any_element()
    }
}

impl super::Pilot {
    /// Where the Talos overview, the summary and the connection stand, for
    /// Overview.
    pub(super) fn overview_state(&self) -> OverviewState {
        let summary = &self.registry.active().kubernetes_summary;
        let unreachable = (self.kubernetes_only.is_none()
            && self.overview.data().is_none()
            && !self.overview.is_loading())
        .then(|| self.overview.error().cloned())
        .flatten();
        let attention_stale = if self.overview.is_stale() {
            self.overview.error().map(String::as_str)
        } else if summary.is_stale() {
            summary.error().map(String::as_str)
        } else {
            None
        };
        OverviewState {
            context: self.applied.context.clone().unwrap_or_default().into(),
            config_error: self.config_error.clone(),
            kubernetes_only: self.kubernetes_only.is_some(),
            unreachable,
            stale_talos: StaleTalos::of(&self.overview),
            stale: self.overview.is_stale() || summary.is_stale(),
            attention_stale: attention_stale.map(|error| error.to_owned().into()),
            connection: self.context_display.state,
            last_known: self.last_known_since().is_some(),
        }
    }

    /// Hands Overview where things stand; it redraws only for a change.
    pub(super) fn push_overview_state(&self, cx: &mut App) {
        let state = self.overview_state();
        self.overview_page
            .update(cx, |page, cx| page.set_state(state, cx));
    }

    /// What Overview asks of the shell.
    pub(super) fn overview_event(
        &mut self,
        event: &OverviewEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match event {
            OverviewEvent::Card(target) => self.open_card(target.clone(), window, cx),
            OverviewEvent::Open(destination) => {
                self.open_destination(destination.clone(), window, cx)
            }
            OverviewEvent::Config(action) => self.config_action(*action, window, cx),
        }
    }
}

/// What tests read and set of Overview through the shell.
#[cfg(test)]
impl super::Pilot {
    pub(super) fn overview_display(&self, cx: &App) -> Arc<Overview> {
        self.overview_page.read(cx).display.clone()
    }

    pub(super) fn attention_expanded(&self, cx: &App) -> bool {
        self.overview_page.read(cx).expanded
    }

    pub(super) fn expand_attention(&self, cx: &mut App) {
        self.overview_page.update(cx, |page, cx| {
            page.expanded = true;
            cx.notify();
        });
    }

    /// Hands the node pane and Overview this attention list.
    pub(super) fn set_attention(&mut self, attention: Attention, cx: &mut App) {
        self.attention = Arc::new(attention);
        let attention = self.attention.clone();
        self.overview_page
            .update(cx, |page, cx| page.set_attention(attention, cx));
    }
}

#[cfg(test)]
mod tests;
