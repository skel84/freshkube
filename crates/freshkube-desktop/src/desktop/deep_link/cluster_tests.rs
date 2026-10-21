//! Which cluster a link opens in, on example data: the open cluster's
//! fingerprint, the four ways a link is matched, Check, the dialog, the
//! shell question and the cache.
use super::identity::{Checked, Standing, Target};
use crate::debug_options::LinkShape;
use crate::desktop::session::SessionKey;
use crate::desktop::testing::record_notices;
use crate::desktop::tests::{fixture, mount, start_shell};
use crate::desktop::{Page, Pilot};
use crate::resources::{example, live};
use crate::{DebugOptions, GpuiOptions};
use freshkube_core::fingerprint::{Fingerprint, StrongRead, strong_from_override, strong_from_uid};
use freshkube_nav::{ClusterRef, FingerprintToken, Link, LinkPage, ViewState};
use gpui_kit::test::TestWindowExt;
use gpui_kit::{AnyWindowHandle, AppContext as _, Entity, TestAppContext};

fn token(text: &str) -> Option<FingerprintToken> {
    Some(FingerprintToken::parse(text).unwrap())
}

/// The link a fixture window open on `context` makes for etcd.
fn made_in(context: &str) -> String {
    Link {
        cluster: ClusterRef {
            strong: token(&strong_from_uid(&format!("example/kube-system/{context}"))),
            ca: token(&strong_from_uid(&format!("example/ca/{context}"))),
        },
        view: ViewState::page(LinkPage::Etcd),
    }
    .to_url()
}

fn shaped(cx: &mut TestAppContext, shape: LinkShape) -> (AnyWindowHandle, Entity<Pilot>) {
    let options = GpuiOptions::fixture().with_debug(DebugOptions {
        links: Some(shape),
        ..Default::default()
    });
    let (runtime, handle, view) = mount(cx, options, 1280., 820.);
    std::mem::forget(runtime);
    (handle, view)
}

fn open_text(cx: &mut TestAppContext, handle: AnyWindowHandle, view: &Entity<Pilot>, text: &str) {
    let text = text.to_owned();
    cx.update_window(handle, |_, window, cx| {
        view.update(cx, |pilot, cx| pilot.open_link_text(&text, window, cx))
    })
    .unwrap();
    cx.run_until_parked();
}

fn switch(cx: &mut TestAppContext, handle: AnyWindowHandle, view: &Entity<Pilot>, id: &str) {
    cx.update_window(handle, |_, window, cx| {
        view.update(cx, |pilot, cx| pilot.switch_cluster(id.into(), window, cx))
    })
    .unwrap();
    cx.run_until_parked();
}

fn active(cx: &mut TestAppContext, view: &Entity<Pilot>) -> SessionKey {
    cx.read(|cx| view.read(cx).switch.registry.active_key().clone())
}

fn asking(cx: &mut TestAppContext, view: &Entity<Pilot>) -> bool {
    cx.read(|cx| view.read(cx).deep_link.identities.choosing.is_some())
}

fn page(cx: &mut TestAppContext, view: &Entity<Pilot>) -> Page {
    cx.read(|cx| view.read(cx).navigation.page)
}

fn rows(cx: &mut TestAppContext, view: &Entity<Pilot>) -> Vec<(String, Standing, Checked)> {
    cx.read(|cx| {
        let choosing = view
            .read(cx)
            .deep_link
            .identities
            .choosing
            .as_ref()
            .expect("asking");
        choosing
            .entity
            .read(cx)
            .rows()
            .iter()
            .map(|row| (row.context.clone(), row.standing, row.checked.clone()))
            .collect()
    })
}

fn drawn(cx: &mut TestAppContext, handle: AnyWindowHandle, id: &str) -> bool {
    let id = id.to_owned();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window.try_find(id).is_some()
    })
    .unwrap()
}

fn click(cx: &mut TestAppContext, handle: AnyWindowHandle, id: &str) {
    let id = id.to_owned();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window.click(id, cx);
    })
    .unwrap();
    cx.run_until_parked();
}

/// Closes the dialog as Escape does.
fn cancel(cx: &mut TestAppContext, handle: AnyWindowHandle) {
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window.press("escape", cx);
    })
    .unwrap();
    cx.run_until_parked();
}

fn clipboard(cx: &mut TestAppContext) -> String {
    cx.read_from_clipboard()
        .and_then(|item| item.text())
        .expect("text on the clipboard")
}

fn copy(cx: &mut TestAppContext, handle: AnyWindowHandle, view: &Entity<Pilot>) -> String {
    cx.update_window(handle, |_, window, cx| {
        view.update(cx, |pilot, cx| pilot.copy_link(window, cx))
    })
    .unwrap();
    cx.run_until_parked();
    clipboard(cx)
}

#[gpui_kit::test]
fn copy_link_carries_the_open_clusters_fingerprint_and_no_name(cx: &mut TestAppContext) {
    let (_runtime, handle, view) = fixture(cx, 1280., 820.);
    let text = copy(cx, handle, &view);
    let link = Link::parse(&text).unwrap();
    assert_eq!(
        link.cluster.strong.as_ref().map(FingerprintToken::as_str),
        Some(strong_from_uid("example/kube-system/prod-fra").as_str())
    );
    assert_eq!(
        link.cluster.ca.as_ref().map(FingerprintToken::as_str),
        Some(strong_from_uid("example/ca/prod-fra").as_str())
    );
    // Neither the context's name nor the namespace's UID is in it.
    assert!(!text.contains("prod-fra"), "{text}");
    assert!(!text.contains("kube-system"), "{text}");
}

#[gpui_kit::test]
fn a_link_made_in_the_open_cluster_opens_there_without_asking(cx: &mut TestAppContext) {
    let (_runtime, handle, view) = fixture(cx, 1280., 820.);
    open_text(cx, handle, &view, &made_in("prod-fra"));
    assert!(!asking(cx, &view));
    assert_eq!(page(cx, &view), Page::Etcd);
}

#[gpui_kit::test]
fn a_cluster_never_opened_matches_on_the_ca_alone_and_asks(cx: &mut TestAppContext) {
    let (_runtime, handle, view) = fixture(cx, 1280., 820.);
    let before = (active(cx, &view), page(cx, &view));
    open_text(cx, handle, &view, &made_in("dev-fra"));
    assert!(asking(cx, &view));
    // Nothing moved, and nothing was checked behind the reader's back.
    assert_eq!((active(cx, &view), page(cx, &view)), before);
    let found = rows(cx, &view);
    let dev = found
        .iter()
        .find(|row| row.0 == "dev-fra")
        .expect("dev-fra is listed");
    assert_eq!((dev.1, &dev.2), (Standing::CaOnly, &Checked::No));
    assert!(drawn(cx, handle, "link-choice"));
    assert!(drawn(cx, handle, "link-choice-row-dev-fra"));
    assert!(drawn(cx, handle, "link-choice-check-dev-fra"));
    assert!(drawn(cx, handle, "link-choice-open-dev-fra"));
    cx.read(|cx| {
        let identities = &view.read(cx).deep_link.identities;
        assert!(identities.choosing.is_some());
    });
    // Every row not checked yet has a Check of its own.
    for (context, _, checked) in &found {
        assert_eq!(
            drawn(cx, handle, &format!("link-choice-check-{context}")),
            *checked == Checked::No,
            "{context}"
        );
    }
}

#[gpui_kit::test]
fn check_makes_a_ca_only_match_the_same_cluster_and_open_here_switches_and_opens(
    cx: &mut TestAppContext,
) {
    let (_runtime, handle, view) = fixture(cx, 1280., 820.);
    open_text(cx, handle, &view, &made_in("dev-fra"));
    click(cx, handle, "link-choice-check-dev-fra");
    let found = rows(cx, &view);
    let dev = found.iter().find(|row| row.0 == "dev-fra").unwrap();
    assert_eq!((dev.1, &dev.2), (Standing::Exact, &Checked::Yes));
    // The others stay as they were: Check is per context.
    assert!(
        found
            .iter()
            .filter(|row| row.0 != "dev-fra")
            .all(|row| row.2 == Checked::No)
    );
    // Open here is the reader's own choice: the switch, then the view.
    click(cx, handle, "link-choice-open-dev-fra");
    assert!(!asking(cx, &view));
    assert_eq!(active(cx, &view), SessionKey::Entry("dev-fra".into()));
    assert_eq!(page(cx, &view), Page::Etcd);
}

#[gpui_kit::test]
fn cancel_leaves_everything_where_it_was(cx: &mut TestAppContext) {
    let (_runtime, handle, view) = fixture(cx, 1280., 820.);
    let before = (active(cx, &view), page(cx, &view));
    open_text(cx, handle, &view, &made_in("dev-fra"));
    cancel(cx, handle);
    assert!(!asking(cx, &view));
    assert_eq!((active(cx, &view), page(cx, &view)), before);
}

#[gpui_kit::test]
fn a_cluster_opened_before_is_an_exact_match_and_opens_without_asking(cx: &mut TestAppContext) {
    let (_runtime, handle, view) = fixture(cx, 1280., 820.);
    // Opening a context reads it once; the window keeps what it found.
    switch(cx, handle, &view, "dev-fra");
    switch(cx, handle, &view, "core-fra");
    open_text(cx, handle, &view, &made_in("dev-fra"));
    assert!(!asking(cx, &view));
    assert_eq!(active(cx, &view), SessionKey::Entry("dev-fra".into()));
    assert_eq!(page(cx, &view), Page::Etcd);
}

#[gpui_kit::test]
fn several_exact_matches_ask_unless_one_of_them_is_open(cx: &mut TestAppContext) {
    let (handle, view) = shaped(cx, LinkShape::Several);
    for id in ["dev-fra", "stage-fra", "core-fra"] {
        switch(cx, handle, &view, id);
    }
    // dev-fra and stage-fra are one cluster; neither is open.
    open_text(cx, handle, &view, &made_in("dev-fra"));
    assert!(asking(cx, &view));
    let found = rows(cx, &view);
    let exact: Vec<_> = found
        .iter()
        .filter(|row| row.1 == Standing::Exact)
        .map(|row| row.0.as_str())
        .collect();
    assert_eq!(exact, ["dev-fra", "stage-fra"]);
    cancel(cx, handle);
    // With one of them open, that one is used, without asking, and nothing
    // switches.
    switch(cx, handle, &view, "stage-fra");
    open_text(cx, handle, &view, &made_in("dev-fra"));
    assert!(!asking(cx, &view));
    assert_eq!(active(cx, &view), SessionKey::Entry("stage-fra".into()));
    assert_eq!(page(cx, &view), Page::Etcd);
}

#[gpui_kit::test]
fn a_match_on_the_ca_alone_always_asks_even_for_the_open_cluster(cx: &mut TestAppContext) {
    let (handle, view) = shaped(cx, LinkShape::CaOnly);
    // Every example cluster refuses the read: the link has its CA alone.
    let text = copy(cx, handle, &view);
    let link = Link::parse(&text).unwrap();
    assert!(
        link.cluster.strong.is_none() && link.cluster.ca.is_some(),
        "{text}"
    );
    open_text(cx, handle, &view, &text);
    assert!(asking(cx, &view));
    let found = rows(cx, &view);
    let open = found.iter().find(|row| row.0 == "prod-fra").unwrap();
    assert_eq!((open.1, &open.2), (Standing::CaOnly, &Checked::Refused));
}

#[gpui_kit::test]
fn no_matching_cluster_says_so_and_offers_the_contexts(cx: &mut TestAppContext) {
    let (handle, view) = shaped(cx, LinkShape::Unmatched);
    open_text(cx, handle, &view, &made_in("prod-fra"));
    assert!(asking(cx, &view));
    let found = rows(cx, &view);
    // Every context is offered, and none claims to be the one.
    assert!(found.len() >= 6, "{found:?}");
    assert!(
        found
            .iter()
            .all(|row| !matches!(row.1, Standing::Exact | Standing::CaOnly))
    );
    cx.read(|cx| {
        let why = view
            .read(cx)
            .deep_link
            .identities
            .choosing
            .as_ref()
            .map(|c| c.why_title());
        assert_eq!(why, Some("No matching cluster"));
    });
    // The reader may still open one, explicitly.
    click(cx, handle, "link-choice-open-dev-fra");
    assert_eq!(active(cx, &view), SessionKey::Entry("dev-fra".into()));
    assert_eq!(page(cx, &view), Page::Etcd);
}

#[gpui_kit::test]
fn unchecked_checks_nothing_not_even_the_open_cluster(cx: &mut TestAppContext) {
    let (handle, view) = shaped(cx, LinkShape::Unchecked);
    cx.read(|cx| {
        let (_, checked) = view.read(cx).identity_of("prod-fra", None);
        assert_eq!(checked, Checked::No);
    });
    // Its link has the CA alone, and so does every link it opens.
    let text = copy(cx, handle, &view);
    let link = Link::parse(&text).unwrap();
    assert!(
        link.cluster.strong.is_none() && link.cluster.ca.is_some(),
        "{text}"
    );
    open_text(cx, handle, &view, &text);
    assert!(asking(cx, &view));
    let found = rows(cx, &view);
    assert!(found.iter().all(|row| row.2 == Checked::No), "{found:?}");
    // Nothing was read, so nothing was kept.
    cx.read(|cx| assert!(view.read(cx).deep_link.identities.is_empty()));
    // Check is the reader's, one context at a time.
    click(cx, handle, "link-choice-check-prod-fra");
    let found = rows(cx, &view);
    assert_eq!(found.iter().filter(|row| row.2 == Checked::Yes).count(), 1);
}

#[gpui_kit::test]
fn a_failed_check_is_shown_and_can_be_tried_again(cx: &mut TestAppContext) {
    let (_runtime, handle, view) = fixture(cx, 1280., 820.);
    open_text(cx, handle, &view, &made_in("dev-fra"));
    cx.update(|cx| {
        view.update(cx, |pilot, cx| {
            let local = pilot.deep_link.identities.local.get("dev-fra").cloned();
            pilot.record(
                "dev-fra",
                local,
                StrongRead::Failed("the sign-in timed out".into()),
                cx,
            )
        })
    });
    cx.run_until_parked();
    let found = rows(cx, &view);
    let dev = found.iter().find(|row| row.0 == "dev-fra").unwrap();
    assert_eq!(dev.2, Checked::Failed("the sign-in timed out".into()));
    // Not a fallback to the CA alone: the row says why, and Check stays.
    assert!(drawn(cx, handle, "link-choice-check-dev-fra"));
    click(cx, handle, "link-choice-check-dev-fra");
    let found = rows(cx, &view);
    assert_eq!(
        found.iter().find(|row| row.0 == "dev-fra").unwrap().2,
        Checked::Yes
    );
}

#[gpui_kit::test]
fn a_running_shell_is_asked_about_only_when_the_link_switches_the_cluster(cx: &mut TestAppContext) {
    let (_runtime, handle, view) = fixture(cx, 1280., 820.);
    switch(cx, handle, &view, "dev-fra");
    switch(cx, handle, &view, "core-fra");
    let context = cx.read(|cx| view.read(cx).applied.context.clone().unwrap());
    let pod = example::read(&context, "pods", None, live::now())
        .unwrap()
        .1
        .into_iter()
        .find(|row| row.cells[2] == "Running")
        .unwrap()
        .identity;
    start_shell(handle, &view, &pod, cx);
    // On the open cluster: no question, and the shell goes on.
    open_text(cx, handle, &view, &made_in("core-fra"));
    assert!(cx.pending_prompt().is_none());
    assert_eq!(page(cx, &view), Page::Etcd);
    cx.read(|cx| assert!(!crate::resources::shell::running_anywhere(cx).is_empty()));
    // Another cluster: the shell question, and Cancel keeps everything.
    open_text(cx, handle, &view, &made_in("dev-fra"));
    assert!(cx.pending_prompt().is_some());
    cx.simulate_prompt_answer("Cancel");
    cx.run_until_parked();
    assert_eq!(active(cx, &view), SessionKey::Entry("core-fra".into()));
    cx.read(|cx| {
        assert!(view.read(cx).switch.pending_link.is_none());
        assert!(!crate::resources::shell::running_anywhere(cx).is_empty());
    });
    // Agreeing ends the shell, switches and opens the view there.
    open_text(cx, handle, &view, &made_in("dev-fra"));
    cx.simulate_prompt_answer("End the shell");
    cx.run_until_parked();
    assert_eq!(active(cx, &view), SessionKey::Entry("dev-fra".into()));
    assert_eq!(page(cx, &view), Page::Etcd);
}

#[gpui_kit::test]
fn a_setup_file_fingerprint_stands_for_the_clusters_own_and_is_never_in_a_link(
    cx: &mut TestAppContext,
) {
    let (_runtime, _handle, view) = fixture(cx, 1280., 820.);
    cx.read(|cx| {
        let pilot = view.read(cx);
        let (identity, checked) = pilot.identity_of("prod-fra", Some("prod-eu"));
        assert_eq!(checked, Checked::Yes);
        assert_eq!(identity.strong, strong_from_override("prod-eu"));
        assert_ne!(
            identity.strong,
            pilot.identity_of("prod-fra", None).0.strong
        );
        // The CA stays the local one; the override text is nowhere.
        assert!(identity.ca.is_some());
        assert!(!format!("{identity:?}").contains("prod-eu"));
    });
}

#[test]
fn what_a_check_found_is_kept_beside_the_preferences_as_hashes_only() {
    use super::cluster::Identities;
    use super::identity::{Local, Record};
    let directory = tempfile::tempdir().unwrap();
    let preferences = directory.path().join("preferences.json");
    let local = Local {
        server: strong_from_uid("https://dev.example.test:6443"),
        ca: Some(strong_from_uid("a ca")),
    };
    let record = Record {
        server: local.server.clone(),
        ca: local.ca.clone(),
        strong: Some(strong_from_uid("11111111-2222-3333-4444-555555555555")),
        refused: false,
    };
    let mut identities = Identities::open(Some(&preferences));
    assert!(identities.is_empty());
    identities.keep("dev", record.clone()).unwrap();
    let saved = std::fs::read_to_string(directory.path().join("fingerprints.json"))
        .expect("the cache is saved beside the preferences");
    // Hashes only: no UID, no address.
    for forbidden in ["11111111", "https://", "example.test", "a ca"] {
        assert!(!saved.contains(forbidden), "{forbidden} in {saved}");
    }
    // A later launch has it at once, while the server and CA are unchanged.
    let later = Identities::open(Some(&preferences));
    assert!(later.has_record("dev"));
    assert_eq!(later.valid("dev", Some(&local)), Some(&record));
    let moved = Local {
        server: strong_from_uid("another"),
        ..local
    };
    assert_eq!(later.valid("dev", Some(&moved)), None);
}

#[gpui_kit::test]
fn example_data_saves_nothing(cx: &mut TestAppContext) {
    cx.executor().allow_parking();
    let directory = tempfile::tempdir().unwrap();
    let preferences = directory.path().join("preferences.json");
    let options = GpuiOptions::fixture().with_preferences(Some(preferences));
    let (_runtime, handle, view) = mount(cx, options, 1280., 820.);
    open_text(cx, handle, &view, &made_in("dev-fra"));
    click(cx, handle, "link-choice-check-dev-fra");
    cx.run_until_parked();
    assert!(!directory.path().join("fingerprints.json").exists());
}

#[gpui_kit::test]
fn settings_shows_the_fingerprint_read_only_with_copy(cx: &mut TestAppContext) {
    let (_runtime, handle, view) = fixture(cx, 1280., 820.);
    cx.update_window(handle, |_, window, cx| {
        view.update(cx, |pilot, cx| {
            pilot.navigate_from_keyboard(Page::Settings, window, cx)
        });
        window.render_frame(cx);
        window.click("settings-cluster-prod-fra", cx);
        window.render_frame(cx);
    })
    .unwrap();
    cx.run_until_parked();
    let strong = strong_from_uid("example/kube-system/prod-fra");
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window.scroll(
            "settings-detail-body",
            gpui_kit::ScrollDelta::Pixels(gpui_kit::point(gpui_kit::px(0.), gpui_kit::px(-4000.))),
            cx,
        );
        window.render_frame(cx);
        assert_eq!(
            window.find("settings-fingerprint-value").label(),
            Some(strong.as_str())
        );
        window.click("settings-fingerprint-copy", cx);
    })
    .unwrap();
    assert_eq!(clipboard(cx), strong);
    // A cluster never opened says so, and has nothing to copy.
    cx.update_window(handle, |_, window, cx| {
        window.scroll(
            "settings-detail-body",
            gpui_kit::ScrollDelta::Pixels(gpui_kit::point(gpui_kit::px(0.), gpui_kit::px(4000.))),
            cx,
        );
        window.render_frame(cx);
        window.click("settings-cluster-dev-fra", cx);
        window.render_frame(cx);
        window.scroll(
            "settings-detail-body",
            gpui_kit::ScrollDelta::Pixels(gpui_kit::point(gpui_kit::px(0.), gpui_kit::px(-4000.))),
            cx,
        );
        window.render_frame(cx);
        assert_eq!(
            window.find("settings-fingerprint-value").label(),
            Some("Not available")
        );
    })
    .unwrap();
}

#[gpui_kit::test]
fn a_link_without_a_fingerprint_still_opens_in_the_open_cluster(cx: &mut TestAppContext) {
    let (_runtime, handle, view) = fixture(cx, 1280., 820.);
    let notices = record_notices(cx, &view);
    open_text(cx, handle, &view, "freshkube://open?page=etcd");
    assert!(!asking(cx, &view));
    assert_eq!(page(cx, &view), Page::Etcd);
    assert!(notices.borrow().is_empty());
}

#[test]
fn the_dialogs_standing_words_are_stable() {
    let fingerprint = Fingerprint::default();
    assert!(fingerprint.is_empty());
    assert_eq!(Target::Entry("a".into()), Target::Entry("a".into()));
}
