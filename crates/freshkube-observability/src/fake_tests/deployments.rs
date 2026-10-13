use super::*;

/// Waits until `count` requests named `part`, and the page read the
/// application.
async fn sent(
    cx: &mut TestAppContext,
    handle: gpui_kit::AnyWindowHandle,
    page: &gpui_kit::Entity<ObservabilityPage>,
    server: &Server,
    part: &'static str,
    count: usize,
) {
    let (observed, paths) = (page.clone(), server.paths.clone());
    cx.wait_for(handle, std::time::Duration::from_secs(5), move |_, cx| {
        let paths = paths.lock().unwrap();
        paths.iter().filter(|p| p.contains(part)).count() >= count
            && !observed.read(cx).live.view.is_loading()
    })
    .await;
}

/// Waits until the fake has written `count` answers to requests named
/// `part`, so a late answer has reached the page if it ever will.
async fn responded(
    cx: &mut TestAppContext,
    handle: gpui_kit::AnyWindowHandle,
    server: &Server,
    part: &'static str,
    count: usize,
) {
    let answered = server.responded.clone();
    cx.wait_for(handle, std::time::Duration::from_secs(5), move |_, _| {
        let answered = answered.lock().unwrap();
        answered.iter().filter(|p| p.contains(part)).count() >= count
    })
    .await;
    cx.run_until_parked();
}

/// Opens Deployments on the fake's application.
fn open_deployments(cx: &mut TestAppContext, page: &gpui_kit::Entity<ObservabilityPage>) {
    cx.update(|cx| {
        page.update(cx, |page, cx| {
            page.selected_app = Some(page.applications[0].id.clone());
            page.open(Destination::Deployments, cx);
        })
    });
}

/// The window the last answer covers, from its start in milliseconds.
fn answered_from(
    cx: &mut TestAppContext,
    page: &gpui_kit::Entity<ObservabilityPage>,
) -> Option<i64> {
    cx.read(|cx| {
        let answer = page.read(cx).live.revision.data();
        answer.map(|answer| answer.from.timestamp_millis())
    })
}

async fn revision_read(
    cx: &mut TestAppContext,
    handle: gpui_kit::AnyWindowHandle,
    page: &gpui_kit::Entity<ObservabilityPage>,
) {
    let observed = page.clone();
    cx.wait_for(handle, std::time::Duration::from_secs(5), move |_, cx| {
        let page = observed.read(cx);
        let failed = page.revision_observations.list_error().is_some();
        !page.live.view.is_loading()
            && (failed || page.live.revision_job.is_some() && !page.live.revision.is_loading())
    })
    .await;
    cx.read(|cx| {
        let error = page.read(cx).revision_observations.list_error().cloned();
        assert_eq!(error, None, "Coroot's revisions decode");
    });
}

#[gpui_kit::test]
async fn deployments_read_coroots_revisions_and_the_window_around_one(cx: &mut TestAppContext) {
    cx.executor().allow_parking();
    let (runtime, handle, page) = mount(cx, false);
    let server = Server::new(&runtime);
    server.deploy_mode.store(1, Ordering::SeqCst);
    connected(cx, handle, &page, server.url.clone()).await;
    cx.update(|cx| {
        page.update(cx, |page, cx| {
            page.selected_app = Some(page.applications[0].id.clone());
            page.open(Destination::Deployments, cx);
        })
    });
    revision_read(cx, handle, &page).await;
    let window = last_path(&server, "from=17899");
    assert!(
        window.contains("/app/cluster-a%3Aprod%3ADeployment%3Aapi"),
        "{window}"
    );
    assert!(window.contains("from=1789997200000"), "{window}");
    assert!(window.contains("to=1790000800000"), "{window}");
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert!(window.find("obs-revision-9c41e7-1789999000").visible());
        assert!(window.find("obs-revision-2b70aa-1789990000").visible());
        assert!(window.find("obs-revision-finding-0").visible());
        assert!(window.find("obs-revision-window").visible());
        // Coroot's page in the fake has no charts.
        assert!(window.find("obs-revision-no-charts").visible());
    })
    .unwrap();

    // Coroot's 404 for the window is no data, never a missing application.
    server.deploy_mode.store(2, Ordering::SeqCst);
    cx.update(|cx| page.update(cx, |page, cx| page.refresh(cx)));
    revision_read(cx, handle, &page).await;
    cx.update_window(handle, |_, window, cx| {
        assert!(page.read(cx).live.revision.error().is_none());
        window.render_frame(cx);
        assert!(window.find("obs-revision-no-data").visible());
        assert!(window.try_find("obs-revision-failed").is_none());
        assert!(window.find("obs-revision-9c41e7-1789999000").visible());
    })
    .unwrap();

    // Any other failure says so, with Retry.
    server.deploy_mode.store(3, Ordering::SeqCst);
    cx.update(|cx| page.update(cx, |page, cx| page.refresh(cx)));
    revision_read(cx, handle, &page).await;
    cx.update_window(handle, |_, window, cx| {
        assert!(page.read(cx).live.revision.error().is_some());
        window.render_frame(cx);
        assert!(window.find("obs-revision-failed").visible());
        assert!(window.find("obs-revision-retry").visible());
    })
    .unwrap();
    server.deploy_mode.store(1, Ordering::SeqCst);
    cx.update_window(handle, |_, window, cx| {
        window.click("obs-revision-retry", cx)
    })
    .unwrap();
    revision_read(cx, handle, &page).await;
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert!(window.try_find("obs-revision-failed").is_none());
        assert!(window.find("obs-revision-window").visible());
    })
    .unwrap();
}

#[gpui_kit::test]
async fn coming_from_the_application_page_keeps_its_revisions_while_it_is_read_again(
    cx: &mut TestAppContext,
) {
    cx.executor().allow_parking();
    let (runtime, handle, page) = mount(cx, false);
    let server = Server::new(&runtime);
    server.deploy_mode.store(1, Ordering::SeqCst);
    connected(cx, handle, &page, server.url.clone()).await;
    cx.update(|cx| {
        page.update(cx, |page, cx| {
            page.selected_app = Some(page.applications[0].id.clone());
            page.open(Destination::Application, cx);
        })
    });
    sent(cx, handle, &page, &server, "/app/", 1).await;
    cx.update_window(handle, |_, window, cx| {
        page.update(cx, |page, cx| {
            page.open(Destination::Deployments, cx);
            // Forgets the first read's page, so only the read again can
            // derive it.
            page.app_page = None;
        });
        assert!(
            page.read(cx).live.view.is_loading(),
            "the page is read again"
        );
        window.render_frame(cx);
        assert!(window.find("obs-revision-9c41e7-1789999000").visible());
        assert!(
            window.try_find("obs-deployments-empty").is_none(),
            "no empty state while the application is read again"
        );
    })
    .unwrap();
    revision_read(cx, handle, &page).await;
    cx.read(|cx| {
        assert!(
            page.read(cx).app_page.is_some(),
            "the Application page follows the read"
        )
    });
}

#[gpui_kit::test]
async fn refresh_and_coming_back_keep_an_older_revision_chosen(cx: &mut TestAppContext) {
    cx.executor().allow_parking();
    let (runtime, handle, page) = mount(cx, false);
    let server = Server::new(&runtime);
    server.deploy_mode.store(1, Ordering::SeqCst);
    connected(cx, handle, &page, server.url.clone()).await;
    open_deployments(cx, &page);
    revision_read(cx, handle, &page).await;
    let older = crate::tables::TableKey::Revision("2b70aa:1789990000".into());
    // The ±30 min window around the older revision's start.
    let around_older = Some(1_789_988_200_000);
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window.click("obs-revision-2b70aa-1789990000", cx);
    })
    .unwrap();
    revision_read(cx, handle, &page).await;
    assert_eq!(answered_from(cx, &page), around_older);

    // While it reads, the table shows its loading rows, as after any
    // Refresh, and the choice waits for the answer.
    let chosen = |cx: &mut TestAppContext, what: &str| {
        cx.read(|cx| {
            let page = page.read(cx);
            assert!(page.live.view.is_loading());
            assert_eq!(page.revision_selected_key(), Some(&older), "{what}");
        })
    };
    let check = |cx: &mut TestAppContext, what: &str| {
        cx.update_window(handle, |_, window, cx| {
            assert_eq!(
                page.read(cx).revision_selected_key(),
                Some(&older),
                "{what} keeps the older revision"
            );
            window.render_frame(cx);
            assert!(window.find("obs-revision-2b70aa-1789990000").visible());
            assert!(window.try_find("obs-deployments-empty").is_none());
        })
        .unwrap();
    };
    cx.update(|cx| page.update(cx, |page, cx| page.refresh_current(cx)));
    chosen(cx, "reading again");
    revision_read(cx, handle, &page).await;
    check(cx, "Refresh");
    assert_eq!(answered_from(cx, &page), around_older);

    cx.update(|cx| {
        page.update(cx, |page, cx| {
            page.set_visible(false, cx);
            page.set_visible(true, cx);
        })
    });
    chosen(cx, "coming back, while reading");
    revision_read(cx, handle, &page).await;
    check(cx, "coming back");
    assert_eq!(answered_from(cx, &page), around_older);
}

#[gpui_kit::test]
async fn a_window_read_shows_loading_and_a_newer_choice_or_hiding_drops_its_answer(
    cx: &mut TestAppContext,
) {
    cx.executor().allow_parking();
    let (runtime, handle, page) = mount(cx, false);
    let server = Server::new(&runtime);
    server.deploy_mode.store(4, Ordering::SeqCst);
    connected(cx, handle, &page, server.url.clone()).await;
    open_deployments(cx, &page);
    sent(cx, handle, &page, &server, NEWEST_WINDOW, 1).await;
    cx.update_window(handle, |_, window, cx| {
        assert!(page.read(cx).live.revision.is_loading());
        window.render_frame(cx);
        assert!(window.find("obs-revision-loading").visible());
        assert!(window.find("obs-revision-read").visible(), "Reading");
        // Another revision supersedes the read in flight.
        window.click("obs-revision-2b70aa-1789990000", cx);
    })
    .unwrap();
    revision_read(cx, handle, &page).await;
    server.window_gate.notify_one();
    responded(cx, handle, &server, NEWEST_WINDOW, 1).await;
    cx.read(|cx| {
        let answer = page.read(cx).live.revision.data().unwrap();
        assert_eq!(answer.revision.id, "2b70aa:1789990000");
    });

    // So does another window.
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window.click("obs-revision-9c41e7-1789999000", cx);
    })
    .unwrap();
    sent(cx, handle, &page, &server, NEWEST_WINDOW, 2).await;
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window.click("obs-revision-window-1", cx);
    })
    .unwrap();
    revision_read(cx, handle, &page).await;
    server.window_gate.notify_one();
    responded(cx, handle, &server, NEWEST_WINDOW, 2).await;
    assert_eq!(answered_from(cx, &page), Some(1_789_995_400_000));

    // Hiding drops the read in flight, and its late answer never lands.
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window.click("obs-revision-window-0", cx);
    })
    .unwrap();
    sent(cx, handle, &page, &server, NEWEST_WINDOW, 3).await;
    cx.update(|cx| {
        page.update(cx, |page, cx| {
            page.set_visible(false, cx);
            assert!(page.live.revision_job.is_none());
        })
    });
    server.window_gate.notify_one();
    responded(cx, handle, &server, NEWEST_WINDOW, 3).await;
    assert_ne!(answered_from(cx, &page), Some(1_789_997_200_000));
}

#[gpui_kit::test]
async fn a_refused_window_says_not_permitted(cx: &mut TestAppContext) {
    cx.executor().allow_parking();
    let (runtime, handle, page) = mount(cx, false);
    let server = Server::new(&runtime);
    server.deploy_mode.store(5, Ordering::SeqCst);
    connected(cx, handle, &page, server.url.clone()).await;
    open_deployments(cx, &page);
    revision_read(cx, handle, &page).await;
    cx.update_window(handle, |_, window, cx| {
        assert!(page.read(cx).live.revision_refused);
        window.render_frame(cx);
        assert!(window.find("obs-revision-refused").visible());
        assert!(window.try_find("obs-revision-failed").is_none());
        // The list stays: only the window was refused.
        assert!(window.find("obs-revision-9c41e7-1789999000").visible());
    })
    .unwrap();
}

#[gpui_kit::test]
async fn a_refused_application_page_says_not_permitted_on_deployments(cx: &mut TestAppContext) {
    cx.executor().allow_parking();
    let (runtime, handle, page) = mount(cx, false);
    let server = Server::new(&runtime);
    server.deploy_mode.store(1, Ordering::SeqCst);
    connected(cx, handle, &page, server.url.clone()).await;
    server.status.store(403, Ordering::SeqCst);
    open_deployments(cx, &page);
    sent(cx, handle, &page, &server, "/app/", 1).await;
    cx.update_window(handle, |_, window, cx| {
        assert!(page.read(cx).live.view_refused);
        window.render_frame(cx);
        assert!(window.find("obs-refused").visible());
        assert!(window.try_find("obs-failed").is_none());
        assert!(window.try_find("obs-deployments-empty").is_none());
    })
    .unwrap();
}

/// Hidden, the page asks Coroot nothing for a revision asked for; showing
/// it reads the application and finds the revision.
#[gpui_kit::test]
async fn a_revision_asked_for_while_hidden_is_read_once_the_page_shows(cx: &mut TestAppContext) {
    use crate::deployments::Outcome;
    cx.executor().allow_parking();
    let (runtime, handle, page) = mount(cx, false);
    let server = Server::new(&runtime);
    server.deploy_mode.store(1, Ordering::SeqCst);
    cx.update(|cx| page.update(cx, |page, cx| page.set_source(Some("conn-a".into()), cx)));
    connected(cx, handle, &page, server.url.clone()).await;
    cx.update(|cx| {
        page.update(cx, |page, cx| {
            page.associate(Some("cluster-a".into()), cx);
            page.set_visible(false, cx);
        })
    });
    cx.run_until_parked();
    let asked = |server: &Server| {
        let paths = server.paths.lock().unwrap();
        paths.iter().filter(|p| p.contains("/app/")).count()
    };
    let before = asked(&server);
    cx.update(|cx| {
        page.update(cx, |page, cx| {
            page.open_revision(
                crate::RevisionLink {
                    access: "conn-a".into(),
                    cluster: "dev".into(),
                    namespace: "prod".into(),
                    name: "api".into(),
                    hash: "2b70aa".into(),
                },
                cx,
            )
        })
    });
    cx.run_until_parked();
    assert_eq!(asked(&server), before, "nothing is read while hidden");
    cx.read(|cx| {
        assert_eq!(
            page.read(cx).revision_observations.wanted_outcome(),
            Some(Outcome::Seeking)
        )
    });
    cx.update(|cx| page.update(cx, |page, cx| page.set_visible(true, cx)));
    revision_read(cx, handle, &page).await;
    cx.read(|cx| {
        let page = page.read(cx);
        assert_eq!(
            page.revision_observations.wanted_outcome(),
            Some(Outcome::Found)
        );
        assert_eq!(
            page.revision_selected_key(),
            Some(&crate::tables::TableKey::Revision(
                "2b70aa:1789990000".into()
            ))
        );
    });
}

#[gpui_kit::test]
async fn a_revision_asked_for_needs_the_clusters_association_and_coroots_listing(
    cx: &mut TestAppContext,
) {
    use crate::deployments::Outcome;
    cx.executor().allow_parking();
    let (runtime, handle, page) = mount(cx, false);
    let server = Server::new(&runtime);
    server.deploy_mode.store(1, Ordering::SeqCst);
    cx.update(|cx| page.update(cx, |page, cx| page.set_source(Some("conn-a".into()), cx)));
    connected(cx, handle, &page, server.url.clone()).await;
    let link = |name: &str| crate::RevisionLink {
        access: "conn-a".into(),
        cluster: "dev".into(),
        namespace: "prod".into(),
        name: name.into(),
        hash: "2b70aa".into(),
    };
    let outcome = |cx: &mut TestAppContext| {
        cx.read(|cx| page.read(cx).revision_observations.wanted_outcome())
    };

    // No Coroot cluster is linked to the connection: nothing is chosen.
    cx.update(|cx| page.update(cx, |page, cx| page.open_revision(link("api"), cx)));
    assert_eq!(outcome(cx), Some(Outcome::NoAssociation));
    cx.read(|cx| assert_eq!(page.read(cx).selected_app, None));
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert!(window.find("obs-wanted-revision").visible());
    })
    .unwrap();

    // Linked, the Deployment's revision is found by its hash, without
    // asking for it again.
    cx.update(|cx| page.update(cx, |page, cx| page.associate(Some("cluster-a".into()), cx)));
    revision_read(cx, handle, &page).await;
    assert_eq!(outcome(cx), Some(Outcome::Found));
    cx.read(|cx| {
        let page = page.read(cx);
        assert_eq!(
            page.selected_app,
            Some(freshkube_core::coroot::AppId::new(
                "cluster-a:prod:Deployment:api"
            ))
        );
        assert_eq!(
            page.revision_selected_key(),
            Some(&crate::tables::TableKey::Revision(
                "2b70aa:1789990000".into()
            )),
            "the hash's revision, not the newest"
        );
    });
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert!(window.find("obs-wanted-revision").visible());
    })
    .unwrap();

    // One Coroot doesn't list stays unchosen, and nothing is read for it.
    cx.update(|cx| page.update(cx, |page, cx| page.open_revision(link("cart"), cx)));
    assert_eq!(outcome(cx), Some(Outcome::NotListed));
    cx.read(|cx| {
        assert_eq!(
            page.read(cx).selected_app,
            Some(freshkube_core::coroot::AppId::new(
                "cluster-a:prod:Deployment:api"
            ))
        )
    });
}
