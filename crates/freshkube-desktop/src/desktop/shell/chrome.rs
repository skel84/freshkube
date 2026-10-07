//! The chrome's parts as cached views beside the page: the header, the
//! rail and the column. A frame that only the page or the loading motion
//! asks for redraws the shell's frame around them and reuses each part as
//! it was drawn. The status bar stays with the shell: it reads the pages,
//! whose own redraws reach it as the shell's children.
//!
//! A part draws with `Pilot`'s own `render_*`, so its controls still act
//! on the shell. It draws again when the shell notifies, and the column
//! also when what it reads from the pages changes (`ChromeParts::watch`).
//! Never cache an ancestor of the pages instead: a cached view that draws
//! again draws every cached view inside it again.
use super::*;
use crate::desktop::probe;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(in crate::desktop) enum Part {
    Header,
    Rail,
    Column,
}

impl Part {
    fn probe(self) -> &'static str {
        match self {
            Part::Header => "chrome.header",
            Part::Rail => "chrome.rail",
            Part::Column => "chrome.column",
        }
    }
}

pub(in crate::desktop) struct Chrome {
    pilot: WeakEntity<Pilot>,
    part: Part,
    _subscriptions: Vec<Subscription>,
}

impl Chrome {
    pub(in crate::desktop) fn new(
        pilot: &Entity<Pilot>,
        part: Part,
        cx: &mut Context<Self>,
    ) -> Self {
        Self {
            pilot: pilot.downgrade(),
            part,
            _subscriptions: vec![cx.observe(pilot, |_, _, cx| cx.notify())],
        }
    }
}

impl Chrome {
    /// Draws again when `entity` notifies while the shell's area is one
    /// `shows`.
    fn watch<T: 'static>(
        &mut self,
        entity: &Entity<T>,
        shows: fn(Area) -> bool,
        cx: &mut Context<Self>,
    ) {
        self._subscriptions
            .push(cx.observe(entity, move |chrome, _, cx| {
                if chrome
                    .pilot
                    .upgrade()
                    .is_some_and(|pilot| shows(pilot.read(cx).area))
                {
                    cx.notify();
                }
            }));
    }
}

impl Render for Chrome {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        probe::hit(self.part.probe());
        let _span = crate::perf::span(self.part.probe());
        let part = self.part;
        // Parts lay out after the shell's render has returned, so the shell
        // is free to update here.
        self.pilot
            .update(cx, |pilot, cx| match part {
                Part::Header => pilot.render_header(window, cx),
                Part::Rail => pilot.render_rail(cx),
                Part::Column => pilot
                    .render_column(window, cx)
                    .unwrap_or_else(|| div().into_any_element()),
            })
            .unwrap_or_else(|_| div().into_any_element())
    }
}

/// The shell's cached parts, made once with the shell.
pub(in crate::desktop) struct ChromeParts {
    pub(in crate::desktop) header: Entity<Chrome>,
    pub(in crate::desktop) rail: Entity<Chrome>,
    pub(in crate::desktop) column: Entity<Chrome>,
}

impl ChromeParts {
    pub(in crate::desktop) fn new(cx: &mut Context<Pilot>) -> Self {
        let pilot = cx.entity();
        let mut part = |part| cx.new(|cx| Chrome::new(&pilot, part, cx));
        Self {
            header: part(Part::Header),
            rail: part(Part::Rail),
            column: part(Part::Column),
        }
    }
}

impl ChromeParts {
    /// What the column reads from the pages: each one's area, and the
    /// namespace the Workloads and Observability columns pick. Resources
    /// notifies with every watch event, so only a new namespace draws.
    /// Custom Resources needs no watch: the shell observes it.
    pub(in crate::desktop) fn watch(&self, pilot: &Pilot, cx: &mut App) {
        let resources = pilot.resources.clone();
        let services = pilot.system_services.clone();
        let (monitoring, observability) = (pilot.monitoring.clone(), pilot.observability.clone());
        self.column.update(cx, |column, cx| {
            column.watch(&services, |area| area == Area::ControlPlane, cx);
            column.watch(&monitoring, |area| area == Area::Monitoring, cx);
            column.watch(&observability, |area| area == Area::Observability, cx);
            let mut shown = resources.read(cx).namespace().map(str::to_owned);
            column
                ._subscriptions
                .push(cx.observe(&resources, move |_, resources, cx| {
                    let namespace = resources.read(cx).namespace().map(str::to_owned);
                    if namespace != shown {
                        shown = namespace;
                        cx.notify();
                    }
                }));
        });
    }
}
