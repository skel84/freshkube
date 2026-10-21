//! Check entries in the real app: the store's counts become one line per
//! entry, and Accept sets one entry's value.
use super::super::labels::Line;
use super::super::{SettingsPage, SettingsSection};
use super::tests::{live, replace};
use freshkube_core::monitoring::scope::LabelCounts;
use freshkube_ui::real_io::RealIo;
use gpui_kit::{AnyWindowHandle, AppContext, Entity, TestAppContext, test::TestWindowExt};
use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};

const SETUP: &str = "\
version: 1
clusters:
- {id: cluster-a, role: core, context: cluster-a, future_entry: {keep: true}}
- {id: cluster-b, role: environment, context: cluster-b, metrics: {value: shop-b}}
- {id: cluster-c, role: environment, context: cluster-c, metrics: {source: {url: 'https://own.example.test'}}}
metrics:
  mode: central
  store:
    entry: cluster-a
    service: {namespace: metrics, name: prometheus, port: 9090}
future: {keep: [7, null, true]}
";

fn counts(values: &[(&str, u64)]) -> LabelCounts {
    LabelCounts {
        label: "cluster".into(),
        counts: values.iter().map(|(v, n)| ((*v).to_owned(), *n)).collect(),
        capped: false,
    }
}

/// The store answers `answer` to every check, and counts the asks.
fn answering(
    cx: &mut TestAppContext,
    page: &Entity<SettingsPage>,
    answer: Result<LabelCounts, String>,
) -> Arc<AtomicUsize> {
    let asked = Arc::new(AtomicUsize::new(0));
    let seen = asked.clone();
    cx.update(|cx| {
        page.update(cx, |page, _| {
            page.monitoring.as_mut().unwrap().labels.labeler = Arc::new(move |_, _, _| {
                seen.fetch_add(1, Ordering::SeqCst);
                let answer = answer.clone();
                Box::pin(async move { answer })
            })
        })
    });
    asked
}

fn lines(cx: &TestAppContext, page: &Entity<SettingsPage>) -> Vec<(String, String, bool)> {
    cx.read(|cx| {
        page.read(cx)
            .monitoring
            .as_ref()
            .unwrap()
            .labels
            .lines
            .iter()
            .map(|line: &Line| {
                (
                    line.entry.clone(),
                    line.words.to_string(),
                    line.suggest.is_some(),
                )
            })
            .collect()
    })
}

async fn checked(cx: &mut TestAppContext, handle: AnyWindowHandle, page: &Entity<SettingsPage>) {
    cx.wait_for_io(handle, |_, cx| {
        let labels = &page.read(cx).monitoring.as_ref().unwrap().labels;
        !labels.read.is_loading() && (labels.read.data().is_some() || labels.failed.is_some())
    })
    .await;
    cx.run_until_parked();
}

async fn start(
    cx: &mut TestAppContext,
    directory: &std::path::Path,
) -> (
    tokio::runtime::Runtime,
    AnyWindowHandle,
    Entity<crate::desktop::Pilot>,
    Entity<SettingsPage>,
) {
    std::fs::write(directory.join("freshkube.yaml"), SETUP).unwrap();
    live(cx, directory, "cluster-a").await
}

#[gpui_kit::test]
async fn check_entries_lists_each_entry(cx: &mut TestAppContext) {
    let directory = tempfile::tempdir().unwrap();
    let (_runtime, handle, _pilot, page) = start(cx, directory.path()).await;
    let asked = answering(
        cx,
        &page,
        Ok(counts(&[("cluster-a", 3), ("cluster-b", 5), ("legacy", 1)])),
    );
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert!(window.try_find("monitoring-entries-frame").is_none());
        window.click("metrics-check-entries", cx);
    })
    .unwrap();
    checked(cx, handle, &page).await;
    assert_eq!(asked.load(Ordering::SeqCst), 1);
    assert_eq!(
        lines(cx, &page),
        [
            (
                "cluster-a".to_owned(),
                "cluster=\"cluster-a\" has 3 series".to_owned(),
                false
            ),
            (
                "cluster-b".to_owned(),
                "No series now for cluster=\"shop-b\"; the context name cluster-b has series: set metrics.value to cluster-b"
                    .to_owned(),
                true
            ),
            ("cluster-c".to_owned(), "reads its own store".to_owned(), false),
        ]
    );
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        for entry in ["cluster-a", "cluster-b", "cluster-c"] {
            assert!(window.find(format!("metrics-entry-{entry}")).visible());
        }
        assert!(
            window.find("entries-unclaimed").visible()
                || window.try_find("entries-unclaimed").is_none()
        );
    })
    .unwrap();
    cx.read(|cx| {
        let unclaimed = page
            .read(cx)
            .monitoring
            .as_ref()
            .unwrap()
            .labels
            .unclaimed
            .clone();
        assert_eq!(
            unclaimed.unwrap(),
            "Values with series that no entry reads: cluster-b (5), legacy (1)"
        );
    });
}

#[gpui_kit::test]
async fn accept_sets_only_that_entrys_value(cx: &mut TestAppContext) {
    let directory = tempfile::tempdir().unwrap();
    let (_runtime, handle, _pilot, page) = start(cx, directory.path()).await;
    answering(cx, &page, Ok(counts(&[("cluster-a", 3), ("cluster-b", 5)])));
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window.click("metrics-check-entries", cx);
    })
    .unwrap();
    checked(cx, handle, &page).await;
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        // An entry without a suggestion can't be accepted.
        window.click("metrics-entry-cluster-a", cx);
        window.click("metrics-entry-accept", cx);
    })
    .unwrap();
    cx.run_until_parked();
    let before = std::fs::read_to_string(directory.path().join("freshkube.yaml")).unwrap();
    assert_eq!(before, SETUP);
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window.click("metrics-entry-cluster-b", cx);
        window.render_frame(cx);
        window.click("metrics-entry-accept", cx);
    })
    .unwrap();
    cx.wait_for_io(handle, |_, cx| page.read(cx).saving.is_none())
        .await;
    cx.run_until_parked();
    let after = std::fs::read_to_string(directory.path().join("freshkube.yaml")).unwrap();
    let parsed = freshkube_core::setup::parse(after.as_bytes()).unwrap();
    let value = |id: &str| {
        parsed
            .workspace
            .clusters
            .iter()
            .find(|e| e.id == id)
            .and_then(|e| e.metrics.as_deref())
            .and_then(|m| m.value.clone())
    };
    assert_eq!(value("cluster-b").as_deref(), Some("cluster-b"));
    assert_eq!(value("cluster-a"), None);
    assert!(
        after.contains("future_entry") && after.contains("future:"),
        "{after}"
    );
    assert!(after.contains("own.example.test"), "{after}");
    // Derived again from the new setup: cluster-b now has series.
    let derived = lines(cx, &page);
    assert!(
        derived
            .iter()
            .any(|(entry, words, suggest)| entry == "cluster-b"
                && words == "cluster=\"cluster-b\" has 5 series"
                && !suggest),
        "{derived:?}"
    );
}

#[gpui_kit::test]
async fn ignore_writes_nothing_and_stays_hidden(cx: &mut TestAppContext) {
    let directory = tempfile::tempdir().unwrap();
    let (_runtime, handle, _pilot, page) = start(cx, directory.path()).await;
    answering(cx, &page, Ok(counts(&[("cluster-a", 3), ("cluster-b", 5)])));
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window.click("metrics-check-entries", cx);
    })
    .unwrap();
    checked(cx, handle, &page).await;
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window.click("metrics-entry-cluster-b", cx);
        window.click("metrics-entry-ignore", cx);
    })
    .unwrap();
    cx.run_until_parked();
    assert!(lines(cx, &page).iter().all(|(_, _, suggest)| !suggest));
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window.click("metrics-check-entries", cx);
    })
    .unwrap();
    checked(cx, handle, &page).await;
    assert!(lines(cx, &page).iter().all(|(_, _, suggest)| !suggest));
    assert_eq!(
        std::fs::read_to_string(directory.path().join("freshkube.yaml")).unwrap(),
        SETUP
    );
}

#[gpui_kit::test]
async fn an_unsaved_draft_disables_check_entries(cx: &mut TestAppContext) {
    let directory = tempfile::tempdir().unwrap();
    let (_runtime, handle, _pilot, page) = start(cx, directory.path()).await;
    let asked = answering(cx, &page, Ok(counts(&[("cluster-a", 3)])));
    cx.update_window(handle, |_, window, cx| {
        replace(window, cx, "metrics-label", "example_cluster");
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window.click("metrics-check-entries", cx);
    })
    .unwrap();
    cx.run_until_parked();
    assert_eq!(asked.load(Ordering::SeqCst), 0);
    assert!(lines(cx, &page).is_empty());
    assert_eq!(
        cx.read(|cx| page.read(cx).why_not_check_entries()),
        Some("Save the Metrics source first")
    );
}

#[gpui_kit::test]
async fn hiding_drops_the_check(cx: &mut TestAppContext) {
    let directory = tempfile::tempdir().unwrap();
    let (_runtime, handle, pilot, page) = start(cx, directory.path()).await;
    let (sender, receiver) = tokio::sync::oneshot::channel::<()>();
    let held = Arc::new(std::sync::Mutex::new(Some(receiver)));
    let entered = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let started = entered.clone();
    cx.update(|cx| {
        page.update(cx, |page, _| {
            page.monitoring.as_mut().unwrap().labels.labeler = Arc::new(move |_, _, _| {
                let receiver = held.lock().unwrap().take().unwrap();
                let started = started.clone();
                Box::pin(async move {
                    started.store(true, Ordering::SeqCst);
                    _ = receiver.await;
                    Ok(counts(&[("cluster-a", 3)]))
                })
            })
        })
    });
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window.click("metrics-check-entries", cx);
    })
    .unwrap();
    cx.wait_for_io(handle, |_, _| entered.load(Ordering::SeqCst))
        .await;
    cx.update_window(handle, |_, window, cx| {
        pilot.update(cx, |pilot, cx| {
            pilot.show_settings(SettingsSection::General, window, cx)
        });
    })
    .unwrap();
    cx.wait_for_io(handle, |_, _| sender.is_closed()).await;
    cx.read(|cx| {
        let labels = &page.read(cx).monitoring.as_ref().unwrap().labels;
        assert!(!labels.read.is_loading());
        assert!(labels.read.data().is_none());
        assert!(labels.lines.is_empty());
    });
}

#[gpui_kit::test]
async fn a_refused_store_suggests_nothing(cx: &mut TestAppContext) {
    let directory = tempfile::tempdir().unwrap();
    let (_runtime, handle, _pilot, page) = start(cx, directory.path()).await;
    answering(cx, &page, Err("The store refused the query".into()));
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window.click("metrics-check-entries", cx);
    })
    .unwrap();
    checked(cx, handle, &page).await;
    assert!(lines(cx, &page).is_empty());
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert!(window.find("entries-failed").visible());
        assert!(window.find("metrics-entry-accept").visible());
    })
    .unwrap();
    cx.read(|cx| {
        let labels = &page.read(cx).monitoring.as_ref().unwrap().labels;
        assert_eq!(
            labels.failed.as_deref(),
            Some("The store refused the query")
        );
    });
}

#[gpui_kit::test]
fn example_data_answers_at_once_and_cannot_accept(cx: &mut TestAppContext) {
    use crate::desktop::testing::fixture;
    let (_runtime, handle, pilot) = fixture(cx, 1280., 1600.);
    cx.update_window(handle, |_, window, cx| {
        pilot.update(cx, |pilot, cx| {
            pilot.show_settings(SettingsSection::Monitoring, window, cx)
        });
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window.click("metrics-check-entries", cx);
        window.render_frame(cx);
    })
    .unwrap();
    let page = cx.read(|cx| pilot.read(cx).settings_page.clone());
    let shown = lines(cx, &page);
    assert_eq!(shown.len(), 3, "{shown:?}");
    cx.update_window(handle, |_, window, cx| {
        window.click("metrics-entry-cluster-b", cx);
        window.click("metrics-entry-accept", cx);
    })
    .unwrap();
    cx.run_until_parked();
    // Nothing is saved from example data.
    cx.read(|cx| {
        let page = page.read(cx);
        assert!(page.saving.is_none());
        assert!(page.workspace().clusters[1].metrics.is_none());
    });
}
