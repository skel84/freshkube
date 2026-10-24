//! The start strip: the pull request (compact), the Build and the
//! Freight as three cards, each join showing its key in a pill and its
//! word under it. One row on a wide page; stacked under 640 dp of it.
//! Derived with the change; drawing only reads it.
use freshkube_ui::page;
use freshkube_ui::palette::palette;
use freshkube_ui::tooltip::FollowTooltip as _;
use gpui_kit::component::{h_flex, v_flex};

use super::*;
use crate::ui::{self, dp};

/// Below this page width, in dp, the strip stacks into one column.
pub const STACKS_BELOW: f32 = 640.;

/// One card of the strip.
#[derive(Clone, Debug, PartialEq)]
pub struct Card {
    /// The id part: `pr`, `build`, `freight`.
    pub id: &'static str,
    /// The hop a press selects.
    pub select: SharedString,
    /// Every hop the card stands for, so its selection shows.
    pub hops: Vec<SharedString>,
    pub title: SharedString,
    pub detail: SharedString,
    pub tone: Tone,
}

/// The join into a card from the one before it.
#[derive(Clone, Debug, PartialEq)]
pub struct Join {
    /// The hop the join runs into, which pressing it selects with its
    /// link in the Inspector.
    pub hop: Option<SharedString>,
    pub track: kit::Track,
    pub tone: Tone,
    /// The key it joined on, short; `no key` when it joined on nothing.
    pub pill: SharedString,
    /// Confirmed, Claimed, or the cause of an Unknown.
    pub word: SharedString,
    pub tooltip: SharedString,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Strip {
    pub cards: Vec<Card>,
    /// `joins[i]` runs into `cards[i + 1]`.
    pub joins: Vec<Join>,
}

pub fn derive(change: &Change, rows: &[HopRow]) -> Strip {
    let hops: Vec<(&Hop, &HopRow)> = change.hops.iter().zip(rows).collect();
    let build_group = change
        .groups
        .iter()
        .position(|group| group.phase == Phase::Build);
    let pr = hops
        .iter()
        .find(|(hop, _)| kind_of(hop) == Some("Pull request"));
    let build: Vec<_> = hops
        .iter()
        .filter(|(hop, _)| Some(hop.group) == build_group && kind_of(hop) != Some("Pull request"))
        .collect();
    let freight = hops.iter().find(|(hop, _)| hop.key == "freight");
    let mut cards = Vec::new();
    let mut ends: Vec<Option<&(&Hop, &HopRow)>> = Vec::new();
    if let Some(pr @ (_, row)) = pr {
        cards.push(Card {
            id: "pr",
            select: row.key.clone(),
            hops: vec![row.key.clone()],
            title: row.name.clone(),
            detail: row.detail.clone(),
            tone: row.tone,
        });
        ends.push(Some(pr));
    }
    if let Some(last) = build
        .iter()
        .rev()
        .find(|(hop, _)| kind_of(hop) == Some("PipelineRun"))
        .or(build.last())
    {
        let tone = build
            .iter()
            .map(|(_, row)| status(row.tone))
            .max_by_key(|tone| rank(*tone))
            .unwrap_or(Tone::Good);
        // What the Build can't confirm is said on its card: the hop and its
        // cause (`Not found · Policy acme-prod`), not the run it shows.
        let unconfirmed = build.iter().find_map(|(_, row)| {
            row.cause
                .as_ref()
                .filter(|cause| cause.is_problem())
                .map(|cause| (row.name.clone(), cause))
        });
        cards.push(Card {
            id: "build",
            select: last.1.key.clone(),
            hops: build.iter().map(|(_, row)| row.key.clone()).collect(),
            title: "Build".into(),
            detail: match unconfirmed {
                Some((name, cause)) => format!("{} · {name}", cause.word()).into(),
                None => last.1.name.clone(),
            },
            tone,
        });
        ends.push(build.first().copied());
    }
    if let Some(freight @ (_, row)) = freight {
        cards.push(Card {
            id: "freight",
            select: row.key.clone(),
            hops: vec![row.key.clone()],
            title: row.name.clone(),
            detail: row.detail.clone(),
            tone: row.tone,
        });
        ends.push(Some(freight));
    }
    let joins = ends.iter().skip(1).map(|end| join(end.copied())).collect();
    Strip { cards, joins }
}

/// The kind of the hop's object, for a hop that has one.
fn kind_of(hop: &Hop) -> Option<&str> {
    match &hop.shows {
        Shows::Hop(detail) => Some(detail.kind.as_str()),
        Shows::Stage(_) => None,
    }
}

/// The join into the card whose first hop is `end`.
fn join(end: Option<(&Hop, &HopRow)>) -> Join {
    let Some((hop, row)) = end else {
        return Join {
            hop: None,
            track: kit::Track::Dotted,
            tone: Tone::Unknown,
            pill: "no key".into(),
            word: "Unknown".into(),
            tooltip: "Nothing read joins these two.".into(),
        };
    };
    let link = match &hop.shows {
        Shows::Hop(detail) => detail.link.as_ref(),
        Shows::Stage(_) => None,
    };
    let confidence = link
        .map(|link| link.confidence)
        .or(row.link)
        .unwrap_or(Confidence::Unknown);
    let cause = row.cause.as_ref();
    let waiting = confidence == Confidence::Unknown && cause.is_some_and(|c| !c.is_problem());
    Join {
        hop: Some(row.key.clone()),
        track: match confidence {
            Confidence::Confirmed => kit::Track::Solid,
            Confidence::Claimed => kit::Track::Dashed,
            Confidence::Unknown => kit::Track::Dotted,
        },
        tone: match (confidence, waiting) {
            (Confidence::Confirmed, _) => Tone::Good,
            (Confidence::Claimed, _) => Tone::Info,
            (Confidence::Unknown, true) => Tone::Waiting,
            (Confidence::Unknown, false) => Tone::Unknown,
        },
        pill: link
            .and_then(|link| link.key.as_deref())
            .map(short_key)
            .unwrap_or_else(|| "no key".into())
            .into(),
        word: cause_word(confidence, cause).into(),
        tooltip: link
            .map(|link| {
                format!(
                    "{} by {}. {} reports {}. This hop reports {}. {}",
                    cause_word(confidence, cause),
                    link.by,
                    link.before,
                    link.before_says,
                    link.here_says,
                    link.why
                )
            })
            .unwrap_or_else(|| "Unknown: a side can't be read".into())
            .into(),
    }
}

/// A key as the pill shows it: a commit or a digest cut to seven.
pub fn short_key(key: &str) -> String {
    let digest = key.starts_with("sha256:");
    let key = key.strip_prefix("sha256:").unwrap_or(key);
    if digest || key.len() >= 7 && key.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        key.chars().take(7).collect()
    } else {
        key.to_owned()
    }
}

impl ChangePage {
    pub(super) fn render_strip(&self, window: &Window, cx: &mut Context<Self>) -> AnyElement {
        let stacked = self.lead_width(window, cx) < STACKS_BELOW;
        let p = palette(cx);
        let mut strip = if stacked { v_flex() } else { h_flex() }
            .id("change-strip")
            .test_support()
            .flex_none()
            .w_full()
            .min_w_0()
            .overflow_hidden()
            .gap(dp(0.))
            .px(dp(page::PANE_PADDING))
            .py(dp(page::PANE_PADDING_Y))
            .when(!stacked, |this| this.items_start());
        for (ix, card) in self.strip.cards.iter().enumerate() {
            if let Some(join) = ix.checked_sub(1).and_then(|ix| self.strip.joins.get(ix)) {
                strip = strip.child(self.render_join(card.id, join, stacked, cx));
            }
            strip = strip.child(self.render_card(card, stacked, cx));
        }
        strip.border_b_1().border_color(p.line).into_any_element()
    }

    fn render_card(&self, card: &Card, stacked: bool, cx: &mut Context<Self>) -> AnyElement {
        let p = palette(cx);
        let selected = self
            .selected
            .as_ref()
            .is_some_and(|key| card.hops.contains(key));
        let select = card.select.clone();
        page::card(cx)
            .id(SharedString::from(format!("change-strip-{}", card.id)))
            .test_support()
            .role(Role::Button)
            .aria_label(format!("{}: {}", card.title, card.detail))
            .flex_1()
            .min_w(dp(120.))
            .px(dp(10.))
            .py(dp(6.))
            .gap(dp(2.))
            .cursor_pointer()
            .when(selected, |this| {
                this.border_color(p.accent).bg(p.accent_soft)
            })
            .when(stacked, |this| this.w_full())
            .child(
                h_flex()
                    .gap(dp(6.))
                    .items_center()
                    .child(div().flex_none().children(ui::status_glyph(card.tone, cx)))
                    .child(
                        div()
                            .min_w_0()
                            .truncate()
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_size(dp(12.5))
                            .text_color(p.ink)
                            .child(card.title.clone()),
                    ),
            )
            .child(
                div()
                    .min_w_0()
                    .truncate()
                    .text_size(dp(11.))
                    .text_color(p.muted)
                    .child(card.detail.clone()),
            )
            .follow_tooltip(format!("{}: {}", card.title, card.detail))
            .on_click(cx.listener(move |this, _, window, cx| {
                this.select(&select, window, cx);
                this.focus(window, cx);
            }))
            .into_any_element()
    }

    /// A join: the line, the key in a pill on it and its word under it.
    fn render_join(
        &self,
        into: &'static str,
        join: &Join,
        stacked: bool,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let p = palette(cx);
        let id = |part: &str| SharedString::from(format!("change-join-{into}-{part}"));
        let line = kit::connector(id("line"), join.track, join.tone, stacked, cx);
        let pill = ui::tag(
            match join.tone {
                Tone::Good => Tone::Outline,
                tone => tone,
            },
            None,
            join.pill.clone(),
            cx,
        )
        .font_family(ui::MONO_FONT);
        let word = div()
            .id(id("word"))
            .test_support()
            .text_size(dp(10.5))
            .text_color(p.muted)
            .child(join.word.clone());
        let words = if stacked {
            h_flex().gap(dp(6.)).items_center()
        } else {
            v_flex().items_center()
        }
        .id(id("key"))
        .test_support()
        .aria_label(format!("{}: {}", join.pill, join.word))
        .flex_none()
        .child(pill)
        .child(word)
        .follow_tooltip(join.tooltip.clone())
        .when_some(join.hop.clone(), |this, hop| {
            this.cursor_pointer()
                .on_click(cx.listener(move |this, _, window, cx| {
                    this.select_link(&hop, window, cx);
                    this.focus(window, cx);
                }))
        });
        if stacked {
            // Down the card's left, a short line and the key beside it.
            h_flex()
                .h(dp(26.))
                .items_center()
                .gap(dp(8.))
                .pl(dp(18.))
                .child(div().w(dp(1.)).h(dp(18.)).child(line))
                .child(words)
                .into_any_element()
        } else {
            v_flex()
                .flex_none()
                .w(dp(104.))
                .items_center()
                .child(div().h(dp(kit::LINE)).w_full().child(line.w_full()))
                .child(words)
                .into_any_element()
        }
    }
}
