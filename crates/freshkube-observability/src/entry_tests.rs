//! The open entry's Coroot link: a linked entry's links, an unlinked
//! entry's state, switching the entry, a setup reload and a late answer.
use super::{ObservabilityPage, tests::mount};
use crate::connection::{ReadJob, Subject};
use freshkube_core::coroot as api;
use freshkube_core::workspace::{Entry, Role};
use gpui_kit::test::TestWindowExt;
use gpui_kit::{AnyWindowHandle, AppContext, Context, Entity, TestAppContext};

fn entry(id: &str, coroot: Option<(Option<&str>, &str)>) -> Entry {
    let mut entry = Entry::new(id, Role::Core, format!("context-{id}"));
    entry.coroot = coroot.map(|(project, cluster)| {
        let mut key = freshkube_core::setup::EntryCoroot::new(cluster);
        key.project = project.map(str::to_owned);
        Box::new(key)
    });
    entry
}

/// A connected, shown page on project `shop-api`, with the entries listed.
fn connected(
    cx: &mut TestAppContext,
    entries: &[Entry],
) -> (
    tokio::runtime::Runtime,
    AnyWindowHandle,
    Entity<ObservabilityPage>,
) {
    let (runtime, handle, page) = mount(cx, false);
    cx.update(|cx| {
        page.update(cx, |page, cx| {
            let provider =
                api::Provider::new("http://127.0.0.1:1", api::Credentials::None).unwrap();
            let project = api::ProjectInfo {
                id: "shop-api".into(),
                name: "Shop".into(),
            };
            page.live.source = Some(provider.source(&project));
            page.live.projects = vec![project];
            page.live.provider = Some(provider);
            page.set_entries(entries, cx);
            page.destination = crate::Destination::Application;
        })
    });
    (runtime, handle, page)
}

fn open(cx: &mut TestAppContext, page: &Entity<ObservabilityPage>, access: &str, id: &str) {
    cx.update(|cx| {
        page.update(cx, |page, cx| {
            page.set_access_and_entry(Some(access.into()), None, Some(id), cx)
        })
    });
}

fn association(
    cx: &mut TestAppContext,
    page: &Entity<ObservabilityPage>,
) -> Option<api::Association> {
    cx.read(|cx| {
        page.read(cx)
            .live
            .source
            .as_ref()
            .and_then(|source| source.association().cloned())
    })
}

/// A read in flight that must never deliver.
fn hold(cx: &mut TestAppContext, page: &Entity<ObservabilityPage>) -> ReadJob {
    cx.update(|cx| {
        page.update(cx, |page: &mut ObservabilityPage, cx: &mut Context<_>| {
            let read = page.spawn_owned_read(
                std::future::pending::<Result<(), api::ReadError>>(),
                |_, _, _| panic!("a read for the old link must not deliver"),
                cx,
            );
            page.live.jobs.push(read.clone());
            read
        })
    })
}

fn note(cx: &mut TestAppContext, page: &Entity<ObservabilityPage>) -> Option<String> {
    cx.read(|cx| page.read(cx).link_note())
}

#[gpui_kit::test]
fn a_linked_entry_reads_its_own_cluster_and_project(cx: &mut TestAppContext) {
    let entries = [
        entry("entry-a", Some((None, "cluster-one"))),
        entry("entry-b", Some((Some("shop-api"), "cluster-two"))),
    ];
    let (_runtime, handle, page) = connected(cx, &entries);
    open(cx, &page, "access:a", "entry-a");
    // The first entry has no project of its own and the connection names
    // none: it is not linked, though its cluster is named.
    assert_eq!(association(cx, &page), None);
    assert!(
        note(cx, &page)
            .unwrap()
            .contains("names a Coroot cluster but no project")
    );

    open(cx, &page, "access:b", "entry-b");
    assert_eq!(
        association(cx, &page),
        Some(api::Association::new(
            "access:b".into(),
            "cluster-two".into()
        ))
    );
    assert_eq!(note(cx, &page), None);
    cx.update_window(handle, |_, window, cx| {
        page.update(cx, |page, cx| {
            page.destination = crate::Destination::Applications;
            page.show_connection(cx)
        });
        window.render_frame(cx);
        assert!(window.try_find("obs-entry-link").is_some());
    })
    .unwrap();
    // The cluster is the entry's, so an application of it opens its object.
    let app = api::AppId::new("cluster-two:prod:Deployment:api");
    cx.read(|cx| {
        let source = page.read(cx).live.source.clone().unwrap();
        assert!(api::ObjectSubject::for_app(&source, &app).is_some());
    });
}

#[gpui_kit::test]
fn an_entry_without_the_key_is_not_linked_even_alone_and_even_by_name(cx: &mut TestAppContext) {
    let (_runtime, handle, page) = connected(cx, &[entry("cluster-two", None)]);
    open(cx, &page, "access:a", "cluster-two");
    assert_eq!(association(cx, &page), None);
    let reason = note(cx, &page).unwrap();
    assert!(reason.contains("clusters[].coroot.cluster"), "{reason}");
    assert!(reason.contains("setup file"), "{reason}");
    assert!(!reason.contains("Settings"), "{reason}");
    // A Coroot cluster of the entry's own name links nothing.
    let app = api::AppId::new("cluster-two:prod:Deployment:api");
    cx.read(|cx| {
        let source = page.read(cx).live.source.clone().unwrap();
        assert!(api::ObjectSubject::for_app(&source, &app).is_none());
    });
    cx.update_window(handle, |_, window, cx| {
        page.update(cx, |page, cx| {
            page.destination = crate::Destination::Applications;
            page.show_connection(cx)
        });
        window.render_frame(cx);
        assert!(window.try_find("obs-entry-link").is_some());
    })
    .unwrap();
    // A connection that is no entry of the setup is not linked either.
    open(cx, &page, "access:a", "somewhere-else");
    assert!(
        note(cx, &page)
            .unwrap()
            .contains("clusters[].coroot.cluster")
    );
}

#[gpui_kit::test]
fn switching_the_entry_reads_again_and_drops_the_old_link(cx: &mut TestAppContext) {
    let entries = [
        entry("entry-a", Some((Some("shop-api"), "cluster-one"))),
        entry("entry-b", Some((Some("shop-api"), "cluster-two"))),
        entry("entry-c", None),
    ];
    let (_runtime, _handle, page) = connected(cx, &entries);
    open(cx, &page, "access:a", "entry-a");
    let read = hold(cx, &page);
    let revision = read.borrow().revision();
    open(cx, &page, "access:b", "entry-b");
    assert!(read.borrow().revision() > revision);
    assert_eq!(
        association(cx, &page),
        Some(api::Association::new(
            "access:b".into(),
            "cluster-two".into()
        ))
    );
    let read = hold(cx, &page);
    let revision = read.borrow().revision();
    open(cx, &page, "access:c", "entry-c");
    assert!(read.borrow().revision() > revision);
    assert_eq!(association(cx, &page), None);
    cx.run_until_parked();
}

#[gpui_kit::test]
fn a_setup_reload_that_changes_the_link_reads_again(cx: &mut TestAppContext) {
    let (_runtime, _handle, page) = connected(
        cx,
        &[entry("entry-a", Some((Some("shop-api"), "cluster-one")))],
    );
    open(cx, &page, "access:a", "entry-a");
    let read = hold(cx, &page);
    let revision = read.borrow().revision();
    // The same setup again changes nothing and reads nothing.
    cx.update(|cx| {
        page.update(cx, |page, cx| {
            page.set_entries(
                &[entry("entry-a", Some((Some("shop-api"), "cluster-one")))],
                cx,
            )
        })
    });
    assert_eq!(read.borrow().revision(), revision);
    // The cluster edited in the file.
    cx.update(|cx| {
        page.update(cx, |page, cx| {
            page.set_entries(
                &[entry("entry-a", Some((Some("shop-api"), "cluster-new")))],
                cx,
            )
        })
    });
    assert!(read.borrow().revision() > revision);
    assert_eq!(
        association(cx, &page),
        Some(api::Association::new(
            "access:a".into(),
            "cluster-new".into()
        ))
    );
    // The key removed from the file.
    cx.update(|cx| {
        page.update(cx, |page, cx| {
            page.set_entries(&[entry("entry-a", None)], cx)
        })
    });
    assert_eq!(association(cx, &page), None);
    cx.run_until_parked();
}

#[gpui_kit::test]
fn an_answer_for_the_old_link_does_not_land(cx: &mut TestAppContext) {
    let entries = [
        entry("entry-a", Some((Some("shop-api"), "cluster-one"))),
        entry("entry-b", Some((Some("shop-api"), "cluster-two"))),
    ];
    let (_runtime, _handle, page) = connected(cx, &entries);
    open(cx, &page, "access:a", "entry-a");
    let old = cx.update(|cx| {
        page.update(cx, |page, _| {
            let identity = page.live.identity(Subject::Applications).unwrap();
            page.live.apps.begin(identity)
        })
    });
    let read = hold(cx, &page);
    open(cx, &page, "access:b", "entry-b");
    // The in-flight read is dropped with the old link, and its request
    // can no longer publish, success or failure.
    assert!(!read.borrow().is_loading());
    cx.update(|cx| {
        page.update(cx, |page, _| {
            assert!(!page.live.apps.apply(&old, Ok(vec![])));
            assert!(!page.live.apps.apply(&old, Err("late".into())));
            assert!(page.live.apps.data().is_none());
        })
    });
    cx.run_until_parked();
}

#[gpui_kit::test]
fn the_link_kubernetes_cluster_action_is_gone(cx: &mut TestAppContext) {
    for coroot in [Some((Some("shop-api"), "cluster-one")), None] {
        let (_runtime, handle, page) = connected(cx, &[entry("entry-a", coroot)]);
        open(cx, &page, "access:a", "entry-a");
        cx.update_window(handle, |_, window, cx| {
            page.update(cx, |page, cx| {
                page.destination = crate::Destination::Applications;
                page.show_connection(cx)
            });
            window.render_frame(cx);
            for id in ["obs-associate", "obs-unmap"] {
                assert!(window.try_find(id).is_none(), "{id}");
            }
        })
        .unwrap();
    }
}

#[gpui_kit::test]
fn a_hidden_page_reads_nothing_for_a_new_link(cx: &mut TestAppContext) {
    let entries = [
        entry("entry-a", Some((Some("shop-api"), "cluster-one"))),
        entry("entry-b", Some((Some("shop-api"), "cluster-two"))),
    ];
    let (_runtime, _handle, page) = connected(cx, &entries);
    open(cx, &page, "access:a", "entry-a");
    cx.update(|cx| page.update(cx, |page, cx| page.set_visible(false, cx)));
    open(cx, &page, "access:b", "entry-b");
    cx.read(|cx| assert!(page.read(cx).live.jobs.is_empty()));
    assert_eq!(
        association(cx, &page),
        Some(api::Association::new(
            "access:b".into(),
            "cluster-two".into()
        ))
    );
}

#[gpui_kit::test]
fn example_data_says_so_when_the_open_entry_is_not_linked(cx: &mut TestAppContext) {
    use crate::{Report, example};
    let (_runtime, handle, page) = mount(cx, true);
    cx.update(|cx| {
        page.update(cx, |page, cx| {
            page.set_entries(
                &[
                    entry("entry-a", None),
                    entry("entry-b", Some((Some("shop-api"), "fixture"))),
                ],
                cx,
            );
            page.set_source(
                Some(&crate::tests::example_source(false)),
                Some("entry-a"),
                cx,
            );
            page.open_app(example::id(example::WORKER), Report::Net, cx)
        })
    });
    let shown = |cx: &mut TestAppContext| {
        cx.update_window(handle, |_, window, cx| {
            window.render_frame(cx);
            window.try_find("obs-no-kubernetes-link").is_some()
        })
        .unwrap()
    };
    assert!(shown(cx));
    cx.update(|cx| {
        page.update(cx, |page, cx| {
            page.set_source(
                Some(&crate::tests::example_source(false)),
                Some("entry-b"),
                cx,
            )
        })
    });
    assert!(!shown(cx));
}
