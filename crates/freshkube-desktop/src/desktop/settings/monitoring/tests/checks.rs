use super::*;

#[gpui_kit::test]
async fn monitoring_tests_unsaved_values_with_pass_failure_and_unknown(cx: &mut TestAppContext) {
    let directory = tempfile::tempdir().unwrap();
    let file = directory.path().join("freshkube.yaml");
    std::fs::write(&file, SETUP).unwrap();
    let (_runtime, handle, _pilot, page) = live(cx, directory.path(), "cluster-a").await;
    for (verdict, state) in [
        (Verdict::Pass, "Passed"),
        (Verdict::Fail("The store is unreachable".into()), "Failed"),
        (
            Verdict::Unknown("No series match this entry".into()),
            "Unknown",
        ),
    ] {
        let seen = Arc::new(std::sync::Mutex::new(None));
        let observed = seen.clone();
        cx.update_window(handle, |_, window, cx| {
            page.update(cx, |page, _| {
                page.monitoring.as_mut().unwrap().checker = Arc::new(move |resolved, _, _| {
                    *observed.lock().unwrap() = Some(resolved);
                    let verdict = verdict.clone();
                    Box::pin(async move {
                        vec![Check {
                            what: "The store answers",
                            verdict,
                        }]
                    })
                })
            });
            replace(window, cx, "metrics-label", "example_cluster");
        })
        .unwrap();
        cx.run_until_parked();
        cx.update_window(handle, |_, window, cx| {
            window.render_frame(cx);
            window.click("metrics-test", cx);
        })
        .unwrap();
        cx.wait_for_io(handle, |_, cx| {
            page.read(cx).monitoring.as_ref().unwrap().outcome.is_some()
        })
        .await;
        cx.update_window(handle, |_, window, cx| {
            window.render_frame(cx);
            assert!(
                page.read(cx)
                    .monitoring
                    .as_ref()
                    .unwrap()
                    .focus
                    .is_focused(window)
            );
            assert_ne!(window.find("metrics-test").focused(), Some(true));
        })
        .unwrap();
        cx.read(|cx| {
            assert!(
                page.read(cx)
                    .monitoring
                    .as_ref()
                    .unwrap()
                    .outcome
                    .as_ref()
                    .unwrap()
                    .1
                    .starts_with(state)
            )
        });
        assert!(
            matches!(seen.lock().unwrap().as_ref(), Some(Resolved::Central { matcher, .. }) if matcher.label() == "example_cluster" && matcher.value() == "cluster-a")
        );
        assert_eq!(std::fs::read_to_string(&file).unwrap(), SETUP);
    }
}

#[gpui_kit::test]
async fn monitoring_discards_a_check_when_the_draft_changes_or_the_section_hides(
    cx: &mut TestAppContext,
) {
    let directory = tempfile::tempdir().unwrap();
    std::fs::write(directory.path().join("freshkube.yaml"), SETUP).unwrap();
    let (_runtime, handle, pilot, page) = live(cx, directory.path(), "cluster-a").await;
    for hide in [false, true] {
        let (sender, receiver) = tokio::sync::oneshot::channel::<()>();
        let held = Arc::new(std::sync::Mutex::new(Some(receiver)));
        let entered = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let started = entered.clone();
        cx.update_window(handle, |_, window, cx| {
            page.update(cx, |page, _| {
                page.monitoring.as_mut().unwrap().checker = Arc::new(move |_, _, _| {
                    let receiver = held.lock().unwrap().take().unwrap();
                    let started = started.clone();
                    Box::pin(async move {
                        started.store(true, std::sync::atomic::Ordering::SeqCst);
                        _ = receiver.await;
                        vec![Check {
                            what: "The store answers",
                            verdict: Verdict::Pass,
                        }]
                    })
                })
            });
            window.render_frame(cx);
            window.click("metrics-test", cx);
        })
        .unwrap();
        cx.wait_for_io(handle, |_, _| {
            entered.load(std::sync::atomic::Ordering::SeqCst)
        })
        .await;
        cx.update_window(handle, |_, window, cx| {
            if hide {
                pilot.update(cx, |pilot, cx| {
                    pilot.show_settings(SettingsSection::General, window, cx)
                });
            } else {
                replace(window, cx, "metrics-label", "example_cluster");
            }
        })
        .unwrap();
        cx.wait_for_io(handle, |_, _| sender.is_closed()).await;
        cx.read(|cx| {
            let draft = page.read(cx).monitoring.as_ref().unwrap();
            assert!(!draft.read.is_loading());
            assert!(draft.outcome.is_none());
        });
        if !hide {
            cx.update_window(handle, |_, window, cx| {
                window.render_frame(cx);
                window.click("metrics-revert", cx);
            })
            .unwrap();
        }
    }
}
