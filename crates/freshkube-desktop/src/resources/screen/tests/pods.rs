use super::*;

#[gpui_kit::test]
fn node_pods_are_filtered_across_namespaces_and_do_not_open_a_nested_pane(cx: &mut TestAppContext) {
    let (_runtime, screen, handle) = mount(cx, Some("prod-fra"));
    cx.update_window(handle, |_, window, cx| {
        screen.update(cx, |screen, cx| {
            screen.set_node(Some("talos-wk-fra1-02"), window, cx)
        });
        window.render_frame(cx);
        let read = screen.read(cx);
        assert!(!read.store.is_empty());
        let column = read
            .store
            .columns()
            .iter()
            .position(|column| column.name == "Node")
            .unwrap();
        assert!(
            read.store
                .entries()
                .iter()
                .all(|entry| entry.row().cells[column] == "talos-wk-fra1-02")
        );
        let old_epoch = read.store.epoch();
        let old_rows = read
            .store
            .entries()
            .iter()
            .map(|entry| entry.row().clone())
            .collect::<Vec<_>>();
        assert!(window.try_find("resource-namespace").is_none());
        screen.update(cx, |screen, cx| screen.focus(window, cx));
        window.press("down", cx);
        window.render_frame(cx);
        assert!(shown(&screen, cx).is_none());
        window.press("enter", cx);
        window.render_frame(cx);
        assert!(shown(&screen, cx).is_none());
        screen.update(cx, |screen, cx| {
            screen.set_node(Some("talos-wk-fra1-01"), window, cx);
            screen.apply(
                old_epoch,
                vec![vec![ResourceEvent::reset(
                    screen.store.columns().to_vec(),
                    old_rows,
                )]],
                cx,
            );
        });
        assert!(
            screen
                .read(cx)
                .store
                .entries()
                .iter()
                .all(|entry| entry.row().cells[column] == "talos-wk-fra1-01")
        );
    })
    .unwrap();
}

/// A node's Pods list has no Logs buttons, since L does nothing there
/// (#337), and each of its cells still starts under its column's label.
#[gpui_kit::test]
fn node_pods_have_no_logs_column_and_keep_their_header_aligned(cx: &mut TestAppContext) {
    use super::super::layout::ColumnSource;
    let (_runtime, screen, handle) = mount(cx, Some("prod-fra"));
    cx.update_window(handle, |_, window, cx| {
        screen.update(cx, |screen, cx| {
            screen.set_node(Some("talos-wk-fra1-02"), window, cx)
        });
        window.render_frame(cx);
        let columns = screen.read(cx).layout.columns.clone();
        assert!(
            columns
                .iter()
                .all(|column| column.source != ColumnSource::Logs)
        );
        let first = identity_at(&screen, 0, cx);
        assert!(
            window
                .try_find(gpui_kit::SharedString::from(format!(
                    "pod-row-logs-{}",
                    first.uid
                )))
                .is_none()
        );
        let row = row_id(&first);
        for (source, cell) in [
            (ColumnSource::Containers, "containers"),
            (ColumnSource::Restarts, "restarts"),
        ] {
            let ix = columns
                .iter()
                .position(|column| column.source == source)
                .unwrap();
            let header = window.find(("resource-sort", ix)).bounds().left();
            let left = window.within(row.clone()).find(cell).bounds().left();
            assert!(
                (left - header).abs() <= px(1.5),
                "{cell}: {left:?} under {header:?}"
            );
        }
    })
    .unwrap();
}

/// A group row's buttons follow its count, within the 280 dp a drawer
/// leaves the list, and the Healthy row offers no fold while a filter keeps
/// every healthy pod in sight (#321).
#[gpui_kit::test]
fn a_group_rows_buttons_follow_its_count_and_a_filter_offers_no_fold(cx: &mut TestAppContext) {
    let (_runtime, screen, handle) = mount(cx, Some("homelab"));
    cx.update_window(handle, |_, window, cx| {
        problems(&screen, cx);
        window.render_frame(cx);
        let row = window.find("resource-group-healthy").bounds();
        let toggle = window.find("resource-group-healthy-toggle").bounds();
        assert!(
            toggle.right() - row.left() <= gpui_kit::px(280.),
            "{row:?} {toggle:?}"
        );

        // The healthy chip shows every healthy pod: nothing to fold.
        window.click("resource-tally-healthy", cx);
        window.render_frame(cx);
        assert_eq!(screen.read(cx).projection.len(), 20);
        assert!(window.try_find("resource-group-healthy-toggle").is_none());
        window.click("resource-tally-healthy", cx);
        window.render_frame(cx);
        assert!(window.try_find("resource-group-healthy-toggle").is_some());

        // Neither does a typed filter, which reaches the screen through
        // the input's change event once the update ends.
        window.click("resource-filter", cx);
        window.render_frame(cx);
        window.input("coredns", cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert_eq!(screen.read(cx).projection.len(), 2);
        assert!(window.find("resource-group-healthy").visible());
        assert!(window.try_find("resource-group-healthy-toggle").is_none());
    })
    .unwrap();
}

#[gpui_kit::test]
fn pods_show_problems_first_and_fold_healthy_ones(cx: &mut TestAppContext) {
    let (_runtime, screen, handle) = mount(cx, Some("homelab"));
    cx.update_window(handle, |_, window, cx| {
        problems(&screen, cx);
        window.render_frame(cx);
        // One pod crashes and one waits to start; the rest are folded.
        let tally = screen.read(cx).projection.tally();
        assert_eq!((tally.failing, tally.waiting, tally.total()), (1, 1, 22));
        assert_eq!(tally.problems(), 2);
        assert_eq!(screen.read(cx).projection.len(), 2);
        assert!(window.find("resource-group-failing").visible());
        assert!(window.find("resource-group-pending").visible());
        let healthy = window
            .find("resource-group-healthy")
            .label()
            .unwrap()
            .to_owned();
        assert!(healthy.starts_with("Healthy · 20 pods · "), "{healthy}");
        assert!(healthy.ends_with("· collapsed"), "{healthy}");
        // The count sits in the table's footer, under the rows.
        let collapsed = window.within("resource-footer").find("resource-collapsed");
        assert!(collapsed.visible());
        assert!(collapsed.bounds().top() >= window.find("resource-table-scroll").bounds().bottom());
        assert_eq!(
            window.find("resource-tally-failing").label(),
            Some("1 failing")
        );
        // The crashing pod says why after its name.
        let failing = screen.read(cx).projection.row(&screen.read(cx).store, 0);
        let failing = failing.unwrap().identity.clone();
        assert_eq!(
            screen
                .read(cx)
                .store
                .get(&failing)
                .unwrap()
                .pod
                .as_ref()
                .unwrap()
                .reason,
            "CrashLoopBackOff · exit 1"
        );

        window.click("resource-group-healthy-toggle", cx);
        window.render_frame(cx);
        assert_eq!(screen.read(cx).projection.len(), 22);
        assert!(window.try_find("resource-collapsed").is_none());
        window.click("resource-group-healthy-toggle", cx);
        window.render_frame(cx);
        assert_eq!(screen.read(cx).projection.len(), 2);

        // Show all lists every pod in one sorted list.
        window.click("resource-show-all", cx);
        window.render_frame(cx);
        assert_eq!(screen.read(cx).list_view, ListView::All);
        assert_eq!(screen.read(cx).projection.len(), 22);
        assert!(window.try_find("resource-group-failing").is_none());
        window.click("resource-view-problems", cx);
        window.render_frame(cx);
        assert_eq!(screen.read(cx).projection.len(), 2);
    })
    .unwrap();
}

/// A pod's glyph by its printed status, on a ready node.
fn pod_glyph(status: &str) -> crate::ui::Tone {
    let cells = ["p", "0/1", status, "0", "", "10.0.0.1", "wk-1"].map(str::to_owned);
    let pod = crate::resources::rows::pod_row(&example::pod_columns(), &cells, None, false);
    super::super::cells::pod_tone(&pod, true)
}

#[test]
fn a_container_that_ran_and_stopped_died_and_one_that_cannot_start_is_critical() {
    use crate::ui::Tone;
    for status in [
        "CrashLoopBackOff",
        "Init:CrashLoopBackOff",
        "Error",
        "OOMKilled",
    ] {
        assert_eq!(pod_glyph(status), Tone::Died, "{status}");
    }
    for status in ["ImagePullBackOff", "ErrImagePull", "Evicted"] {
        assert_eq!(pod_glyph(status), Tone::Crit, "{status}");
    }
    // An unschedulable pod prints Pending: it waits, as before.
    assert_eq!(pod_glyph("Pending"), Tone::Unknown);
    assert_eq!(pod_glyph("Running"), Tone::Warn);
}

#[gpui_kit::test]
fn a_pod_that_died_counts_with_the_failing_ones(cx: &mut TestAppContext) {
    let (_runtime, screen, handle) = mount(cx, Some("homelab"));
    cx.update_window(handle, |_, window, cx| {
        problems(&screen, cx);
        window.render_frame(cx);
        let view = screen.read(cx);
        let crashing = view.projection.row(&view.store, 0).unwrap();
        let pod = crashing.pod.as_ref().unwrap();
        assert_eq!(pod.status, "CrashLoopBackOff");
        assert_eq!(
            super::super::cells::pod_tone(pod, true),
            crate::ui::Tone::Died
        );
        // The chip and the group count it as failing, under the critical
        // glyph.
        assert_eq!(view.projection.tally().failing, 1);
        assert_eq!(
            window.find("resource-tally-failing").label(),
            Some("1 failing")
        );
        assert!(window.find("resource-group-failing").visible());
    })
    .unwrap();
}

#[gpui_kit::test]
fn opening_a_folded_pod_unfolds_the_healthy_ones(cx: &mut TestAppContext) {
    let (_runtime, screen, handle) = mount(cx, Some("homelab"));
    cx.update_window(handle, |_, window, cx| {
        let healthy = {
            let view = screen.read(cx);
            view.store
                .entries()
                .iter()
                .map(|entry| entry.row())
                .find(|row| row.pod.as_ref().unwrap().reason.is_empty())
                .unwrap()
                .identity
                .clone()
        };
        problems(&screen, cx);
        screen.update(cx, |screen, cx| {
            screen.open_identity_on(healthy.clone(), crate::resources::Tab::Overview, window, cx)
        });
        window.render_frame(cx);
        assert_eq!(selected(&screen, cx), Some(healthy.clone()));
        assert!(window.find(row_id(&healthy)).visible());
    })
    .unwrap();
}

#[gpui_kit::test]
fn pods_on_a_node_that_isnt_ready_group_under_it(cx: &mut TestAppContext) {
    let (_runtime, screen, handle) = mount(cx, Some("homelab"));
    let links = Rc::new(RefCell::new(Vec::new()));
    let sink = links.clone();
    cx.update(|cx| {
        cx.subscribe(
            &screen,
            move |_, link: &crate::resources::ResourceLink, _| {
                if let crate::resources::ResourceLink::Node(node, tab) = link {
                    sink.borrow_mut().push((node.clone(), *tab));
                }
            },
        )
        .detach()
    });
    cx.update_window(handle, |_, window, cx| {
        screen.update(cx, |screen, cx| {
            screen.not_ready = [("talos-home".to_owned(), None)].into();
            screen.set_list_view(ListView::Problems, cx);
        });
        window.render_frame(cx);
        let group = window
            .find("resource-group-node:talos-home")
            .label()
            .unwrap()
            .to_owned();
        assert!(group.starts_with("Node not ready · "), "{group}");
        assert!(group.contains("metrics are last known"), "{group}");
        // The crashing pod stays failing; the waiting ones and the
        // running ones can't be confirmed.
        let tally = screen.read(cx).projection.tally();
        assert_eq!(tally.failing, 1);
        assert!(tally.warning > 10, "{tally:?}");
    })
    .unwrap();
    // The group's menu offers its node, as O does.
    choose(
        cx,
        handle,
        |window, cx| window.right_click("resource-group-node:talos-home-line", cx),
        "Open node talos-home",
    );
    assert_eq!(
        links.borrow().as_slice(),
        [(
            "talos-home".to_owned(),
            crate::desktop::nodes::NodeTab::Overview
        )]
    );
}

/// Ready sorts the least ready pods first and Restarts the most restarted
/// last; each pod's restarts have a cell of their own (#291).
#[gpui_kit::test]
fn ready_and_restarts_sort_on_their_own(cx: &mut TestAppContext) {
    use crate::resources::{model::SortKey, rows::PodRow};
    let (_runtime, screen, handle) = mount(cx, Some("homelab"));
    cx.update_window(handle, |_, window, cx| {
        screen.update(cx, |screen, cx| screen.set_list_view(ListView::All, cx));
        window.render_frame(cx);
        let column =
            |screen: &Entity<ResourcesScreen>, cx: &gpui_kit::App, pick: fn(&PodRow) -> f64| {
                let view = screen.read(cx);
                (0..view.projection.len())
                    .map(|ix| {
                        pick(
                            view.projection
                                .row(&view.store, ix)
                                .unwrap()
                                .pod
                                .as_ref()
                                .unwrap(),
                        )
                    })
                    .collect::<Vec<f64>>()
            };
        let share = |pod: &PodRow| {
            let (up, all) = pod.ready.split_once('/').unwrap();
            up.parse::<f64>().unwrap() / all.parse::<f64>().unwrap()
        };
        screen.update(cx, |screen, cx| screen.sort_by(SortKey::Ready, cx));
        let ready = column(&screen, cx, share);
        assert!(ready.is_sorted(), "{ready:?}");
        assert!(ready[0] < 1., "{ready:?}");

        screen.update(cx, |screen, cx| screen.sort_by(SortKey::Restarts, cx));
        let restarts = column(&screen, cx, |pod| pod.restarts as f64);
        assert!(restarts.is_sorted(), "{restarts:?}");
        assert!(*restarts.last().unwrap() > 0., "{restarts:?}");

        // Descending, the most restarted pod leads, its count in its cell.
        screen.update(cx, |screen, cx| screen.sort_by(SortKey::Restarts, cx));
        window.render_frame(cx);
        let view = screen.read(cx);
        let first = view.projection.row(&view.store, 0).unwrap();
        let (restarted, count) = (first.identity.clone(), first.pod.as_ref().unwrap().restarts);
        assert_eq!(count as f64, *restarts.last().unwrap());
        let cell = window.within(row_id(&restarted)).find("restarts");
        assert_eq!(cell.label(), Some(format!("{count} restarts").as_str()));
    })
    .unwrap();
}

/// Containers sorts the pod with the worst container first, and each
/// row's squares name every container, init containers first (#291).
#[gpui_kit::test]
fn containers_sort_the_worst_first_and_name_each_container(cx: &mut TestAppContext) {
    use crate::resources::model::SortKey;
    let (_runtime, screen, handle) = mount(cx, Some("homelab"));
    cx.update_window(handle, |_, window, cx| {
        screen.update(cx, |screen, cx| {
            screen.set_list_view(ListView::All, cx);
            screen.sort_by(SortKey::Containers, cx);
        });
        window.render_frame(cx);
        let view = screen.read(cx);
        let rows: Vec<_> = (0..view.projection.len())
            .map(|ix| view.projection.row(&view.store, ix).unwrap())
            .collect();
        let worst: Vec<u8> = rows
            .iter()
            .map(|row| row.pod.as_ref().unwrap().worst)
            .collect();
        assert!(worst.is_sorted(), "{worst:?}");
        assert_eq!(worst[0], 0, "a failing container leads");
        let first = rows[0].identity.clone();
        let pod = rows[0].pod.as_ref().unwrap();
        let cell = window.within(row_id(&first)).find("containers");
        assert_eq!(cell.label(), Some(pod.containers_label.as_str()));
        for container in &pod.containers {
            assert!(
                pod.containers_label.contains(&container.name),
                "{}",
                pod.containers_label
            );
        }
        // Ledger migrates first: its init container leads its squares,
        // dimmed, and its line.
        let ledger = rows
            .iter()
            .find(|row| row.identity.name.starts_with("ledger-"))
            .unwrap()
            .pod
            .as_ref()
            .unwrap();
        assert!(ledger.squares[0].dim);
        assert!(
            ledger
                .containers_label
                .starts_with("migrate (init): exited 0"),
            "{}",
            ledger.containers_label
        );
    })
    .unwrap();
}

/// A pod with more containers than the column draws counts the rest
/// whole, inside the cell, at the default and the largest text size
/// (#329 review).
#[gpui_kit::test]
fn a_crowded_pods_count_fits_its_cell(cx: &mut TestAppContext) {
    let (_runtime, screen, handle) = mount(cx, Some("homelab"));
    for size in [freshkube_ui::ui::BASE_TEXT, 20.] {
        cx.update_window(handle, |_, window, cx| {
            window.set_rem_size(gpui_kit::px(size));
            screen.update(cx, |screen, cx| {
                screen.set_list_view(ListView::All, cx);
                screen.set_filter("gateway", window, cx);
            });
            window.render_frame(cx);
            let gateway = identity_at(&screen, 0, cx);
            assert!(gateway.name.starts_with("gateway-"), "{gateway:?}");
            let view = screen.read(cx);
            let pod = view
                .projection
                .row(&view.store, 0)
                .unwrap()
                .pod
                .clone()
                .unwrap();
            assert_eq!(pod.squares.len(), 10);
            assert_eq!(pod.squares.more().map(|more| more.as_ref()), Some("+2"));
            let row = window.within(row_id(&gateway));
            let cell = row.find("containers").bounds();
            let more = row.find("squares-more");
            let pad = freshkube_ui::ui::dp_px(freshkube_ui::table::CELL_PAD, window);
            assert!(
                more.bounds().right() <= cell.right() - pad + gpui_kit::px(0.5),
                "at {size}: {:?} in {cell:?}",
                more.bounds()
            );
        })
        .unwrap();
    }
}

#[gpui_kit::test]
fn pod_rows_show_owner_readiness_use_and_node(cx: &mut TestAppContext) {
    let (_runtime, screen, handle) = mount(cx, Some("homelab"));
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        let view = screen.read(cx);
        // Example use arrives with the rows, so the first frame has it.
        assert_eq!(view.usage_state, super::super::UsageState::Known);
        let running = view
            .store
            .entries()
            .iter()
            .find(|entry| entry.usage().is_some())
            .unwrap()
            .row()
            .identity
            .clone();
        let row = view.store.get(&running).unwrap();
        assert_eq!(row.owner.as_ref().unwrap().short, "deploy");
        let label = window.find(row_id(&running)).label().unwrap().to_owned();
        assert!(label.contains(" · deploy/"), "{label}");
        // Use sorts by value with unknown use below any known: busiest
        // first, then the pods with no use.
        let pod = row.pod.clone().unwrap();
        assert!(!pod.node.is_empty() && !pod.containers.is_empty());
        screen.update(cx, |screen, cx| {
            screen.sort_by(crate::resources::model::SortKey::Cpu, cx);
            screen.sort_by(crate::resources::model::SortKey::Cpu, cx);
        });
        let view = screen.read(cx);
        let cpu: Vec<Option<f64>> = (0..view.projection.len())
            .map(|ix| {
                view.projection
                    .entry(&view.store, ix)
                    .unwrap()
                    .usage()
                    .and_then(|usage| usage.cpu_millis)
            })
            .collect();
        let known = cpu.iter().take_while(|cpu| cpu.is_some()).count();
        assert!(
            known > 0 && cpu[known..].iter().all(Option::is_none),
            "{cpu:?}"
        );
        assert!(
            cpu[..known].windows(2).all(|pair| pair[0] >= pair[1]),
            "{cpu:?}"
        );
    })
    .unwrap();
}

#[gpui_kit::test]
fn workloads_take_their_glyph_from_replicas_ready(cx: &mut TestAppContext) {
    let (_runtime, screen, handle) = mount(cx, Some("homelab"));
    cx.update_window(handle, |_, window, cx| {
        screen.update(cx, |screen, cx| {
            screen.set_kind(kind("deployments.apps"), window, cx)
        });
        window.render_frame(cx);
        let layout = &screen.read(cx).layout;
        assert_eq!(
            layout.columns[0].source,
            super::super::layout::ColumnSource::Glyph
        );
        assert!(matches!(
            layout.tone_from,
            Some(super::super::layout::ToneSource::Ready(_))
        ));
        assert!(screen.read(cx).projection.grouping().is_none());
        assert!(window.try_find("resource-view").is_none());
    })
    .unwrap();
}

/// Opens a context menu with `press` and picks the item labelled `label`.
fn choose(
    cx: &mut TestAppContext,
    handle: AnyWindowHandle,
    press: impl FnOnce(&mut gpui_kit::Window, &mut gpui_kit::App),
    label: &str,
) {
    let items = open_menu(cx, handle, press);
    let ix = items
        .iter()
        .position(|item| item.as_deref() == Some(label))
        .unwrap_or_else(|| panic!("no {label:?} in {items:?}"));
    cx.update_window(handle, |_, window, cx| {
        window.within("popup-menu").click(ix, cx);
    })
    .unwrap();
    cx.run_until_parked();
}
