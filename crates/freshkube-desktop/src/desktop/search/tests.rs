use super::Part;
use crate::{
    desktop::{Page, tests::fixture},
    resources::{Tab, example},
};
use gpui_kit::{AppContext, TestAppContext, component::WindowExt, test::TestWindowExt};

#[gpui_kit::test]
fn access_replacement_invalidates_an_open_search_and_its_late_answers(cx: &mut TestAppContext) {
    let (_runtime, handle, pilot) = fixture(cx, 1500., 1050.);
    cx.update_window(handle, |_, window, cx| {
        pilot.update(cx, |pilot, cx| pilot.open_search(window, cx));
        let search = pilot.read(cx).search.clone();
        let sequence = search.read(cx).sequence;
        pilot.update(cx, |pilot, cx| pilot.invalidate_target(window, cx));
        search.update(cx, |search, cx| {
            // An answer already queued before cancellation cannot repopulate
            // this generation, even with the same displayed context name.
            search.answer(sequence, "pods", Err("obsolete session".into()), cx);
            assert!(search.parts.get("pods").is_none_or(Result::is_ok));
            assert_ne!(sequence, search.sequence);
        });
    })
    .unwrap();
}

#[gpui_kit::test]
fn search_opens_a_page_node_pod_and_secret_by_name(cx: &mut TestAppContext) {
    let (_runtime, handle, pilot) = fixture(cx, 1500., 1050.);
    let pod = example::read("prod-fra", "pods", None, chrono::Utc::now().timestamp())
        .unwrap()
        .1[0]
        .identity
        .clone();
    let secret = example::read("prod-fra", "secrets", None, chrono::Utc::now().timestamp())
        .unwrap()
        .1[0]
        .identity
        .clone();
    for query in [
        "Security".to_owned(),
        "talos-wk-fra1-02".into(),
        format!("{}/{}", pod.namespace, pod.name),
        format!("{}/{}", secret.namespace, secret.name),
    ] {
        cx.update_window(handle, |_, window, cx| {
            window.render_frame(cx);
            window.click("search-everything", cx);
        })
        .unwrap();
        cx.run_until_parked();
        cx.update_window(handle, |_, window, cx| {
            window.render_frame(cx);
            assert!(window.has_active_dialog(cx));
            window.input(&query, cx);
        })
        .unwrap();
        cx.run_until_parked();
        cx.update_window(handle, |_, window, cx| {
            window.render_frame(cx);
            window.press("enter", cx);
        })
        .unwrap();
        cx.run_until_parked();
        cx.update_window(handle, |_, window, cx| {
            window.render_frame(cx);
            assert!(!window.has_active_dialog(cx));
            let view = pilot.read(cx);
            if query == "Security" {
                assert_eq!(view.page, Page::Security);
            } else if query == "talos-wk-fra1-02" {
                assert_eq!(view.page, Page::Nodes);
                assert_eq!(
                    view.node_workspace
                        .selected
                        .as_ref()
                        .unwrap()
                        .kubernetes
                        .as_deref(),
                    Some(query.as_str())
                );
            } else {
                let expected = if query.ends_with(&pod.name) {
                    &pod
                } else {
                    &secret
                };
                assert_eq!(view.page, Page::Resources);
                assert_eq!(view.resources.read(cx).detail_identity(cx), Some(expected));
                assert_eq!(view.resources.read(cx).detail_tab(cx), Tab::Overview);
                assert!(
                    window.find("resource-detail").focused().unwrap_or(false)
                        || window
                            .find("detail-tab-overview")
                            .focused()
                            .unwrap_or(false)
                );
            }
        })
        .unwrap();
    }
}

#[gpui_kit::test]
fn refusal_is_visible_and_other_groups_still_open(cx: &mut TestAppContext) {
    let (_runtime, handle, pilot) = fixture(cx, 1500., 1050.);
    cx.update_window(handle, |_, window, cx| {
        pilot.update(cx, |pilot, cx| pilot.open_search(window, cx))
    })
    .unwrap();
    cx.update(|cx| {
        let search = pilot.read(cx).search.clone();
        search.update(cx, |search, cx| {
            let sequence = search.sequence;
            search.answer(sequence, "secrets", Err("forbidden".into()), cx);
        });
    });
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        let status = window.find("search-status-secrets");
        let label = status.label().unwrap();
        assert!(label.contains("Can't list secrets"));
        assert!(label.contains("forbidden"));
        window.input("Overview", cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window.press("enter", cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.update(|cx| assert_eq!(pilot.read(cx).page, Page::Overview));
}

#[gpui_kit::test]
fn cache_uses_executor_time_expires_and_rejects_closed_answers(cx: &mut TestAppContext) {
    let (_runtime, handle, pilot) = fixture(cx, 1500., 1050.);
    cx.update_window(handle, |_, window, cx| {
        pilot.update(cx, |pilot, cx| pilot.open_search(window, cx))
    })
    .unwrap();
    cx.update(|cx| {
        let search = pilot.read(cx).search.clone();
        search.update(cx, |search, cx| {
            let sequence = search.sequence;
            search.answer(sequence, "secrets", Err("cached refusal".into()), cx);
            search.close(cx);
            search.answer(
                sequence,
                "secrets",
                Ok(Part {
                    entries: Vec::new(),
                    capped: false,
                }),
                cx,
            );
            assert!(search.parts["secrets"].is_err());
        });
    });
    cx.update_window(handle, |_, window, cx| window.close_dialog(cx))
        .unwrap();
    cx.executor()
        .advance_clock(std::time::Duration::from_secs(29));
    cx.update_window(handle, |_, window, cx| {
        pilot.update(cx, |pilot, cx| pilot.open_search(window, cx))
    })
    .unwrap();
    cx.update(|cx| assert!(pilot.read(cx).search.read(cx).parts["secrets"].is_err()));
    cx.update_window(handle, |_, window, cx| window.close_dialog(cx))
        .unwrap();
    cx.run_until_parked();
    cx.executor()
        .advance_clock(std::time::Duration::from_secs(31));
    cx.update_window(handle, |_, window, cx| {
        pilot.update(cx, |pilot, cx| pilot.open_search(window, cx))
    })
    .unwrap();
    cx.update(|cx| assert!(pilot.read(cx).search.read(cx).parts["secrets"].is_ok()));
}

#[gpui_kit::test]
fn results_are_capped_per_group_and_match_namespace_name(cx: &mut TestAppContext) {
    let (_runtime, handle, pilot) = fixture(cx, 1500., 1050.);
    cx.update_window(handle, |_, window, cx| {
        pilot.update(cx, |pilot, cx| pilot.open_search(window, cx))
    })
    .unwrap();
    cx.update(|cx| {
        let search = pilot.read(cx).search.clone();
        search.update(cx, |search, _| {
            search.query = "payments/".into();
            search.rebuild();
            assert!(search.destinations.iter().all(|group| group.len() <= 9));
            assert!(
                search
                    .destinations
                    .iter()
                    .any(|group| group.last().is_some_and(Option::is_none))
            );
            search.query = "a name with no matches".into();
            search.rebuild();
            assert!(search.destinations.is_empty());
        });
    });
}

#[gpui_kit::test]
fn searching_secret_names_never_reads_a_secret_document(cx: &mut TestAppContext) {
    let (_runtime, handle, pilot) = fixture(cx, 1500., 1050.);
    let before = crate::desktop::probe::count("example.secret-document");
    cx.update_window(handle, |_, window, cx| {
        pilot.update(cx, |pilot, cx| pilot.open_search(window, cx))
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window.input("payments/", cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| window.render_frame(cx))
        .unwrap();
    assert_eq!(
        crate::desktop::probe::count("example.secret-document"),
        before
    );
}

#[gpui_kit::test]
fn each_answer_appears_without_waiting_for_other_kinds(cx: &mut TestAppContext) {
    let (_runtime, handle, pilot) = fixture(cx, 1500., 1050.);
    cx.update_window(handle, |_, window, cx| {
        pilot.update(cx, |pilot, cx| pilot.open_search(window, cx))
    })
    .unwrap();
    cx.update(|cx| {
        let search = pilot.read(cx).search.clone();
        search.update(cx, |search, cx| {
            let mut pods = search.parts["pods"].as_ref().unwrap().clone();
            pods.capped = true;
            search.parts.clear();
            search.answer(search.sequence, "pods", Ok(pods), cx);
        });
    });
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert!(
            window
                .find("search-status-pods")
                .label()
                .unwrap()
                .contains("First 2,000 shown")
        );
        assert!(
            window
                .find("search-status-secrets")
                .label()
                .unwrap()
                .contains("Listing secrets")
        );
    })
    .unwrap();
    cx.update(|cx|{
        let search=pilot.read(cx).search.read(cx);assert!(search.destinations.iter().flatten().any(|destination|matches!(destination,Some(super::model::Destination::Object(kind,_)) if kind.is_pod())));
    });
}

#[gpui_kit::test]
fn search_fits_the_minimum_window_at_the_largest_text_size(cx: &mut TestAppContext) {
    let (_runtime, handle, pilot) = fixture(cx, 760., 560.);
    cx.update_window(handle, |_, window, cx| {
        crate::text_size::set(20., cx);
        pilot.update(cx, |pilot, cx| pilot.open_search(window, cx));
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        let bounds = window.find("command").bounds();
        let viewport = window.viewport_size();
        assert!(bounds.left() >= gpui_kit::px(0.));
        assert!(bounds.right() <= viewport.width);
        assert!(bounds.top() >= gpui_kit::px(0.));
        assert!(bounds.bottom() <= viewport.height);
    })
    .unwrap();
}
