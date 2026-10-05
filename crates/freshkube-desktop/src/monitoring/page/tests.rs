use freshkube_core::monitoring::{
    Candidate, Discovery, ErrorKind, PrometheusService, QueryError, Rank, Tried,
};
use gpui_kit::component::{Root, Theme, ThemeMode};
use gpui_kit::test::{TestAppContextExt, TestWindowExt};
use gpui_kit::{AnyWindowHandle, AppContext, Entity, SharedString, TestAppContext, px, size};

use super::*;
use crate::monitoring::panel::PanelEvent;

const NOW: i64 = 1_700_003_600;

fn example_source() -> KubeSource {
    KubeSource {
        id: "example".into(),
        context: "prod-fra".into(),
        access: KubeAccess::Example,
    }
}

/// The page alone in a window, its clock fixed and its source example data.
fn mount(
    cx: &mut TestAppContext,
    source: Option<KubeSource>,
) -> (
    tokio::runtime::Runtime,
    AnyWindowHandle,
    Entity<MonitoringPage>,
) {
    let runtime = tokio::runtime::Runtime::new().unwrap();
    let (handle, page) = mount_with(cx, &runtime, source, None, None);
    (runtime, handle, page)
}

fn mount_with(
    cx: &mut TestAppContext,
    runtime: &tokio::runtime::Runtime,
    source: Option<KubeSource>,
    preferences: Option<&std::path::Path>,
    secrets: Option<crate::secrets::Secrets>,
) -> (AnyWindowHandle, Entity<MonitoringPage>) {
    cx.update(|cx| {
        gpui_kit::init(cx);
        crate::theme::install(cx);
        crate::text_size::install(None, cx);
        Theme::change(ThemeMode::Dark, None, cx);
        cx.set_reduce_motion(true);
    });
    let mut page = None;
    let window = cx.open_window(size(px(1200.), px(900.)), |window, cx| {
        let view = cx.new(|cx| {
            let mut view =
                MonitoringPage::new(runtime.handle().clone(), preferences, secrets.clone(), cx);
            view.now = || NOW;
            view.set_source(source, cx);
            view
        });
        page = Some(view.clone());
        Root::new(view, window, cx)
    });
    cx.run_until_parked();
    (window.into(), page.unwrap())
}

fn frame(cx: &mut TestAppContext, handle: AnyWindowHandle) {
    cx.update_window(handle, |_, window, cx| window.render_frame(cx))
        .unwrap();
    cx.run_until_parked();
}

fn shown(cx: &mut TestAppContext, handle: AnyWindowHandle, id: &str) -> bool {
    let id = SharedString::from(id.to_owned());
    cx.update_window(handle, move |_, window, cx| {
        window.render_frame(cx);
        window.try_find(id).is_some()
    })
    .unwrap()
}

fn show(cx: &mut TestAppContext, handle: AnyWindowHandle, page: &Entity<MonitoringPage>) {
    cx.update(|cx| page.update(cx, |page, cx| page.set_visible(true, cx)));
    cx.run_until_parked();
    frame(cx, handle);
}

/// The slots answered, by title.
fn ready(cx: &mut TestAppContext, page: &Entity<MonitoringPage>) -> Vec<String> {
    cx.read(|cx| {
        let page = page.read(cx);
        page.board
            .as_ref()
            .map(|board| {
                board
                    .slots
                    .iter()
                    .filter(|slot| slot.view.read(cx).is_ready())
                    .map(|slot| slot.spec.title.clone())
                    .collect()
            })
            .unwrap_or_default()
    })
}

fn slot_count(cx: &mut TestAppContext, page: &Entity<MonitoringPage>) -> usize {
    cx.read(|cx| {
        page.read(cx)
            .board
            .as_ref()
            .map_or(0, |board| board.slots.len())
    })
}

#[gpui_kit::test]
fn a_hidden_page_reads_nothing_and_showing_answers_the_panels_in_view(cx: &mut TestAppContext) {
    let (_runtime, handle, page) = mount(cx, Some(example_source()));
    cx.read(|cx| {
        let page = page.read(cx);
        assert!(page.board.is_none(), "a hidden page opened a dashboard");
        assert!(matches!(page.connection, Connection::None));
    });

    show(cx, handle, &page);
    assert!(shown(cx, handle, "monitoring-title"));
    assert!(shown(cx, handle, "monitoring-variable-node"));
    assert!(shown(cx, handle, "monitoring-variable-namespace"));
    let answered = ready(cx, &page);
    assert!(answered.contains(&"Ready nodes".to_owned()), "{answered:?}");
    assert!(
        answered.contains(&"CPU usage by node".to_owned()),
        "{answered:?}"
    );
    assert_eq!(answered.len(), slot_count(cx, &page));
}

#[gpui_kit::test]
fn without_a_source_the_page_says_it_is_not_connected(cx: &mut TestAppContext) {
    let (_runtime, handle, page) = mount(cx, None);
    show(cx, handle, &page);
    assert!(shown(cx, handle, "monitoring-page"));
    cx.read(|cx| assert!(matches!(page.read(cx).connection, Connection::None)));
    assert!(ready(cx, &page).is_empty());
    assert!(!shown(cx, handle, "monitoring-grid"));
}

#[gpui_kit::test]
fn discovery_states_show_what_was_looked_for_and_offer_retry(cx: &mut TestAppContext) {
    let (_runtime, handle, page) = mount(cx, Some(example_source()));
    show(cx, handle, &page);
    let service = PrometheusService::new("monitoring", "metrics", 9090);
    let missing = Discovery::Missing {
        candidates: vec![Candidate {
            service: service.clone(),
            rank: Rank::Named,
        }],
        tried: vec![Tried {
            service: PrometheusService::new("monitoring", "prometheus", 9090),
            error: QueryError::new(ErrorKind::NotFound, "Not found"),
        }],
    };
    cx.update(|cx| page.update(cx, |page, cx| page.discovered(Ok(missing), cx)));
    assert!(shown(cx, handle, "monitoring-candidate-0"));
    assert!(shown(cx, handle, "monitoring-retry"));
    assert!(!shown(cx, handle, "monitoring-grid"));

    cx.update(|cx| {
        page.update(cx, |page, cx| {
            page.discovered(Err(QueryError::refused()), cx)
        })
    });
    assert!(matches!(
        cx.read(|cx| page.read(cx).connection_name()),
        "refused"
    ));
    assert!(shown(cx, handle, "monitoring-retry"));

    let failed = QueryError::new(ErrorKind::Unavailable, "Service unavailable");
    cx.update(|cx| page.update(cx, |page, cx| page.discovered(Err(failed), cx)));
    assert_eq!(cx.read(|cx| page.read(cx).connection_name()), "failed");

    // Try again starts over: the example source answers at once.
    cx.update_window(handle, |_, window, cx| window.click("monitoring-retry", cx))
        .unwrap();
    cx.run_until_parked();
    frame(cx, handle);
    assert_eq!(cx.read(|cx| page.read(cx).connection_name()), "example");
    assert!(shown(cx, handle, "monitoring-grid"));
}

#[gpui_kit::test]
fn a_variable_change_reads_the_variables_and_panels_again(cx: &mut TestAppContext) {
    let (_runtime, handle, page) = mount(cx, Some(example_source()));
    show(cx, handle, &page);
    let (index, before, choice) = cx.read(|cx| {
        let board = page.read(cx).board.as_ref().unwrap();
        let control = board
            .controls
            .iter()
            .find(|control| control.id.as_ref() == "monitoring-variable-node")
            .unwrap();
        (
            control.index,
            control.value.clone(),
            control.options[1].clone(),
        )
    });
    assert_eq!(before.as_ref(), "All");
    let generation = cx.read(|cx| page.read(cx).generation);
    cx.update(|cx| page.update(cx, |page, cx| page.set_variable(index, choice.clone(), cx)));
    cx.run_until_parked();
    frame(cx, handle);
    cx.read(|cx| {
        let page = page.read(cx);
        assert!(page.generation > generation);
        let board = page.board.as_ref().unwrap();
        let control = board.controls.iter().find(|c| c.index == index).unwrap();
        assert_eq!(control.value, choice);
        assert_eq!(control.options[0].as_ref(), "All");
        assert_eq!(
            control
                .options
                .iter()
                .filter(|o| o.as_ref() == "All")
                .count(),
            1
        );
    });
    assert_eq!(ready(cx, &page).len(), slot_count(cx, &page));
}

#[gpui_kit::test]
fn a_time_range_change_asks_every_panel_over_the_new_window(cx: &mut TestAppContext) {
    let (_runtime, handle, page) = mount(cx, Some(example_source()));
    show(cx, handle, &page);
    let (generation, span) = cx.read(|cx| {
        let page = page.read(cx);
        (page.generation, page.board.as_ref().unwrap().span)
    });
    assert_eq!(span, 6 * 3600);
    cx.update(|cx| page.update(cx, |page, cx| page.set_range(3600, cx)));
    cx.run_until_parked();
    frame(cx, handle);
    cx.read(|cx| {
        let page = page.read(cx);
        let board = page.board.as_ref().unwrap();
        assert_eq!(page.generation, generation + 1);
        assert_eq!(board.range_label.as_ref(), "Last 1 hour");
        assert_eq!(board.window.unwrap().span, 3600);
        assert!(
            board
                .slots
                .iter()
                .filter(|slot| slot.view.read(cx).is_ready())
                .all(|slot| slot.asked == Some(page.generation))
        );
    });
}

#[gpui_kit::test]
fn the_cursor_on_one_chart_shows_on_the_others(cx: &mut TestAppContext) {
    let (_runtime, handle, page) = mount(cx, Some(example_source()));
    show(cx, handle, &page);
    let (cpu, memory) = cx.read(|cx| {
        let board = page.read(cx).board.as_ref().unwrap();
        let find = |title: &str| {
            board
                .slots
                .iter()
                .find(|slot| slot.spec.title == title)
                .unwrap()
                .view
                .clone()
        };
        (find("CPU usage by node"), find("Memory usage by node"))
    });
    let time = (NOW - 3600) as f64;
    cx.update(|cx| cpu.update(cx, |_, cx| cx.emit(PanelEvent::Cursor(Some(time)))));
    assert!(cx.read(|cx| memory.read(cx).has_cursor()));
    cx.update(|cx| cpu.update(cx, |_, cx| cx.emit(PanelEvent::Cursor(None))));
    assert!(!cx.read(|cx| memory.read(cx).has_cursor()));
}

#[gpui_kit::test]
fn hiding_mid_request_drops_the_reads_and_showing_asks_again(cx: &mut TestAppContext) {
    let (_runtime, handle, page) = mount(cx, Some(example_source()));
    // Show and hide before any read can answer.
    cx.update(|cx| {
        page.update(cx, |page, cx| {
            page.set_visible(true, cx);
            assert!(page.board.as_ref().unwrap().resolving.is_some());
            page.set_visible(false, cx);
        })
    });
    cx.run_until_parked();
    cx.read(|cx| {
        let page = page.read(cx);
        let board = page.board.as_ref().unwrap();
        assert!(board.resolving.is_none());
        assert!(board.variables.is_none(), "an answer arrived after hiding");
        assert!(page.refresh_task.is_none());
    });
    assert!(ready(cx, &page).is_empty());

    // Panels asked, then hidden: their reads go and nothing lands.
    show(cx, handle, &page);
    let before = cx.read(|cx| page.read(cx).generation);
    cx.update(|cx| {
        page.update(cx, |page, cx| {
            page.ask_again(cx);
            page.set_visible(false, cx);
            let board = page.board.as_ref().unwrap();
            assert!(board.slots.iter().all(|slot| slot.request.is_none()));
            assert!(board.slots.iter().all(|slot| slot.asked.is_none()));
        })
    });
    cx.run_until_parked();
    assert!(cx.read(|cx| page.read(cx).generation) > before);

    show(cx, handle, &page);
    assert_eq!(ready(cx, &page).len(), slot_count(cx, &page));
    cx.read(|cx| {
        let page = page.read(cx);
        let generation = page.generation;
        assert!(
            page.board
                .as_ref()
                .unwrap()
                .slots
                .iter()
                .all(|slot| slot.asked == Some(generation))
        );
    });
}

#[gpui_kit::test]
fn a_folded_row_shows_its_count_and_unfolding_asks_its_panels(cx: &mut TestAppContext) {
    let (_runtime, handle, page) = mount(cx, Some(example_source()));
    show(cx, handle, &page);
    cx.update(|cx| {
        page.update(cx, |page, cx| {
            page.open(EntryId::Builtin("freshkube-workloads"), cx)
        })
    });
    cx.run_until_parked();
    frame(cx, handle);
    let network: Vec<String> = vec!["Received by pod".into(), "Sent by pod".into()];
    let answered = ready(cx, &page);
    assert!(answered.contains(&"CPU by pod".to_owned()), "{answered:?}");
    assert!(network.iter().all(|title| !answered.contains(title)));
    let row = cx.read(|cx| {
        let board = page.read(cx).board.as_ref().unwrap();
        let (section, header) = board
            .rows
            .iter()
            .enumerate()
            .find_map(|(index, row)| {
                row.as_ref()
                    .filter(|row| row.title.as_ref() == "Network")
                    .map(|row| (index, row))
            })
            .unwrap();
        assert!(board.is_collapsed(section));
        assert_eq!(header.count.as_ref(), "2 panels");
        header.id.clone()
    });
    cx.update_window(handle, |_, window, cx| window.click(row, cx))
        .unwrap();
    cx.run_until_parked();
    frame(cx, handle);
    let answered = ready(cx, &page);
    assert!(
        network.iter().all(|title| answered.contains(title)),
        "{answered:?}"
    );
}

#[gpui_kit::test]
fn a_file_that_is_not_a_dashboard_shows_why(cx: &mut TestAppContext) {
    let folder = std::env::temp_dir().join(format!(
        "freshkube-monitoring-folder-{}",
        std::process::id()
    ));
    _ = std::fs::remove_dir_all(&folder);
    std::fs::create_dir_all(&folder).unwrap();
    std::fs::write(folder.join("broken.json"), "{ not json").unwrap();
    let (_runtime, handle, page) = mount(cx, Some(example_source()));
    cx.update(|cx| {
        page.update(cx, |page, cx| {
            page.saved.folder = Some(folder.clone());
            page.read_folder(cx);
        })
    });
    cx.run_until_parked();
    let id = cx.read(|cx| match &page.read(cx).catalog.folder {
        FolderState::Read { entries, .. } => entries[0].id.clone(),
        _ => panic!("the folder wasn't read"),
    });
    show(cx, handle, &page);
    cx.update(|cx| page.update(cx, |page, cx| page.open(id, cx)));
    cx.run_until_parked();
    frame(cx, handle);
    cx.read(|cx| {
        let board = page.read(cx).board.as_ref().unwrap();
        assert!(board.error.is_some());
        assert!(board.slots.is_empty());
    });
    assert!(!shown(cx, handle, "monitoring-grid"));
    assert!(!shown(cx, handle, "monitoring-range"));
    std::fs::remove_dir_all(&folder).unwrap();
}

/// The labels of the markers a chart draws.
fn chart_markers(
    cx: &mut TestAppContext,
    page: &Entity<MonitoringPage>,
    title: &str,
) -> Vec<String> {
    cx.read(|cx| {
        let board = page.read(cx).board.as_ref().unwrap();
        let slot = board
            .slots
            .iter()
            .find(|slot| slot.spec.title == title)
            .unwrap();
        slot.view
            .read(cx)
            .marker_labels()
            .into_iter()
            .map(|label| label.to_string())
            .collect()
    })
}

fn choose(cx: &mut TestAppContext, page: &Entity<MonitoringPage>, variable: &str, value: &str) {
    let index = cx.read(|cx| {
        let board = page.read(cx).board.as_ref().unwrap();
        let id = format!("monitoring-variable-{variable}");
        let control = board
            .controls
            .iter()
            .find(|control| control.id.as_ref() == id)
            .unwrap();
        assert!(
            control
                .options
                .iter()
                .any(|option| option.as_ref() == value)
        );
        control.index
    });
    cx.update(|cx| page.update(cx, |page, cx| page.set_variable(index, value.into(), cx)));
    cx.run_until_parked();
}

const CPU: &str = "CPU usage by node";

#[gpui_kit::test]
fn every_chart_draws_the_example_deploys_and_node_events(cx: &mut TestAppContext) {
    let (_runtime, handle, page) = mount(cx, Some(example_source()));
    show(cx, handle, &page);
    assert!(shown(cx, handle, "monitoring-markers-deploys"));
    assert!(shown(cx, handle, "monitoring-markers-nodes"));
    assert!(!shown(cx, handle, "monitoring-markers-unavailable"));
    let markers = chart_markers(cx, &page, CPU);
    assert_eq!(
        markers,
        [
            "cp-2 rebooted",
            "cp-2 Ready",
            "deploy/coredns → 1.11.3",
            "deploy/checkout → revision 14",
            "deploy/kube-state-metrics → 2.13.0",
            "deploy/api → 1.8.2",
            "worker-2 NotReady",
        ]
    );
    // Every timeseries draws them; a stat has no time axis to draw them on.
    assert_eq!(chart_markers(cx, &page, "Memory usage by node"), markers);
    assert!(chart_markers(cx, &page, "Ready nodes").is_empty());

    // A shorter range keeps those within it.
    cx.update(|cx| page.update(cx, |page, cx| page.set_range(3600, cx)));
    cx.run_until_parked();
    assert_eq!(
        chart_markers(cx, &page, CPU),
        ["deploy/api → 1.8.2", "worker-2 NotReady"]
    );
}

#[gpui_kit::test]
fn the_annotation_toggles_hide_and_show_each_kind(cx: &mut TestAppContext) {
    let (_runtime, handle, page) = mount(cx, Some(example_source()));
    show(cx, handle, &page);
    let click = |cx: &mut TestAppContext, id: &'static str| {
        cx.update_window(handle, |_, window, cx| {
            window.render_frame(cx);
            window.click(id, cx);
        })
        .unwrap();
        cx.run_until_parked();
    };
    click(cx, "monitoring-markers-deploys");
    let markers = chart_markers(cx, &page, CPU);
    assert_eq!(
        markers,
        ["cp-2 rebooted", "cp-2 Ready", "worker-2 NotReady"]
    );
    click(cx, "monitoring-markers-nodes");
    assert!(chart_markers(cx, &page, CPU).is_empty());
    click(cx, "monitoring-markers-deploys");
    click(cx, "monitoring-markers-nodes");
    assert_eq!(chart_markers(cx, &page, CPU).len(), 7);

    // A refresh keeps the choice.
    click(cx, "monitoring-markers-deploys");
    cx.update(|cx| page.update(cx, |page, cx| page.refresh(cx)));
    cx.run_until_parked();
    assert_eq!(chart_markers(cx, &page, CPU).len(), 3);
}

#[gpui_kit::test]
fn markers_follow_the_namespace_and_node_variables(cx: &mut TestAppContext) {
    let (_runtime, handle, page) = mount(cx, Some(example_source()));
    show(cx, handle, &page);
    choose(cx, &page, "namespace", "payments");
    assert_eq!(
        chart_markers(cx, &page, CPU),
        [
            "cp-2 rebooted",
            "cp-2 Ready",
            "deploy/checkout → revision 14",
            "deploy/api → 1.8.2",
            "worker-2 NotReady",
        ]
    );
    choose(cx, &page, "node", "worker-2");
    assert_eq!(
        chart_markers(cx, &page, CPU),
        [
            "deploy/checkout → revision 14",
            "deploy/api → 1.8.2",
            "worker-2 NotReady",
        ]
    );
    choose(cx, &page, "namespace", "All");
    choose(cx, &page, "node", "All");
    assert_eq!(chart_markers(cx, &page, CPU).len(), 7);
}

#[gpui_kit::test]
fn another_context_forgets_the_markers_read(cx: &mut TestAppContext) {
    let (_runtime, handle, page) = mount(cx, Some(example_source()));
    show(cx, handle, &page);
    assert_eq!(chart_markers(cx, &page, CPU).len(), 7);
    cx.update(|cx| {
        page.update(cx, |page, cx| {
            page.set_source(None, cx);
            assert!(page.markers.all.is_empty());
            assert!(page.markers.shown.is_empty());
        })
    });
}

#[gpui_kit::test]
fn history_reads_where_the_page_found_or_remembers_prometheus(cx: &mut TestAppContext) {
    use crate::monitoring::history::HistoryKind;
    let (_runtime, _handle, page) = mount(cx, Some(example_source()));
    let kind = |cx: &mut TestAppContext| {
        cx.read(|cx| {
            page.read(cx).history().map(|history| match history.kind {
                HistoryKind::Example => "example".to_owned(),
                HistoryKind::Ready(prometheus) => {
                    format!("ready {}", prometheus.endpoint().label())
                }
                HistoryKind::Remembered { service, .. } => {
                    format!("remembered {}", service.label())
                }
            })
        })
    };
    // Example data answers without the page ever showing.
    assert_eq!(kind(cx).as_deref(), Some("example"));

    // A live context with nothing remembered has none until the page finds one.
    let live = KubeSource {
        id: "live".into(),
        context: "prod-ams".into(),
        access: KubeAccess::Direct(crate::resources::direct::DirectAccess::new(
            Vec::new(),
            "prod-ams".into(),
            freshkube_core::ConfigurationRevision::default(),
        )),
    };
    cx.update(|cx| page.update(cx, |page, cx| page.set_source(Some(live.clone()), cx)));
    assert_eq!(kind(cx), None);

    let service = PrometheusService::new("monitoring", "prometheus-operated", 9090);
    cx.update(|cx| {
        page.update(cx, |page, cx| {
            page.saved
                .services
                .insert("prod-ams".into(), service.clone());
            page.set_source(None, cx);
            page.set_source(Some(live), cx);
        })
    });
    assert_eq!(
        kind(cx).as_deref(),
        Some("remembered monitoring/prometheus-operated:9090")
    );

    // Looked for and not found: none, so the panes show nothing.
    let missing = Discovery::Missing {
        candidates: Vec::new(),
        tried: Vec::new(),
    };
    cx.update(|cx| page.update(cx, |page, cx| page.discovered(Ok(missing), cx)));
    assert_eq!(kind(cx), None);
}

fn live_source() -> KubeSource {
    KubeSource {
        id: "live".into(),
        context: "prod-ams".into(),
        access: KubeAccess::Direct(crate::resources::direct::DirectAccess::new(
            Vec::new(),
            "prod-ams".into(),
            freshkube_core::ConfigurationRevision::default(),
        )),
    }
}

/// A Prometheus API on loopback that wants `Bearer s3cret` and answers
/// only the confirming query.
fn metrics_server(runtime: &tokio::runtime::Runtime) -> String {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let listener = runtime
        .block_on(tokio::net::TcpListener::bind("127.0.0.1:0"))
        .unwrap();
    let address = listener.local_addr().unwrap();
    runtime.spawn(async move {
        while let Ok((mut socket, _)) = listener.accept().await {
            tokio::spawn(async move {
                let mut buffer = vec![0; 16 * 1024];
                let mut read = 0;
                while !buffer[..read].windows(4).any(|w| w == b"\r\n\r\n") {
                    match socket.read(&mut buffer[read..]).await {
                        Ok(0) | Err(_) => return,
                        Ok(n) => read += n,
                    }
                }
                let head = String::from_utf8_lossy(&buffer[..read]).to_lowercase();
                let target = head.split_whitespace().nth(1).unwrap_or("").to_owned();
                let (status, body) = if !head.contains("authorization: bearer s3cret") {
                    (401, "unauthorized".to_owned())
                } else if target == "/vm/api/v1/query?query=1" {
                    (
                        200,
                        r#"{"status":"success","data":{"resultType":"scalar","result":[1,"1"]}}"#
                            .to_owned(),
                    )
                } else {
                    (404, "not found".to_owned())
                };
                let answer = format!(
                    "HTTP/1.1 {status} X\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}",
                    body.len()
                );
                let _ = socket.write_all(answer.as_bytes()).await;
            });
        }
    });
    format!("http://{address}/vm")
}

fn test_said(cx: &mut TestAppContext, page: &Entity<MonitoringPage>) -> Option<String> {
    cx.read(|cx| match &page.read(cx).form.as_ref()?.test {
        source::Test::Passed(message) => Some(format!("passed: {message}")),
        source::Test::Failed(message) => Some(format!("failed: {message}")),
        _ => None,
    })
}

async fn tested(cx: &mut TestAppContext, handle: AnyWindowHandle, page: &Entity<MonitoringPage>) {
    let observed = page.clone();
    cx.wait_for(handle, Duration::from_secs(5), move |_, cx| {
        !matches!(
            observed.read(cx).form.as_ref().map(|form| &form.test),
            Some(source::Test::Testing(_))
        )
    })
    .await;
}

async fn settled(cx: &mut TestAppContext, handle: AnyWindowHandle, page: &Entity<MonitoringPage>) {
    let observed = page.clone();
    cx.wait_for(handle, Duration::from_secs(5), move |_, cx| {
        !matches!(
            observed.read(cx).connection,
            Connection::Looking { .. } | Connection::None
        )
    })
    .await;
}

#[gpui_kit::test]
async fn a_url_chosen_in_settings_is_tested_saved_and_read_with_its_token_from_the_store(
    cx: &mut TestAppContext,
) {
    use crate::secrets::{MemoryStore, Secrets};
    cx.executor().allow_parking();
    let directory = std::env::temp_dir().join(format!(
        "freshkube-monitoring-source-{}-{}",
        std::process::id(),
        line!()
    ));
    let _ = std::fs::remove_dir_all(&directory);
    std::fs::create_dir_all(&directory).unwrap();
    let preferences = directory.join("preferences.json");
    let store = std::sync::Arc::new(MemoryStore::default());
    let secrets: Secrets = store.clone();
    let runtime = tokio::runtime::Runtime::new().unwrap();
    let base = metrics_server(&runtime);
    let (handle, page) = mount_with(
        cx,
        &runtime,
        Some(live_source()),
        Some(&preferences),
        Some(secrets.clone()),
    );

    // Test checks the form as typed: a wrong token is refused.
    let fill = |cx: &mut TestAppContext, token: &'static str| {
        let url = base.clone();
        cx.update_window(handle, |_, window, cx| {
            page.update(cx, |page, cx| {
                page.source_form(window, cx);
                page.set_mode(source::Mode::Url, cx);
                let form = page.form.as_ref().unwrap();
                form.url
                    .update(cx, |input, cx| input.set_value(url, window, cx));
                form.token
                    .update(cx, |input, cx| input.set_value(token, window, cx));
                page.test_source(cx);
            })
        })
        .unwrap();
    };
    fill(cx, "wrong");
    tested(cx, handle, &page).await;
    let said = test_said(cx, &page).unwrap();
    assert!(said.starts_with("failed: The server refused"), "{said}");
    assert!(!said.contains("wrong"), "{said}");
    fill(cx, "s3cret");
    tested(cx, handle, &page).await;
    assert_eq!(
        test_said(cx, &page).unwrap(),
        format!("passed: Prometheus API answered at {base}")
    );
    // Testing saved nothing.
    assert!(store.keys.lock().unwrap().is_empty());
    cx.read(|cx| assert!(page.read(cx).saved.choices.is_empty()));

    // Save keeps the URL in monitoring.json and the token in the store,
    // then the page reads from it.
    cx.update(|cx| page.update(cx, |page, cx| page.set_visible(true, cx)));
    cx.update_window(handle, |_, window, cx| {
        page.update(cx, |page, cx| page.save_source(window, cx))
    })
    .unwrap();
    settled(cx, handle, &page).await;
    let url = format!("{base}/");
    cx.read(|cx| {
        let page = page.read(cx);
        assert_eq!(page.connection_name(), "ready");
        let history = page.history().unwrap();
        assert!(matches!(
            history.kind,
            crate::monitoring::history::HistoryKind::Ready(ref prometheus)
                if prometheus.endpoint().label() == base
        ));
    });
    let account = crate::monitoring::store::token_account(&url);
    cx.wait_for(handle, Duration::from_secs(5), {
        let store = store.clone();
        let account = account.clone();
        move |_, _| store.keys.lock().unwrap().contains_key(&account)
    })
    .await;
    assert_eq!(
        store.keys.lock().unwrap().get(&account).map(String::as_str),
        Some("s3cret")
    );
    let file = directory.join("monitoring.json");
    cx.wait_for(handle, Duration::from_secs(5), {
        let file = file.clone();
        move |_, _| std::fs::read_to_string(&file).is_ok_and(|text| text.contains("sources"))
    })
    .await;
    let written = std::fs::read_to_string(&file).unwrap();
    assert!(written.contains(&url), "{written}");
    assert!(!written.contains("s3cret"), "{written}");

    // The next launch reads the token from the store.
    let (relaunched_handle, relaunched) = mount_with(
        cx,
        &runtime,
        Some(live_source()),
        Some(&preferences),
        Some(secrets),
    );
    cx.update(|cx| relaunched.update(cx, |page, cx| page.set_visible(true, cx)));
    settled(cx, relaunched_handle, &relaunched).await;
    cx.read(|cx| assert_eq!(relaunched.read(cx).connection_name(), "ready"));

    // Forget drops the token, and the server refuses the page.
    cx.update_window(relaunched_handle, |_, window, cx| {
        relaunched.update(cx, |page, cx| {
            page.source_form(window, cx);
            page.forget_token(window, cx);
        })
    })
    .unwrap();
    settled(cx, relaunched_handle, &relaunched).await;
    cx.read(|cx| assert_eq!(relaunched.read(cx).connection_name(), "refused"));
    cx.wait_for(relaunched_handle, Duration::from_secs(5), {
        let store = store.clone();
        move |_, _| store.keys.lock().unwrap().is_empty()
    })
    .await;
    assert!(store.keys.lock().unwrap().is_empty());
    std::fs::remove_dir_all(&directory).unwrap();
}

#[gpui_kit::test]
fn the_form_says_why_it_cannot_be_saved(cx: &mut TestAppContext) {
    let (_runtime, handle, page) = mount(cx, Some(live_source()));
    cx.update_window(handle, |_, window, cx| {
        page.update(cx, |page, cx| {
            page.source_form(window, cx);
            page.set_mode(source::Mode::Service, cx);
            page.form
                .as_ref()
                .unwrap()
                .namespace
                .update(cx, |input, cx| input.set_value("vm", window, cx));
            page.save_source(window, cx);
            assert!(page.saved.choices.is_empty());
            page.set_mode(source::Mode::Url, cx);
            page.form.as_ref().unwrap().url.update(cx, |input, cx| {
                input.set_value("https://u:p@vm.lan", window, cx)
            });
            page.save_source(window, cx);
            assert!(page.saved.choices.is_empty());
        })
    })
    .unwrap();
    // A chosen Service is the only one tried, and history reads it.
    let service = PrometheusService::new("vm", "vmselect", 8481).with_path("select/0/prometheus");
    cx.update(|cx| {
        page.update(cx, |page, cx| {
            page.saved.choices.insert(
                "prod-ams".into(),
                crate::monitoring::store::Choice::Service(service.clone()),
            );
            page.saved.services.insert(
                "prod-ams".into(),
                PrometheusService::new("monitoring", "prometheus-operated", 9090),
            );
            page.set_source(None, cx);
            page.set_source(Some(live_source()), cx);
        })
    });
    cx.read(|cx| {
        assert!(matches!(
            page.read(cx).history().unwrap().kind,
            crate::monitoring::history::HistoryKind::Remembered { service: ref s, .. } if *s == service
        ));
    });
}
