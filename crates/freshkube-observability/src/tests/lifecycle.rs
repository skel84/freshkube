use super::*;

#[gpui_kit::test]

async fn hidden_page_cancels_tokio_work_and_rejects_a_queued_answer(cx: &mut TestAppContext) {
    use std::sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    };
    cx.executor().allow_parking();
    let (_runtime, handle, page) = mount(cx, false);
    let dropped = Arc::new(AtomicBool::new(false));
    let started = Arc::new(AtomicBool::new(false));
    struct OnDrop(Arc<AtomicBool>);
    impl Drop for OnDrop {
        fn drop(&mut self) {
            self.0.store(true, Ordering::SeqCst);
        }
    }
    cx.update(|cx| {
        page.update(cx, |page, cx| {
            let (dropped, started) = (dropped.clone(), started.clone());
            page.spawn_read(
                async move {
                    let _drop = OnDrop(dropped);
                    started.store(true, Ordering::SeqCst);
                    std::future::pending::<()>().await;
                    Ok(example::applications())
                },
                |page, result, cx| {
                    page.apply_applications(&result.unwrap());
                    cx.notify();
                },
                cx,
            );
        })
    });
    cx.wait_for_io(handle, move |_, _| started.load(Ordering::SeqCst))
        .await;
    cx.update(|cx| page.update(cx, |page, cx| page.set_visible(false, cx)));
    cx.wait_for_io(handle, move |_, _| dropped.load(Ordering::SeqCst))
        .await;
    cx.update(|cx| {
        assert!(page.read(cx).applications.is_empty());
        assert!(page.read(cx).live.jobs.is_empty());
    });
}

#[gpui_kit::test]
fn source_project_credentials_and_range_invalidate_old_requests(cx: &mut TestAppContext) {
    use freshkube_core::coroot::{Association, Credentials, ProjectInfo, Provider};
    let (_runtime, handle, page) = mount(cx, false);
    cx.update_window(handle, |_, window, cx| {
        page.update(cx, |page, cx| {
            let provider = Provider::new("http://127.0.0.1:1", Credentials::None).unwrap();
            page.live.provider = Some(provider.clone());
            let project = ProjectInfo {
                id: "one".into(),
                name: "Same name".into(),
            };
            page.live.source = Some(provider.source(&project));
            // Without a chosen application, its page starts no I/O.
            page.destination = Destination::Application;
            let read = hold_read(page, cx);
            let revision = read.borrow().revision();
            page.select_project(
                &ProjectInfo {
                    id: "two".into(),
                    name: "Same name".into(),
                },
                cx,
            );
            assert!(read.borrow().revision() > revision);
            let prior = page.live.source.clone();
            page.live.access = Some("access:a".into());
            page.entry_for_test(Some((Some("two"), "coroot-cluster")), cx);
            assert_ne!(prior, page.live.source);
            assert_eq!(
                page.live.source.as_ref().unwrap().association(),
                Some(&Association::new(
                    "access:a".into(),
                    "coroot-cluster".into()
                ))
            );
            let read = hold_read(page, cx);
            let revision = read.borrow().revision();
            page.set_source(None, None, cx);
            assert!(read.borrow().revision() > revision);
            assert!(page.live.source.as_ref().unwrap().association().is_none());
            let prior = page.live.range;
            page.set_range(24, cx);
            assert_ne!(prior, page.live.range);
            let read = hold_read(page, cx);
            let revision = read.borrow().revision();
            page.disconnect(window, cx);
            assert!(read.borrow().revision() > revision);
            assert!(page.live.provider.is_none());
            assert!(page.live.source.is_none());
            assert_ne!(
                provider.id(),
                Provider::new("http://127.0.0.1:1", Credentials::None)
                    .unwrap()
                    .id()
            );
        })
    })
    .unwrap();
}
