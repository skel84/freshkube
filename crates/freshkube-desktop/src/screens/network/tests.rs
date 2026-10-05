use std::sync::Arc;

use freshkube_core::inspection::{InspectionSource, InspectionUnavailable};
use gpui_kit::component::Root;
use gpui_kit::test::TestWindowExt;
use gpui_kit::{
    App, AppContext, Entity, InputEvent, MouseMoveEvent, Pixels, Point, ScrollDelta,
    ScrollWheelEvent, SharedString, Size, TestAppContext, TouchPhase, Window, WindowHandle, point,
    px, size,
};
use tokio::runtime::{Builder, Runtime};

// Not `super::*`: gpui_kit's glob would shadow the built-in `#[test]`.
use super::{KubeSpanState, NetworkScreen, ScreenPanel, ScreenSource, View, example};
use crate::backend::Target;
use crate::{fixture, presentation};

fn source(node: &str) -> ScreenSource {
    let nodes = presentation::node_summaries(&fixture::cluster("prod-fra", 1));
    let summary = nodes.iter().find(|summary| summary.name == node).unwrap();
    ScreenSource {
        target: Target {
            epoch: 1,
            context: "prod-fra".into(),
            node: summary.name.clone(),
            address: summary.address.clone(),
        },
        nodes: Arc::new(nodes),
        live: None,
    }
}

fn mount(
    cx: &mut TestAppContext,
    node: &str,
) -> (Runtime, Entity<NetworkScreen>, WindowHandle<Root>) {
    mount_in(cx, node, size(px(1100.), px(1500.)))
}

fn mount_in(
    cx: &mut TestAppContext,
    node: &str,
    bounds: Size<Pixels>,
) -> (Runtime, Entity<NetworkScreen>, WindowHandle<Root>) {
    let runtime = Builder::new_multi_thread()
        .worker_threads(1)
        .enable_all()
        .build()
        .unwrap();
    cx.update(|cx| {
        gpui_kit::init(cx);
        crate::theme::install(cx);
    });
    let source = source(node);
    let mut screen = None;
    let handle = cx.open_window(bounds, |window, cx| {
        let view = cx.new(|cx| {
            let mut view = NetworkScreen::new(runtime.handle().clone(), window, cx);
            view.set_source(Some(source), window, cx);
            view.activate(window, cx);
            view
        });
        screen = Some(view.clone());
        Root::new(view, window, cx)
    });
    cx.run_until_parked();
    (runtime, screen.unwrap(), handle)
}

/// The showing view's row keys, derived as render would.
fn keys(screen: &Entity<NetworkScreen>, cx: &mut App) -> Vec<String> {
    screen.update(cx, |screen, cx| {
        screen.sync_rows(cx);
        screen.row_keys().iter().map(ToString::to_string).collect()
    })
}

/// The showing view's row element ids, top to bottom.
fn row_ids(screen: &Entity<NetworkScreen>, cx: &mut App) -> Vec<SharedString> {
    screen.update(cx, |screen, cx| {
        screen.sync_rows(cx);
        screen.row_ids()
    })
}

#[gpui_kit::test]
fn keyboard_selection_updates_details(cx: &mut TestAppContext) {
    let (_runtime, screen, handle) = mount(cx, "talos-cp-fra1-01");
    cx.update_window(handle.into(), |_, window, cx| {
        window.render_frame(cx);
        assert!(screen.read(cx).loader.data().is_some());
        window.find("interface-details");
        let ids = row_ids(&screen, cx);
        window.click(ids[0].clone(), cx);
        let first = keys(&screen, cx)[0].clone();
        window.press("down", cx);
        let second = keys(&screen, cx)[1].clone();
        assert_eq!(screen.read(cx).selected_iface.as_ref(), Some(&second));
        assert_ne!(first, second);
        window.render_frame(cx);
        assert_eq!(window.find(ids[1].clone()).selected(), Some(true));
        assert_eq!(window.find(ids[0].clone()).selected(), Some(false));
        window.press("end", cx);
        let last = keys(&screen, cx).last().cloned();
        assert_eq!(screen.read(cx).selected_iface, last);
    })
    .unwrap();
}

#[gpui_kit::test]
fn second_refresh_produces_rates(cx: &mut TestAppContext) {
    let (_runtime, screen, handle) = mount(cx, "talos-cp-fra1-01");
    cx.update_window(handle.into(), |_, window, cx| {
        window.render_frame(cx);
        let first = screen.read(cx).snapshot().unwrap().clone();
        assert!(first.interfaces.iter().all(|i| i.rate.is_none()));
        assert!(
            window
                .find("network-summary")
                .label()
                .is_some_and(|label| label.contains("measuring"))
        );
        screen.update(cx, |screen, cx| screen.refresh(window, cx));
        window.render_frame(cx);
        let second = screen.read(cx).snapshot().unwrap().clone();
        assert!(second.interfaces.iter().all(|i| i.rate.is_some()));
        assert!(second.totals.rx_bytes_per_sec > 0);
    })
    .unwrap();
}

#[gpui_kit::test]
fn connections_view_filters_rows(cx: &mut TestAppContext) {
    let (_runtime, screen, handle) = mount(cx, "talos-wk-fra1-02");
    cx.update_window(handle.into(), |_, window, cx| {
        window.render_frame(cx);
        window.click("network-view-connections", cx);
        window.render_frame(cx);
        assert_eq!(screen.read(cx).view, View::Connections);
        window.find("connection-list");
        window.find(row_ids(&screen, cx)[0].clone());
        let all = keys(&screen, cx).len();
        window.click("state-syn-sent", cx);
        window.render_frame(cx);
        let syn = keys(&screen, cx).len();
        assert_eq!(syn, 7, "example has seven SYN_SENT sockets");
        window.click("state-all", cx);
        screen.update(cx, |screen, cx| {
            screen
                .query
                .update(cx, |input, cx| input.set_value("kubelet", window, cx));
        });
        window.render_frame(cx);
        let text = keys(&screen, cx).len();
        assert!(text > 0 && text < all);
        // The listeners tab keeps listening sockets only.
        window.click("network-view-listeners", cx);
        window.render_frame(cx);
        window.find("listener-list");
        assert!(keys(&screen, cx).len() <= text);
    })
    .unwrap();
}

#[gpui_kit::test]
fn degraded_node_is_notable_and_kubespan_is_unknown(cx: &mut TestAppContext) {
    let (_runtime, screen, handle) = mount(cx, "talos-wk-fra1-02");
    cx.update_window(handle.into(), |_, window, cx| {
        window.render_frame(cx);
        // KubeSpan isn't asked for until its tab is shown, so nothing is
        // reported about it yet.
        assert!(window.try_find("partial-notice").is_none());
        window.find("network-notices");
        screen.update(cx, |screen, cx| {
            screen.set_sort(super::Sort::Errors, cx);
        });
        assert_eq!(keys(&screen, cx).first().map(String::as_str), Some("eth0"));
        window.click("network-view-kubespan", cx);
        window.render_frame(cx);
        // It couldn't be read: reported as unavailable, not failed.
        window.find("kubespan-status");
        window.find("partial-notice");
    })
    .unwrap();
}

#[gpui_kit::test]
fn kubespan_peers_render_when_enabled(cx: &mut TestAppContext) {
    let (_runtime, screen, handle) = mount(cx, "talos-cp-fra1-01");
    cx.update_window(handle.into(), |_, window, cx| {
        window.render_frame(cx);
        window.click("network-view-kubespan", cx);
        window.render_frame(cx);
        assert!(matches!(
            screen.read(cx).kubespan.data(),
            Some(KubeSpanState::Enabled(peers)) if !peers.is_empty()
        ));
        window.find("peer-list");
        window.find(row_ids(&screen, cx)[0].clone());
        window.find("peer-details");
    })
    .unwrap();
}

#[gpui_kit::test]
fn missing_connections_still_show_interfaces(cx: &mut TestAppContext) {
    let (_runtime, screen, handle) = mount(cx, "talos-cp-fra1-01");
    cx.update_window(handle.into(), |_, window, cx| {
        screen.update(cx, |screen, _| {
            let mut data = (**screen.loader.data().unwrap()).clone();
            data.revision = super::NetworkData::next_revision();
            data.snapshot.connections = None;
            data.snapshot.unavailable.push(InspectionUnavailable {
                source: InspectionSource::NetworkConnections,
                message: "netstat timed out".into(),
            });
            let target = screen.source.as_ref().unwrap().target.clone();
            screen.loader.resolve(target, Ok(Arc::new(data)));
        });
        window.render_frame(cx);
        window.find("partial-notice");
        window.find(row_ids(&screen, cx)[0].clone());
        window.click("network-view-connections", cx);
        window.render_frame(cx);
        assert!(row_ids(&screen, cx).is_empty());
        // Unknown, not empty: the table says netstat didn't answer.
        window.find("connection-empty");
    })
    .unwrap();
}

#[gpui_kit::test]
fn silent_node_offers_retry_without_data(cx: &mut TestAppContext) {
    let (_runtime, screen, handle) = mount(cx, "talos-wk-fra1-03");
    cx.update_window(handle.into(), |_, window, cx| {
        window.render_frame(cx);
        assert!(screen.read(cx).loader.data().is_none());
        window.find("screen-retry");
        assert!(window.try_find("interface-list").is_none());
    })
    .unwrap();
}

#[gpui_kit::test]
fn changing_target_drops_old_data(cx: &mut TestAppContext) {
    let (_runtime, screen, handle) = mount(cx, "talos-cp-fra1-01");
    cx.update_window(handle.into(), |_, window, cx| {
        screen.update(cx, |screen, cx| {
            screen.selected_iface = Some("eth0".into());
            screen.iface_filter = Some("eth0".into());
            screen.set_source(Some(source("talos-wk-fra1-02")), window, cx);
            assert!(screen.loader.data().is_none());
            assert!(screen.selected_iface.is_none());
            assert!(screen.iface_filter.is_none());
        });
    })
    .unwrap();
}

#[test]
fn example_keeps_unresponsive_nodes_unknown() {
    let silent = source("talos-wk-fra1-03");
    assert!(example(&silent, 0).is_err());
}

/// A scratch directory under the system temp dir, removed on drop even
/// when an assertion fails.
struct Scratch(std::path::PathBuf);

impl Scratch {
    fn new(name: &str) -> Self {
        let path =
            std::env::temp_dir().join(format!("freshkube-desktop-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&path);
        std::fs::create_dir_all(&path).unwrap();
        Self(path)
    }

    fn files(&self) -> Vec<std::path::PathBuf> {
        std::fs::read_dir(&self.0)
            .unwrap()
            .map(|entry| entry.unwrap().path())
            .collect()
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// Renders until `ready` holds. The capture worker runs on Tokio, so
/// this lets real time pass between frames.
fn wait_until(
    cx: &mut TestAppContext,
    handle: WindowHandle<Root>,
    what: &str,
    ready: impl Fn(&mut gpui_kit::Window, &mut gpui_kit::App) -> bool,
) {
    for _ in 0..300 {
        cx.run_until_parked();
        let done = cx
            .update_window(handle.into(), |_, window, cx| {
                window.render_frame(cx);
                ready(window, cx)
            })
            .unwrap();
        if done {
            return;
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    panic!("timed out waiting for {what}");
}

fn status(window: &gpui_kit::Window) -> String {
    window
        .find("capture-status")
        .label()
        .unwrap_or_default()
        .to_owned()
}

/// Starts an example capture and stops it once four packets are in.
fn capture_four_packets(
    cx: &mut TestAppContext,
    handle: WindowHandle<Root>,
    screen: &Entity<NetworkScreen>,
) {
    cx.executor().allow_parking();
    cx.update_window(handle.into(), |_, window, cx| {
        window.render_frame(cx);
        window.click("capture-start", cx);
        window.render_frame(cx);
    })
    .unwrap();
    wait_until(cx, handle, "the example packets", |window, _| {
        status(window).contains("4 complete packets")
    });
    cx.update_window(handle.into(), |_, window, cx| {
        window.click("capture-stop", cx);
    })
    .unwrap();
    wait_until(cx, handle, "the capture to stop", |_, cx| {
        screen.read(cx).capture.can_save()
    });
}

#[gpui_kit::test]
fn capture_is_offered_only_when_it_can_run(cx: &mut TestAppContext) {
    let (_runtime, screen, handle) = mount(cx, "talos-cp-fra1-01");
    cx.update_window(handle.into(), |_, window, cx| {
        window.render_frame(cx);
        assert!(window.find("capture-panel").visible());
        // Nothing to stop and nothing captured yet.
        assert!(!screen.read(cx).capture.active());
        assert!(!screen.read(cx).capture.can_save());
        assert!(
            window
                .find("capture-interface")
                .label()
                .unwrap()
                .starts_with("Capture interface ")
        );
        // The Talos API port is left out unless the user says otherwise.
        assert!(screen.read(cx).capture.exclude_api);
        window.click("capture-exclude-api", cx);
        window.render_frame(cx);
        assert!(!screen.read(cx).capture.exclude_api);
        window.click(("capture-limit", 4usize), cx);
        window.render_frame(cx);
        assert_eq!(screen.read(cx).capture.limit_mib, 4);
    })
    .unwrap();
    cx.executor().allow_parking();
    cx.update_window(handle.into(), |_, window, cx| {
        window.click("capture-start", cx);
        window.render_frame(cx);
        // Capturing: the setup is locked, and only Stop is on offer.
        assert!(screen.read(cx).capture.active());
        assert!(!screen.read(cx).capture.can_save());
        window.click("capture-exclude-api", cx);
        window.click(("capture-limit", 1usize), cx);
        window.render_frame(cx);
        assert!(!screen.read(cx).capture.exclude_api);
        assert_eq!(screen.read(cx).capture.limit_mib, 4);
    })
    .unwrap();
    wait_until(cx, handle, "the example packets", |window, _| {
        status(window).contains("4 complete packets")
    });
    // A capture that is still running can't be saved half-framed.
    cx.update_window(handle.into(), |_, _, cx| {
        assert!(!screen.read(cx).capture.can_save());
    })
    .unwrap();
    cx.update_window(handle.into(), |_, window, cx| {
        window.click("capture-stop", cx);
    })
    .unwrap();
    wait_until(cx, handle, "the capture to stop", |_, cx| {
        !screen.read(cx).capture.active()
    });
    cx.update_window(handle.into(), |_, _window, cx| {
        assert!(screen.read(cx).capture.can_save());
        let status = screen.read(cx).capture.status();
        assert!(status.contains("Stopped by user"), "{status}");
    })
    .unwrap();
}

#[gpui_kit::test]
fn saving_writes_a_valid_pcap_where_the_user_chose(cx: &mut TestAppContext) {
    let scratch = Scratch::new("capture-save");
    let chosen = scratch.0.join("chosen.pcap");
    let (_runtime, screen, handle) = mount(cx, "talos-cp-fra1-01");
    capture_four_packets(cx, handle, &screen);
    cx.update_window(handle.into(), |_, window, cx| {
        window.click("capture-save", cx);
        window.render_frame(cx);
    })
    .unwrap();
    // Nothing is written until the dialog is answered.
    assert!(scratch.files().is_empty());
    let target = chosen.clone();
    cx.simulate_new_path_selection(move |_| Some(target));
    wait_until(cx, handle, "the save to finish", |window, _| {
        window
            .try_find("capture-save-status")
            .is_some_and(|status| status.label().unwrap_or_default().starts_with("Saved to "))
    });
    assert_eq!(scratch.files(), vec![chosen.clone()]);
    let bytes = std::fs::read(&chosen).unwrap();
    let mut framing = freshkube_core::pcap::PcapFraming::default();
    framing.advance(&bytes).unwrap();
    assert_eq!(framing.records, 4);
    assert_eq!(framing.complete, bytes.len(), "no trailing partial record");
    assert_eq!(&bytes[..4], &[0xd4, 0xc3, 0xb2, 0xa1]);
}

#[gpui_kit::test]
fn cancelling_the_save_dialog_writes_nothing(cx: &mut TestAppContext) {
    let scratch = Scratch::new("capture-cancel");
    let (_runtime, screen, handle) = mount(cx, "talos-cp-fra1-01");
    capture_four_packets(cx, handle, &screen);
    cx.update_window(handle.into(), |_, window, cx| {
        window.click("capture-save", cx);
        window.render_frame(cx);
    })
    .unwrap();
    cx.simulate_new_path_selection(|_| None);
    wait_until(cx, handle, "the cancelled save", |window, _| {
        window
            .try_find("capture-save-status")
            .is_some_and(|status| status.label().unwrap_or_default().contains("cancelled"))
    });
    assert!(scratch.files().is_empty());
    // The capture is still there to save somewhere else.
    cx.update_window(handle.into(), |_, _, cx| {
        assert!(screen.read(cx).capture.can_save());
    })
    .unwrap();
}

#[gpui_kit::test]
fn changing_the_target_drops_the_capture(cx: &mut TestAppContext) {
    let (_runtime, screen, handle) = mount(cx, "talos-cp-fra1-01");
    capture_four_packets(cx, handle, &screen);
    let other = source("talos-wk-fra1-02");
    cx.update_window(handle.into(), |_, window, cx| {
        screen.update(cx, |screen, cx| screen.set_source(Some(other), window, cx));
        window.render_frame(cx);
    })
    .unwrap();
    cx.update_window(handle.into(), |_, window, cx| {
        let capture = &screen.read(cx).capture;
        assert!(!capture.can_save(), "the old node's capture is gone");
        assert!(!capture.active());
        if let Some(status) = window.try_find("capture-status") {
            let status = status.label().unwrap_or_default().to_owned();
            assert!(!status.contains("complete packets"), "{status}");
        }
    })
    .unwrap();
}

#[test]
fn capture_files_are_named_for_the_node_and_the_time() {
    use chrono::TimeZone;
    let when = chrono::Local.with_ymd_and_hms(2026, 3, 4, 5, 6, 7).unwrap();
    assert_eq!(
        super::capture::file_name("talos cp/01", when),
        "talos-capture-talos-cp-01-20260304-050607.pcap"
    );
}

#[gpui_kit::test]
fn connection_rows_are_derived_only_when_the_data_filters_or_sort_change(cx: &mut TestAppContext) {
    let (_runtime, screen, handle) = mount(cx, "talos-wk-fra1-02");
    cx.update_window(handle.into(), |_, window, cx| {
        window.render_frame(cx);
        window.click("network-view-connections", cx);
        window.render_frame(cx);
        let computed = || crate::desktop::probe::count("network.rows");
        let base = computed();
        // Render, selection and navigation read the rows as derived.
        window.render_frame(cx);
        window.click(row_ids(&screen, cx)[0].clone(), cx);
        window.press("down", cx);
        window.press("down", cx);
        window.render_frame(cx);
        assert_eq!(computed(), base, "nothing they depend on changed");
        // The state filter.
        window.click("state-syn-sent", cx);
        assert_eq!(keys(&screen, cx).len(), 7);
        assert_eq!(computed(), base + 1);
        window.click("state-all", cx);
        // Sort.
        screen.update(cx, |screen, cx| screen.set_sort(super::Sort::Port, cx));
        keys(&screen, cx);
        let sorted = computed();
        assert!(sorted > base + 1);
        // The filter text.
        screen.update(cx, |screen, cx| {
            screen
                .query
                .update(cx, |input, cx| input.set_value("kubelet", window, cx));
        });
        keys(&screen, cx);
        assert_eq!(computed(), sorted + 1);
        keys(&screen, cx);
        assert_eq!(computed(), sorted + 1);
        // The tab: listeners keep listening sockets only.
        window.click("network-view-listeners", cx);
        keys(&screen, cx);
        assert_eq!(computed(), sorted + 2);
        // A new sample.
        screen.update(cx, |screen, cx| screen.refresh(window, cx));
        keys(&screen, cx);
        assert_eq!(computed(), sorted + 3);
    })
    .unwrap();
}

#[gpui_kit::test]
fn selection_survives_a_refresh_by_key(cx: &mut TestAppContext) {
    let (_runtime, screen, handle) = mount(cx, "talos-wk-fra1-02");
    cx.update_window(handle.into(), |_, window, cx| {
        window.render_frame(cx);
        window.click("network-view-connections", cx);
        window.render_frame(cx);
        window.click(row_ids(&screen, cx)[0].clone(), cx);
        window.press("down", cx);
        window.press("down", cx);
        let key = screen.read(cx).selected_conn.clone().unwrap();
        assert_eq!(keys(&screen, cx).iter().position(|k| *k == key), Some(2));
        screen.update(cx, |screen, cx| screen.refresh(window, cx));
        // The sockets are the same, so the same one stays selected.
        assert_eq!(keys(&screen, cx).iter().position(|k| *k == key), Some(2));
        let after = screen.read(cx).selected_connection().unwrap();
        assert_eq!(super::conn_key(&after.connection), key);
    })
    .unwrap();
}

#[gpui_kit::test]
fn kubespan_is_not_collected_while_its_tab_is_hidden(cx: &mut TestAppContext) {
    let (_runtime, screen, handle) = mount(cx, "talos-cp-fra1-01");
    cx.update_window(handle.into(), |_, window, cx| {
        window.render_frame(cx);
        for _ in 0..2 {
            screen.update(cx, |screen, cx| screen.refresh(window, cx));
        }
        window.click("network-view-connections", cx);
        window.render_frame(cx);
        let screen = screen.read(cx);
        assert!(screen.loader.data().is_some(), "the rest still loads");
        assert!(screen.kubespan.data().is_none());
        assert!(!screen.kubespan.is_loading());
        assert!(screen.kubespan.error().is_none());
        // Unknown, never "off" or failed, while it hasn't been asked for.
        let label = window.find("network-view-kubespan");
        drop(label);
    })
    .unwrap();
}

#[gpui_kit::test]
fn kubespan_loads_when_its_tab_is_shown(cx: &mut TestAppContext) {
    let (_runtime, screen, handle) = mount(cx, "talos-cp-fra1-01");
    cx.update_window(handle.into(), |_, window, cx| {
        window.render_frame(cx);
        assert!(screen.read(cx).kubespan.data().is_none());
        window.click("network-view-kubespan", cx);
        window.render_frame(cx);
        assert!(matches!(
            screen.read(cx).kubespan.data(),
            Some(KubeSpanState::Enabled(peers)) if !peers.is_empty()
        ));
        window.find("peer-list");
        // From now on a refresh brings it along.
        let before = screen.read(cx).kubespan.last_successful();
        screen.update(cx, |screen, cx| screen.refresh(window, cx));
        assert!(screen.read(cx).kubespan.last_successful() >= before);
    })
    .unwrap();
}

#[gpui_kit::test]
fn kubespan_tab_says_loading_not_off_before_it_has_an_answer(cx: &mut TestAppContext) {
    let (_runtime, screen, handle) = mount(cx, "talos-cp-fra1-01");
    cx.update_window(handle.into(), |_, window, cx| {
        window.render_frame(cx);
        screen.update(cx, |screen, cx| {
            // As the tab is shown, before any answer has arrived.
            screen.view = View::KubeSpan;
            cx.notify();
        });
        window.render_frame(cx);
        let status = window.find("kubespan-status");
        let label = status.label().unwrap_or_default().to_owned();
        assert!(label.contains("Loading"), "{label}");
        assert!(!label.contains("isn't enabled"));
    })
    .unwrap();
}

/// Each view's table follows DESIGN.md's rows and header at both text sizes.
#[gpui_kit::test]
fn network_draws_the_shared_tables_at_both_text_sizes(cx: &mut TestAppContext) {
    use crate::desktop::layout_check::{Table, assert_table};
    let (_runtime, screen, handle) = mount(cx, "talos-cp-fra1-01");
    let views = [
        (View::Interfaces, "interface-table-scroll", "interface-list"),
        (
            View::Connections,
            "connection-table-scroll",
            "connection-list",
        ),
        (View::Listeners, "listener-table-scroll", "listener-list"),
        (View::KubeSpan, "peer-table-scroll", "peer-list"),
    ];
    for text_size in [crate::ui::BASE_TEXT, 20.] {
        for (view, scroll, list) in views {
            cx.update_window(handle.into(), |_, window, cx| {
                crate::text_size::set(text_size, cx);
                screen.update(cx, |screen, cx| screen.switch(view, window, cx));
            })
            .unwrap();
            cx.run_until_parked();
            cx.update_window(handle.into(), |_, window, cx| {
                let table = Table {
                    table: Some(scroll),
                    list,
                };
                let rows = assert_table(window, cx, &table);
                assert!(rows.header.is_some(), "{view:?} at {text_size}: {rows:#?}");
            })
            .unwrap();
        }
    }
}

/// Marks only what needs a look: an interface with errors, a peer by its
/// state, and none on a quiet interface.
#[gpui_kit::test]
fn glyphs_mark_errors_and_peer_states(cx: &mut TestAppContext) {
    let (_runtime, _screen, handle) = mount(cx, "talos-wk-fra1-02");
    cx.update_window(handle.into(), |_, window, cx| {
        window.render_frame(cx);
        window.find("interface-eth0-state");
        assert!(window.try_find("interface-lo-state").is_none());
    })
    .unwrap();
    let (_runtime, screen, handle) = mount(cx, "talos-cp-fra1-01");
    cx.update_window(handle.into(), |_, window, cx| {
        window.render_frame(cx);
        window.click("network-view-kubespan", cx);
        window.render_frame(cx);
        let first = row_ids(&screen, cx)[0].clone();
        window.find(SharedString::from(format!("{first}-state")));
    })
    .unwrap();
}

/// One wheel event at `position`, as the start of a gesture.
fn wheel(window: &mut Window, position: Point<Pixels>, y: f32, cx: &mut App) {
    window.dispatch_event(
        MouseMoveEvent {
            position,
            ..Default::default()
        }
        .to_platform_input(),
        cx,
    );
    window.dispatch_event(
        ScrollWheelEvent {
            position,
            delta: ScrollDelta::Pixels(point(px(0.), px(y))),
            touch_phase: TouchPhase::Started,
            ..Default::default()
        }
        .to_platform_input(),
        cx,
    );
    window.render_frame(cx);
}

/// A short window keeps a list of `SHORT_LIST_HEIGHT` and scrolls the page
/// down to the details under it, rather than squeezing either.
#[gpui_kit::test]
fn a_short_window_scrolls_the_page_to_the_details(cx: &mut TestAppContext) {
    use freshkube_ui::page::SHORT_LIST_HEIGHT;
    let (_runtime, _screen, handle) = mount_in(cx, "talos-cp-fra1-01", size(px(760.), px(560.)));
    cx.update_window(handle.into(), |_, window, cx| {
        crate::text_size::set(20., cx);
        window.render_frame(cx);
        let bottom = window.viewport_size().height;
        let list = window.find("interface-table-scroll").bounds();
        let least = crate::ui::dp_px(SHORT_LIST_HEIGHT, window) - px(2.);
        let table = window.find("network-table").bounds();
        assert!(table.size.height >= least, "{table:?} < {least:?}");
        let details = window.find("interface-details").bounds();
        assert!(
            details.top() >= table.bottom(),
            "the details sit under the list"
        );
        assert!(
            details.bottom() > bottom,
            "the page is taller than the window"
        );
        // A wheel over the page, outside the table, scrolls to the details.
        let at = point(list.center().x, px(40.));
        for _ in 0..20 {
            wheel(window, at, -200., cx);
        }
        let details = window.find("interface-details").bounds();
        assert!(
            details.top() < bottom && details.top() >= px(0.),
            "the details are out of reach: {details:?}"
        );
    })
    .unwrap();
}
