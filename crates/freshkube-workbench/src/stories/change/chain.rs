//! Invented delivery joins on the shared canvas, without a delivery or
//! application dependency. Each control isolates one Chain state.
use freshkube_ui::graph::{
    self, Cursor, EdgeStyle, GraphCanvas, GraphEdge, GraphNode, GraphSource, GraphState, GraphText,
};
use freshkube_ui::inspector::{self, Inspector, InspectorSplit};
use freshkube_ui::page::{self, PageHeader};
use freshkube_ui::ui::{self, Tone, dp};
use gpui_kit::component::{Sizable, button::Button, h_flex};
use gpui_kit::prelude::*;
use gpui_kit::{AnyElement, AnyView, App, Context, Render, SharedString, Window, div};

const CASES: [(&str, &str); 6] = [
    ("confirmed", "Confirmed"),
    ("claimed", "Claimed"),
    ("unknown", "Unknown"),
    ("refused", "Refused"),
    ("missing", "Missing policy"),
    ("stages", "Two Stages"),
];

pub fn build(_: &mut Window, cx: &mut App) -> AnyView {
    graph::bind_keys(cx);
    cx.new(ChainStory::new).into()
}

pub struct ChainStory {
    graph: GraphState<usize>,
    split: InspectorSplit,
    case: usize,
    cursor: Option<Cursor<usize>>,
    heading: SharedString,
    detail: SharedString,
}

impl ChainStory {
    fn new(cx: &mut Context<Self>) -> Self {
        let graph = GraphState::new(
            "change-chain",
            GraphText {
                nouns: ("hops", "links"),
                title: "Link".into(),
                problems: "".into(),
                caption: "".into(),
            },
        )
        .unpaged()
        .with_keys(cx);
        let mut story = Self {
            graph,
            split: InspectorSplit::new("workbench-chain", cx),
            case: 0,
            cursor: None,
            heading: "Choose a hop or link".into(),
            detail: "Arrows walk the Chain".into(),
        };
        story.place();
        story
    }

    fn place(&mut self) {
        let mut names = vec![
            "Pull request",
            "Commit",
            "PipelineRun",
            "Image",
            "Freight",
            "dev",
        ];
        let mut ends = vec![(0, 1), (1, 2), (2, 3), (3, 4), (4, 5)];
        if self.case == 4 {
            names.push("Policy");
            ends.push((2, 6));
        }
        if self.case == 5 {
            names.push("prod");
            ends.push((4, 6));
        }
        let nodes = names
            .iter()
            .enumerate()
            .map(|(ix, name)| GraphNode {
                key: ix,
                id: format!("change-chain-card-{ix}").into(),
                label: (*name).into(),
                detail: if ix == 6 && self.case == 4 {
                    "Not found"
                } else if ix == 5 && self.case == 3 {
                    "pods on cluster-a refused"
                } else {
                    "Read from cluster-a"
                }
                .into(),
                tone: Some(
                    if (ix == 6 && self.case == 4) || (ix == 5 && self.case >= 2) {
                        Tone::Unknown
                    } else {
                        Tone::Good
                    },
                ),
                tooltip: (*name).into(),
                ghost: ix == 6 && self.case == 4,
            })
            .collect();
        let edges = ends
            .into_iter()
            .enumerate()
            .map(|(ix, (from, to))| {
                let style = match self.case {
                    1 => EdgeStyle::Dashed,
                    2 | 3 => EdgeStyle::Dotted,
                    4 if to == 6 => EdgeStyle::Dotted,
                    _ => EdgeStyle::Solid,
                };
                GraphEdge {
                    from,
                    to,
                    marker_id: format!("change-chain-link-{to}").into(),
                    row_id: format!("change-chain-row-{to}").into(),
                    tone: None,
                    width: 1.1,
                    label: "Join".into(),
                    tooltip: format!("{} → {} · {}", names[from], names[to], CASES[self.case].1)
                        .into(),
                    style,
                    pill: Some(
                        if style == EdgeStyle::Dotted {
                            "no key"
                        } else if ix < 2 {
                            "3f9a1c2"
                        } else {
                            "9a8b7c6"
                        }
                        .into(),
                    ),
                }
            })
            .collect();
        self.graph.set(nodes, edges);
        self.cursor = None;
        self.heading = "Choose a hop or link".into();
        self.detail = "Arrows walk the Chain".into();
    }
}

impl GraphSource for ChainStory {
    fn word_details(&self) -> bool {
        true
    }
    type Key = usize;
    fn graph(&self) -> &GraphState<usize> {
        &self.graph
    }
    fn graph_mut(&mut self) -> &mut GraphState<usize> {
        &mut self.graph
    }
    fn graph_split(&self) -> &InspectorSplit {
        &self.split
    }
    fn cursor(&self) -> Option<Cursor<usize>> {
        self.cursor.clone()
    }
    fn open(&mut self, key: &usize, window: &mut Window, cx: &mut Context<Self>) {
        self.step_to(Cursor::Node(*key), window, cx);
        cx.notify();
    }
    fn step_to(&mut self, cursor: Cursor<usize>, _: &mut Window, _: &mut Context<Self>) {
        match cursor {
            Cursor::Node(key) => {
                self.graph.clear_selection();
                let node = &self.graph.nodes()[key];
                self.heading = node.label.clone();
                self.detail = node.detail.clone();
            }
            Cursor::Link(from, to) => {
                self.graph.select(&from, &to);
                let edge = self.graph.selected_edge().unwrap();
                self.heading = "Link".into();
                self.detail = edge.tooltip.clone();
            }
        }
        self.cursor = Some(cursor);
    }
    fn inspector_head(&self, _: Option<&GraphEdge<usize>>, _: &mut Context<Self>) -> AnyElement {
        div().child(self.detail.clone()).into_any_element()
    }
    fn empty(&self, _: &mut Context<Self>) -> AnyElement {
        div().child("No hops").into_any_element()
    }
}

impl Render for ChainStory {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let controls =
            h_flex()
                .gap(dp(2.))
                .children(CASES.iter().enumerate().map(|(case, (id, label))| {
                    ui::segment(
                        Button::new(SharedString::from(format!("change-chain-case-{id}"))),
                        self.case == case,
                        cx,
                    )
                    .small()
                    .label(*label)
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.case = case;
                        this.place();
                        cx.notify();
                    }))
                }));
        let header = PageHeader::new("change-chain", "Change chain")
            .secondary(controls)
            .render(window, cx);
        let canvas = GraphCanvas::render(self, window, cx);
        let details = Inspector::new("change-chain-detail")
            .heading(div().child(self.heading.clone()))
            .child(div().child(self.detail.clone()))
            .render(cx)
            .into_any_element();
        let body = inspector::split(
            "change-chain-split",
            &self.split,
            page::page_width(window) >= inspector::SPLIT_WIDTH,
            canvas,
            Some(details),
            window,
        );
        page::page("change-chain-page")
            .child(page::toolbar(cx).child(header))
            .child(div().flex_1().min_h_0().child(body))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui_kit::{AppContext, TestAppContext};
    #[gpui_kit::test]
    fn every_case_keeps_its_hops_and_the_missing_policy_is_a_branch(cx: &mut TestAppContext) {
        cx.update(gpui_kit::init);
        let story = cx.new(ChainStory::new);
        for case in 0..CASES.len() {
            story.update(cx, |story, _| {
                story.case = case;
                story.place();
                assert_eq!(story.graph.pages(), 1);
                assert_eq!(story.graph.nodes().len(), if case >= 4 { 7 } else { 6 });
                if case == 4 {
                    assert!(story.graph.nodes()[6].ghost);
                    let edge = story.graph.edges().last().unwrap();
                    assert_eq!((edge.from, edge.to), (2, 6));
                    assert_eq!(edge.style, EdgeStyle::Dotted);
                }
                if case == 5 {
                    assert_eq!(
                        story.graph.edges().last().map(|e| (e.from, e.to)),
                        Some((4, 6))
                    );
                }
            });
        }
    }
}
