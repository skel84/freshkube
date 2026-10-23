//! The page follows the agent's reads, on example data: the places arrive as
//! the endpoint's `ReadEvent`s do, and the follower's holds step on the
//! executor's clock.

use std::time::Duration;

use freshkube_agent::endpoint::{Outcome as ReadOutcome, ReadEvent};
use freshkube_agent::{ReadId, SessionId};
use freshkube_core::agent_reads::Code;
use freshkube_nav::{KindKey, LinkPage, Name, Namespace, ViewObject, ViewState, ViewTab};
use freshkube_probe::probe;
use gpui_kit::component::WindowExt as _;
use gpui_kit::test::TestWindowExt;
use gpui_kit::{AppContext, ScrollDelta, TestAppContext, point, px};

use super::follow::HOLD;
use super::tests::{Tab, open, tour};
use crate::desktop::Page;
use crate::desktop::dock::TabKind;
use crate::resources::{example, live};

/// A Resources view of `kind`, in `namespace`, of one object when named.
fn view(kind: &str, namespace: Option<&str>, name: Option<&str>, tab: ViewTab) -> ViewState {
    let mut view = ViewState::page(LinkPage::Resources);
    view.kind = Some(KindKey::parse(kind).unwrap());
    view.ns = namespace.map(|ns| Namespace::parse(ns).unwrap());
    view.object = name.map(|name| ViewObject {
        name: Name::parse(name).unwrap(),
        uid: None,
    });
    view.tab = tab;
    view
}

impl Tab {
    fn entry(&self, cx: &mut TestAppContext) -> String {
        cx.update(|cx| self.view.read(cx).own.as_ref().unwrap().entry.clone())
    }

    /// Hands the follower read `number`, as the endpoint's event would.
    fn feed(
        &self,
        number: u32,
        cluster: Option<&str>,
        view: Option<ViewState>,
        outcome: ReadOutcome,
        cx: &mut TestAppContext,
    ) {
        let entry = self.entry(cx);
        let event = ReadEvent {
            session: SessionId::new(),
            read: ReadId::numbered(number),
            tool: "get_object",
            cluster: cluster.map(str::to_owned).unwrap_or(entry),
            view,
            outcome,
        };
        self.view.update(cx, |view, cx| {
            view.trail.read(&event, &view.own.as_ref().unwrap().entry);
            view.follow_read(&event, cx)
        });
        cx.run_until_parked();
    }

    fn read(&self, number: u32, place: ViewState, cx: &mut TestAppContext) {
        self.feed(
            number,
            None,
            Some(place),
            ReadOutcome::Answered { cut: None },
            cx,
        );
    }

    /// Lets a hold end.
    fn wait_out_hold(&self, cx: &mut TestAppContext) {
        cx.executor().advance_clock(HOLD);
        cx.run_until_parked();
    }

    fn shown(&self, cx: &mut TestAppContext) -> Option<ViewState> {
        cx.run_until_parked();
        cx.read(|cx| self.pilot.read(cx).current_view(cx))
    }

    fn following(&self, cx: &mut TestAppContext) -> bool {
        cx.read(|cx| self.view.read(cx).is_following())
    }

    fn log_tabs(&self, cx: &mut TestAppContext) -> Vec<String> {
        cx.read(|cx| {
            self.dock
                .read(cx)
                .tabs
                .iter()
                .filter(|tab| matches!(tab.kind, TabKind::Pod(_) | TabKind::Workload(_)))
                .map(|tab| tab.title.to_string())
                .collect()
        })
    }

    fn follow_tab(&self, cx: &mut TestAppContext) -> Option<u64> {
        cx.read(|cx| self.dock.read(cx).follow_tab())
    }
}

/// What the example cluster has in payments.
struct Places {
    namespace: String,
    deployment: String,
    pods: Vec<String>,
}

fn places(tab: &Tab, cx: &mut TestAppContext) -> Places {
    let context = cx.update(|cx| tab.view.read(cx).own.as_ref().unwrap().context.clone());
    let now = live::now();
    let (_, deployments) = example::read(&context, "deployments.apps", None, now).unwrap();
    let api = deployments
        .iter()
        .find(|row| row.identity.name == "api")
        .unwrap_or(&deployments[0]);
    let namespace = api.identity.namespace.clone();
    let (_, pods) = example::read(&context, "pods", None, now).unwrap();
    Places {
        deployment: api.identity.name.clone(),
        pods: pods
            .iter()
            .filter(|row| row.identity.namespace == namespace)
            .map(|row| row.identity.name.clone())
            .collect(),
        namespace,
    }
}

fn kind_of(view: &ViewState) -> &str {
    view.kind.as_ref().map(|kind| kind.as_str()).unwrap_or("")
}

fn object_of(view: &ViewState) -> &str {
    view.object
        .as_ref()
        .map(|object| object.name.as_str())
        .unwrap_or("")
}

#[gpui_kit::test]
fn the_page_follows_through_apply_view(cx: &mut TestAppContext) {
    let tab = open(cx, "ready", 1280., 880.);
    let at = places(&tab, cx);
    let pod = at.pods[0].clone();

    // The list.
    tab.read(
        1,
        view("pods", Some(&at.namespace), None, ViewTab::Details),
        cx,
    );
    let shown = tab.shown(cx).unwrap();
    assert_eq!(shown.page, LinkPage::Resources);
    assert_eq!(kind_of(&shown), "pods");
    assert_eq!(shown.ns.as_ref().unwrap().as_str(), at.namespace);
    assert!(shown.object.is_none());
    // The frame and the strip are drawn.
    assert!(tab.present("follow-frame", cx));
    assert!(tab.present("follow-strip", cx));
    let strip = tab.label("follow-place", cx).unwrap();
    assert_eq!(
        strip,
        format!("Following agent · pods · {} · 1 of 1", at.namespace)
    );

    // The object.
    tab.wait_out_hold(cx);
    tab.read(
        2,
        view(
            "deployments.apps",
            Some(&at.namespace),
            Some(&at.deployment),
            ViewTab::Details,
        ),
        cx,
    );
    let shown = tab.shown(cx).unwrap();
    assert_eq!(kind_of(&shown), "deployments.apps");
    assert_eq!(object_of(&shown), at.deployment);

    // A pod's log, in the dock tab that follow owns.
    tab.wait_out_hold(cx);
    tab.read(
        3,
        view("pods", Some(&at.namespace), Some(&pod), ViewTab::Logs),
        cx,
    );
    let shown = tab.shown(cx).unwrap();
    assert_eq!(kind_of(&shown), "pods");
    assert_eq!(object_of(&shown), pod);
    assert!(tab.follow_tab(cx).is_some());
    assert_eq!(tab.log_tabs(cx), [format!("Pod {pod}")]);

    // Its Events.
    tab.wait_out_hold(cx);
    tab.read(
        4,
        view(
            "deployments.apps",
            Some(&at.namespace),
            Some(&at.deployment),
            ViewTab::Events,
        ),
        cx,
    );
    let shown = tab.shown(cx).unwrap();
    assert_eq!(shown.tab, ViewTab::Events);
    assert_eq!(object_of(&shown), at.deployment);
    assert!(tab.label("follow-place", cx).unwrap().ends_with("4 of 4"));
    assert!(tab.following(cx));
}

#[gpui_kit::test]
fn a_view_waits_out_its_hold_before_the_next_place_shows(cx: &mut TestAppContext) {
    let tab = open(cx, "ready", 1280., 880.);
    let at = places(&tab, cx);
    tab.read(
        1,
        view("pods", Some(&at.namespace), None, ViewTab::Details),
        cx,
    );
    tab.read(
        2,
        view("services", Some(&at.namespace), None, ViewTab::Details),
        cx,
    );
    tab.read(
        3,
        view("configmaps", Some(&at.namespace), None, ViewTab::Details),
        cx,
    );
    // 0.4 s, 0.8 s: the first still shows.
    cx.executor().advance_clock(Duration::from_millis(800));
    cx.run_until_parked();
    assert_eq!(kind_of(&tab.shown(cx).unwrap()), "pods");
    // After 1.5 s the latest shows and the second was skipped.
    cx.executor().advance_clock(Duration::from_millis(700));
    cx.run_until_parked();
    assert_eq!(kind_of(&tab.shown(cx).unwrap()), "configmaps");
}

#[gpui_kit::test]
fn refused_and_viewless_reads_never_move_the_page(cx: &mut TestAppContext) {
    let tab = open(cx, "ready", 1280., 880.);
    let before = tab.shown(cx);
    let at = places(&tab, cx);
    let place = view("pods", Some(&at.namespace), None, ViewTab::Details);
    tab.feed(
        1,
        None,
        Some(place),
        ReadOutcome::Refused(Code::Refused),
        cx,
    );
    tab.feed(2, None, None, ReadOutcome::Answered { cut: None }, cx);
    tab.wait_out_hold(cx);
    assert_eq!(tab.shown(cx), before);
    assert!(!tab.present("follow-strip", cx));
}

#[gpui_kit::test]
fn a_read_on_another_cluster_never_moves_the_page_and_warns(cx: &mut TestAppContext) {
    let tab = open(cx, "ready", 1280., 880.);
    let at = places(&tab, cx);
    let before = tab.shown(cx);
    let context = cx.read(|cx| tab.pilot.read(cx).applied.context.clone());
    let own = tab.entry(cx);
    tab.feed(
        1,
        Some("stage-fra"),
        Some(view("pods", Some(&at.namespace), None, ViewTab::Details)),
        ReadOutcome::Answered { cut: None },
        cx,
    );
    tab.wait_out_hold(cx);
    assert_eq!(tab.shown(cx), before, "the page stays");
    assert_eq!(
        cx.read(|cx| tab.pilot.read(cx).applied.context.clone()),
        context
    );
    // The strip says so.
    assert!(tab.present("follow-strip", cx));
    assert_eq!(
        tab.label("follow-elsewhere", cx).unwrap(),
        format!("The agent read stage-fra; this page stays on {own}.")
    );
    // And so does the trail row, which also has no view to open.
    let rows = tab.rows(cx);
    assert_eq!(rows[0].elsewhere.as_deref(), Some("stage-fra"));
    assert!(rows[0].view.is_none());
}

#[gpui_kit::test]
fn a_tab_of_another_cluster_than_the_shown_one_never_moves_the_page(cx: &mut TestAppContext) {
    let tab = open(cx, "ready", 1280., 880.);
    let at = places(&tab, cx);
    let before = tab.shown(cx);
    // The window moves on to another cluster; the tab keeps its own.
    cx.update_window(tab.handle, |_, window, cx| {
        tab.pilot.update(cx, |pilot, cx| {
            pilot.switch_cluster("dev-fra".into(), window, cx)
        })
    })
    .unwrap();
    cx.run_until_parked();
    let moved = tab.shown(cx);
    tab.read(
        1,
        view("pods", Some(&at.namespace), None, ViewTab::Details),
        cx,
    );
    assert_eq!(tab.shown(cx), moved);
    let _ = before;
}

/// The person's `input` takes the page back, says how many places are newer,
/// and Jump follows again at the latest.
fn hands_back(input: &str, cx: &mut TestAppContext) {
    {
        let tab = open(cx, "ready", 1280., 880.);
        let at = places(&tab, cx);
        let pod = at.pods[0].clone();
        tab.read(
            1,
            view("pods", Some(&at.namespace), None, ViewTab::Details),
            cx,
        );
        // Two newer places, the second of which is the latest.
        tab.read(
            2,
            view(
                "deployments.apps",
                Some(&at.namespace),
                Some(&at.deployment),
                ViewTab::Details,
            ),
            cx,
        );
        tab.read(
            3,
            view("pods", Some(&at.namespace), Some(&pod), ViewTab::Details),
            cx,
        );
        assert!(tab.following(cx), "{input}");
        let identity = example::read(
            &cx.update(|cx| tab.view.read(cx).own.as_ref().unwrap().context.clone()),
            "pods",
            Some(&at.namespace),
            live::now(),
        )
        .unwrap()
        .1
        .into_iter()
        .find(|row| row.identity.name == pod)
        .unwrap()
        .identity;
        tab.draw(cx);
        cx.update_window(tab.handle, |_, window, cx| match input {
            "navigation" => window.click("nav-nodes", cx),
            "selection" => window.click(crate::resources::row_id(&identity), cx),
            "scroll" => window.scroll(
                "page-cell",
                ScrollDelta::Pixels(point(px(0.), px(-40.))),
                cx,
            ),
            _ => {
                // The keyboard is on the page, not in the Agent tab.
                let page = tab.pilot.read(cx).focus.clone();
                window.focus(&page, cx);
                window.press("down", cx)
            }
        })
        .unwrap();
        cx.run_until_parked();
        let toasts = cx
            .update_window(tab.handle, |_, window, cx| window.notifications(cx).len())
            .unwrap();
        assert_eq!(toasts, 1, "{input} says so once");
        assert!(!tab.following(cx), "{input} takes the page back");
        assert_eq!(
            tab.label("follow-toast-newer", cx).as_deref(),
            Some("2 newer"),
            "{input}"
        );
        // The hold ending moves nothing now.
        let held = tab.shown(cx);
        tab.wait_out_hold(cx);
        assert_eq!(tab.shown(cx), held, "{input}");
        // Jump follows again, at the latest place.
        tab.click("follow-jump", cx);
        assert!(tab.following(cx), "{input}");
        let shown = tab.shown(cx).unwrap();
        assert_eq!(object_of(&shown), pod, "{input}");
    }
}

#[gpui_kit::test]
fn any_input_hands_the_page_back_by_navigation(cx: &mut TestAppContext) {
    hands_back("navigation", cx);
}

#[gpui_kit::test]
fn any_input_hands_the_page_back_by_selection(cx: &mut TestAppContext) {
    hands_back("selection", cx);
}

#[gpui_kit::test]
fn any_input_hands_the_page_back_by_scrolling(cx: &mut TestAppContext) {
    hands_back("scroll", cx);
}

#[gpui_kit::test]
fn any_input_hands_the_page_back_by_a_keystroke(cx: &mut TestAppContext) {
    hands_back("key", cx);
}

#[gpui_kit::test]
fn input_in_the_dock_keeps_the_page_following(cx: &mut TestAppContext) {
    let tab = open(cx, "ready", 1280., 880.);
    let at = places(&tab, cx);
    tab.read(
        1,
        view("pods", Some(&at.namespace), None, ViewTab::Details),
        cx,
    );
    // The Agent tab is the person's to use: its toolbar is not the page's.
    tab.click("agent-cluster", cx);
    assert!(tab.following(cx));
    // The strip's own buttons step the trail and keep following.
    tab.click("follow-next", cx);
    assert!(tab.following(cx));
}

#[gpui_kit::test]
fn the_next_prompt_resumes_following(cx: &mut TestAppContext) {
    let tab = open(cx, "ready", 1280., 880.);
    let at = places(&tab, cx);
    tab.read(
        1,
        view("pods", Some(&at.namespace), None, ViewTab::Details),
        cx,
    );
    tab.click("follow-stop", cx);
    assert!(!tab.following(cx));
    assert!(!tab.present("follow-strip", cx));
    tab.script(tour(Duration::ZERO), cx);
    tab.ask("why is api restarting?", cx);
    tab.click("agent-turn-on", cx);
    tab.answered(cx);
    assert!(tab.following(cx));
    assert!(tab.present("follow-strip", cx));
    // The toolbar's toggle does the same by hand.
    tab.click("agent-following", cx);
    assert!(!tab.following(cx));
    tab.click("agent-following", cx);
    assert!(tab.following(cx));
}

#[gpui_kit::test]
fn a_trail_row_click_navigates_and_stops_following(cx: &mut TestAppContext) {
    let tab = open(cx, "ready", 1280., 880.);
    tab.script(tour(Duration::ZERO), cx);
    tab.ask("why is api restarting?", cx);
    tab.click("agent-turn-on", cx);
    tab.answered(cx);
    assert!(tab.following(cx));
    // The second read got the Deployment; the page shows the first or the
    // last, so a click on it is the one that moves it.
    let rows = tab.rows(cx);
    assert_eq!(rows.len(), 3);
    let deployment = rows[1].view.clone().expect("a read that ran has a view");
    tab.click("agent-trail-r2", cx);
    assert!(!tab.following(cx));
    let shown = tab.shown(cx).unwrap();
    assert_eq!(kind_of(&shown), "deployments.apps");
    assert_eq!(object_of(&shown), object_of(&deployment));
    // The refused shell on the trail has no view, so it is no link.
    assert!(rows.iter().all(|row| row.number > 0));
}

#[gpui_kit::test]
fn following_never_opens_a_dialog_or_moves_focus_onto_a_button(cx: &mut TestAppContext) {
    let tab = open(cx, "ready", 1280., 880.);
    let at = places(&tab, cx);
    let pod = at.pods[0].clone();
    let focused = |cx: &mut TestAppContext| {
        cx.update_window(tab.handle, |_, window, cx| window.focused(cx))
            .unwrap()
    };
    let before = focused(cx);
    tab.read(
        1,
        view("pods", Some(&at.namespace), None, ViewTab::Details),
        cx,
    );
    for (number, place) in [
        (
            2,
            view(
                "deployments.apps",
                Some(&at.namespace),
                Some(&at.deployment),
                ViewTab::Details,
            ),
        ),
        (
            3,
            view("pods", Some(&at.namespace), Some(&pod), ViewTab::Logs),
        ),
        (
            4,
            view(
                "deployments.apps",
                Some(&at.namespace),
                Some(&at.deployment),
                ViewTab::Events,
            ),
        ),
    ] {
        tab.wait_out_hold(cx);
        tab.read(number, place, cx);
        tab.draw(cx);
        assert!(
            !cx.update_window(tab.handle, |_, window, cx| window.has_active_dialog(cx))
                .unwrap()
        );
        assert!(focused(cx) == before, "read {number} moved the focus");
    }
    assert!(
        !cx.update_window(tab.handle, |_, window, cx| {
            tab.dock.read(cx).has_focus(window, cx)
        })
        .unwrap(),
        "the log tab opened without the keyboard"
    );
    assert!(tab.following(cx));
}

#[gpui_kit::test]
fn follow_owns_at_most_one_log_tab(cx: &mut TestAppContext) {
    let tab = open(cx, "ready", 1280., 880.);
    let at = places(&tab, cx);
    assert!(at.pods.len() >= 3, "the example has pods to read");
    for (ix, pod) in at.pods.iter().take(3).enumerate() {
        if ix > 0 {
            tab.wait_out_hold(cx);
        }
        tab.read(
            ix as u32 + 1,
            view("pods", Some(&at.namespace), Some(pod), ViewTab::Logs),
            cx,
        );
        assert_eq!(tab.log_tabs(cx), [format!("Pod {pod}")], "read {}", ix + 1);
        assert!(tab.follow_tab(cx).is_some());
    }
    // Following stops: the tab goes.
    tab.click("follow-stop", cx);
    assert!(tab.log_tabs(cx).is_empty());
    assert!(tab.follow_tab(cx).is_none());
}

#[gpui_kit::test]
fn a_log_tab_the_person_used_stays_when_following_stops(cx: &mut TestAppContext) {
    let tab = open(cx, "ready", 1280., 880.);
    let at = places(&tab, cx);
    let pod = at.pods[0].clone();
    tab.read(
        1,
        view("pods", Some(&at.namespace), Some(&pod), ViewTab::Logs),
        cx,
    );
    let id = tab.follow_tab(cx).expect("follow's tab");
    // The person selects it.
    cx.update_window(tab.handle, |_, window, cx| {
        tab.dock
            .update(cx, |dock, cx| dock.select(id, false, window, cx))
    })
    .unwrap();
    assert!(tab.follow_tab(cx).is_none(), "it is the person's now");
    tab.view.update(cx, |view, cx| {
        view.hand_back(cx);
    });
    cx.run_until_parked();
    assert_eq!(tab.log_tabs(cx), [format!("Pod {pod}")]);
}

#[gpui_kit::test]
fn closing_the_agent_tab_takes_follows_log_tab_with_it(cx: &mut TestAppContext) {
    let tab = open(cx, "ready", 1280., 880.);
    let at = places(&tab, cx);
    tab.read(
        1,
        view(
            "pods",
            Some(&at.namespace),
            Some(&at.pods[0]),
            ViewTab::Logs,
        ),
        cx,
    );
    assert_eq!(tab.log_tabs(cx).len(), 1);
    let agent = cx.read(|cx| {
        tab.dock
            .read(cx)
            .tabs
            .iter()
            .find(|tab| matches!(tab.kind, TabKind::Agent(_)))
            .map(|tab| tab.id)
            .unwrap()
    });
    cx.update_window(tab.handle, |_, window, cx| {
        tab.dock
            .update(cx, |dock, cx| dock.close_tab(agent, window, cx))
    })
    .unwrap();
    cx.run_until_parked();
    assert!(tab.log_tabs(cx).is_empty());
}

#[gpui_kit::test]
fn the_dwell_hairline_does_not_redraw_the_page(cx: &mut TestAppContext) {
    let tab = open(cx, "ready", 1280., 880.);
    let at = places(&tab, cx);
    cx.update(|cx| cx.set_reduce_motion(false));
    tab.read(
        1,
        view("pods", Some(&at.namespace), None, ViewTab::Details),
        cx,
    );
    tab.draw(cx);
    let frame = |cx: &mut TestAppContext| {
        cx.background_executor
            .advance_clock(Duration::from_millis(16));
        let asked = cx
            .update_window(tab.handle, |_, window, cx| window.simulate_next_frame(cx))
            .unwrap();
        cx.run_until_parked();
        asked
    };
    // Whatever the first draw asked for settles.
    while frame(cx) > 0 {}
    // A new place starts a hold, and its hairline runs.
    tab.wait_out_hold(cx);
    tab.read(
        2,
        view("services", Some(&at.namespace), None, ViewTab::Details),
        cx,
    );
    frame(cx);
    let strip = probe::count("agent.follow-strip");
    let page = probe::count("resources");
    let chrome = ["chrome.header", "chrome.rail", "chrome.column"].map(probe::count);
    for _ in 0..40 {
        assert!(frame(cx) > 0, "the hairline asks for frames while it runs");
    }
    assert!(probe::count("agent.follow-strip") >= strip + 40);
    assert_eq!(
        probe::count("resources"),
        page,
        "the page is not drawn again"
    );
    assert_eq!(
        ["chrome.header", "chrome.rail", "chrome.column"].map(probe::count),
        chrome
    );
}

#[gpui_kit::test]
fn reduced_motion_leaves_the_hairline_still(cx: &mut TestAppContext) {
    let tab = open(cx, "ready", 1280., 880.);
    let at = places(&tab, cx);
    tab.read(
        1,
        view("pods", Some(&at.namespace), None, ViewTab::Details),
        cx,
    );
    tab.draw(cx);
    let strip = probe::count("agent.follow-strip");
    for _ in 0..20 {
        cx.background_executor
            .advance_clock(Duration::from_millis(16));
        cx.update_window(tab.handle, |_, window, cx| window.simulate_next_frame(cx))
            .unwrap();
        cx.run_until_parked();
    }
    let drawn = probe::count("agent.follow-strip") - strip;
    assert!(drawn <= 1, "the strip drew {drawn} times in 20 frames");
}

#[gpui_kit::test]
fn a_missing_object_reads_not_found_on_the_step_and_the_trail_row(cx: &mut TestAppContext) {
    use super::trail::Mark;
    use freshkube_agent::scripted::{Script, TourRead};
    let tab = open(cx, "ready", 1280., 880.);
    let at = places(&tab, cx);
    tab.script(
        Script {
            tour: vec![TourRead {
                tool: "get_object".into(),
                arguments: serde_json::json!({
                    "kind": "deployments.apps",
                    "namespace": at.namespace,
                    "name": "no-such-deployment",
                }),
            }],
            ..Script::default()
        },
        cx,
    );
    tab.ask("is it there?", cx);
    tab.click("agent-turn-on", cx);
    tab.answered(cx);
    let rows = tab.rows(cx);
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].mark, Mark::NotFound);
    assert!(
        rows[0].view.is_none(),
        "nothing to show for a missing object"
    );
    let steps = cx.update(|cx| tab.view.read(cx).steps());
    assert_eq!(steps.len(), 1);
    let id: &'static str = Box::leak(format!("agent-step-{}", steps[0].0).into_boxed_str());
    let label = tab.label(id, cx).unwrap();
    assert!(label.ends_with("Not found"), "{label}");
    // It is not a refusal and not a break.
    assert!(!label.contains("Refused") && !label.contains("Couldn't read"));
    let _ = Page::Resources;
}
