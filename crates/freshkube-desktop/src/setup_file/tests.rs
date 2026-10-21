use super::*;
use freshkube_probe::probe;
use gpui_kit::test::TestWindowExt;
use gpui_kit::{
    AnyWindowHandle, Entity, IntoElement, Render, Subscription, TestAppContext, Window, div, px,
    size,
};

const FIRST: &str = "version: 1\nclusters:\n- {id: cluster-a, role: core, context: cluster-a}\n";
const SECOND: &str = "version: 1\nclusters:\n- {id: cluster-b, role: core, context: cluster-b}\n";

struct Reader {
    file: Entity<SetupFile>,
    changes: usize,
    _subscription: Subscription,
}

impl Render for Reader {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        probe::hit("setup.reader.render");
        div()
    }
}

fn mount(
    cx: &mut TestAppContext,
    directory: &std::path::Path,
    example: bool,
) -> (AnyWindowHandle, Entity<Reader>) {
    let preferences = directory.join("preferences.json");
    let mut reader = None;
    let handle = cx.open_window(size(px(400.), px(300.)), |_, cx| {
        let file = cx.new(|cx| SetupFile::new(Some(preferences), example, cx));
        reader = Some(cx.entity());
        Reader {
            _subscription: cx.subscribe(&file, |this: &mut Reader, _, event, cx| {
                if matches!(event, Event::Changed { .. }) {
                    this.changes += 1;
                }
                cx.notify();
            }),
            file,
            changes: 0,
        }
    });
    let handle = handle.into();
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| window.render_frame(cx))
        .unwrap();
    (handle, reader.unwrap())
}

fn advance(cx: &mut TestAppContext, duration: Duration) {
    cx.executor().advance_clock(duration);
    cx.run_until_parked();
}

fn model(reader: &Entity<Reader>, cx: &mut TestAppContext) -> Entity<SetupFile> {
    cx.update(|cx| reader.read(cx).file.clone())
}

fn keep_modified(path: &std::path::Path, modified: std::time::SystemTime) {
    std::fs::File::options()
        .write(true)
        .open(path)
        .unwrap()
        .set_times(std::fs::FileTimes::new().set_modified(modified))
        .unwrap();
    assert_eq!(
        std::fs::metadata(path).unwrap().modified().unwrap(),
        modified
    );
}

#[gpui_kit::test]
fn save_with_starts_a_missing_setup_and_workspace_saves_keep_metrics(cx: &mut TestAppContext) {
    let directory = tempfile::tempdir().unwrap();
    let (_, reader) = mount(cx, directory.path(), false);
    let file = model(&reader, cx);
    let save = file.update(cx, |file, cx| {
        file.save_with(
            Seen::Missing,
            |setup| setup.metrics.mode = setup::Mode::PerEntry,
            cx,
        )
    });
    cx.spawn(async move |_| assert!(save.await.is_ok()))
        .detach();
    cx.run_until_parked();
    let expected = cx.read(|cx| file.read(cx).seen.clone());
    let workspace = setup::parse(FIRST.as_bytes()).unwrap().workspace;
    let save = file.update(cx, |file, cx| {
        file.save_workspace(workspace.clone(), expected, cx)
    });
    cx.spawn(async move |_| assert!(save.await.is_ok()))
        .detach();
    cx.run_until_parked();
    let saved =
        setup::parse(&std::fs::read(directory.path().join("freshkube.yaml")).unwrap()).unwrap();
    assert_eq!(saved.workspace, workspace);
    assert_eq!(saved.metrics.mode, setup::Mode::PerEntry);
}

#[gpui_kit::test]
fn a_quiet_poll_redraws_nothing(cx: &mut TestAppContext) {
    let directory = tempfile::tempdir().unwrap();
    std::fs::write(directory.path().join("freshkube.yaml"), FIRST).unwrap();
    let (handle, _reader) = mount(cx, directory.path(), false);
    let renders = probe::count("setup.reader.render");
    let reads = probe::count("setup.read");
    for _ in 0..3 {
        advance(cx, POLL);
        cx.update_window(handle, |_, window, cx| window.simulate_next_frame(cx))
            .unwrap();
    }
    assert_eq!(probe::count("setup.reader.render"), renders);
    assert_eq!(probe::count("setup.read"), reads);
}

#[gpui_kit::test]
fn a_change_reads_once_after_two_stats_agree(cx: &mut TestAppContext) {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("freshkube.yaml");
    std::fs::write(&path, FIRST).unwrap();
    let (_, reader) = mount(cx, directory.path(), false);
    let reads = probe::count("setup.read");
    let modified = std::fs::metadata(&path).unwrap().modified().unwrap();
    std::fs::write(&path, "version:").unwrap();
    keep_modified(&path, modified);
    advance(cx, POLL);
    advance(cx, DEBOUNCE / 2);
    std::fs::write(&path, SECOND).unwrap();
    // Model a coarse timestamp: the final edit has the accepted file's
    // length and mtime, but must still restart the quiet interval.
    keep_modified(&path, modified);
    assert_eq!(FIRST.len(), SECOND.len());
    advance(cx, DEBOUNCE / 2);
    assert_eq!(probe::count("setup.read"), reads);
    advance(cx, DEBOUNCE);
    assert_eq!(probe::count("setup.read"), reads + 1);
    cx.update(|cx| {
        assert_eq!(reader.read(cx).changes, 1);
        let file = reader.read(cx).file.read(cx);
        assert_eq!(
            file.setup.as_ref().unwrap().workspace.clusters[0].id,
            "cluster-b"
        );
        assert!(
            file.receipt
                .as_ref()
                .unwrap()
                .starts_with("Reloaded freshkube.yaml · ")
        );
    });
    advance(cx, POLL);
    assert_eq!(probe::count("setup.read"), reads + 1);
}

#[gpui_kit::test]
fn a_same_size_edit_with_the_same_mtime_is_reloaded(cx: &mut TestAppContext) {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("freshkube.yaml");
    std::fs::write(&path, FIRST).unwrap();
    let (_, reader) = mount(cx, directory.path(), false);
    let original = std::fs::metadata(&path).unwrap();
    std::fs::write(&path, SECOND).unwrap();
    keep_modified(&path, original.modified().unwrap());
    assert_eq!(std::fs::metadata(&path).unwrap().len(), original.len());
    advance(cx, POLL);
    cx.update(|cx| assert_eq!(reader.read(cx).changes, 0));
    advance(cx, DEBOUNCE);
    cx.update(|cx| {
        assert_eq!(reader.read(cx).changes, 1);
        assert_eq!(
            reader.read(cx).file.read(cx).seen,
            Seen::Bytes(SECOND.as_bytes().to_vec())
        );
    });
}

#[gpui_kit::test]
fn an_editor_can_rename_over_the_file(cx: &mut TestAppContext) {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("freshkube.yaml");
    std::fs::write(&path, FIRST).unwrap();
    let (_, reader) = mount(cx, directory.path(), false);
    let replacement = directory.path().join("editor.tmp");
    std::fs::write(&replacement, SECOND).unwrap();
    // Force the collision on every platform, including Windows where
    // FileStat has no inode to distinguish these same-length files.
    let original = std::fs::metadata(&path).unwrap();
    keep_modified(&replacement, original.modified().unwrap());
    assert_eq!(FIRST.len(), SECOND.len());
    std::fs::rename(replacement, &path).unwrap();
    advance(cx, POLL);
    advance(cx, DEBOUNCE);
    cx.update(|cx| assert_eq!(reader.read(cx).changes, 1));
}

#[gpui_kit::test]
fn an_own_write_during_debounce_reads_nothing(cx: &mut TestAppContext) {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("freshkube.yaml");
    std::fs::write(&path, FIRST).unwrap();
    let (_, reader) = mount(cx, directory.path(), false);
    let file = model(&reader, cx);
    std::fs::write(&path, SECOND).unwrap();
    advance(cx, POLL);
    let stat = FileStat::read(&path);
    file.update(cx, |file, _| {
        file.record_write(
            setup::parse(SECOND.as_bytes()).unwrap(),
            Seen::Bytes(SECOND.as_bytes().to_vec()),
            stat,
        )
    });
    let reads = probe::count("setup.read");
    advance(cx, DEBOUNCE);
    advance(cx, POLL);
    assert_eq!(probe::count("setup.read"), reads);
    cx.update(|cx| assert_eq!(reader.read(cx).changes, 0));
}

#[gpui_kit::test]
fn broken_and_deleted_files_keep_the_last_good_setup(cx: &mut TestAppContext) {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("freshkube.yaml");
    std::fs::write(&path, FIRST).unwrap();
    let (_, reader) = mount(cx, directory.path(), false);
    let file = model(&reader, cx);
    std::fs::write(&path, "version: [").unwrap();
    advance(cx, POLL);
    advance(cx, DEBOUNCE);
    cx.update(|cx| {
        let file = file.read(cx);
        assert_eq!(file.seen, Seen::Bytes(FIRST.as_bytes().to_vec()));
        assert_eq!(
            file.setup.as_ref().unwrap().workspace.clusters[0].id,
            "cluster-a"
        );
        assert!(
            file.problem
                .as_ref()
                .unwrap()
                .contains("invalid YAML at line")
        );
        assert!(
            file.problem
                .as_ref()
                .unwrap()
                .ends_with("The last good setup stays in use.")
        );
    });
    std::fs::write(&path, SECOND).unwrap();
    advance(cx, POLL);
    advance(cx, DEBOUNCE);
    cx.update(|cx| {
        assert!(file.read(cx).problem.is_none());
        assert_eq!(
            file.read(cx).setup.as_ref().unwrap().workspace.clusters[0].id,
            "cluster-b"
        );
    });
    std::fs::remove_file(&path).unwrap();
    advance(cx, POLL);
    advance(cx, DEBOUNCE);
    cx.update(|cx| {
        assert_eq!(
            file.read(cx).setup.as_ref().unwrap().workspace.clusters[0].id,
            "cluster-b"
        );
        assert!(file.read(cx).problem.as_ref().unwrap().contains("is gone"));
    });
}

#[gpui_kit::test]
fn a_broken_file_at_launch_has_no_setup(cx: &mut TestAppContext) {
    let directory = tempfile::tempdir().unwrap();
    std::fs::write(directory.path().join("freshkube.yaml"), "version: [").unwrap();
    let (_, reader) = mount(cx, directory.path(), false);
    let file = model(&reader, cx);
    cx.update(|cx| {
        assert!(file.read(cx).setup.is_none());
        assert_eq!(file.read(cx).seen, Seen::Missing);
        assert!(file.read(cx).problem.is_some());
    });
}

#[gpui_kit::test]
fn a_comment_edit_changes_neither_setup_nor_events(cx: &mut TestAppContext) {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("freshkube.yaml");
    std::fs::write(&path, FIRST).unwrap();
    let (_, reader) = mount(cx, directory.path(), false);
    std::fs::write(&path, format!("# a comment\n{FIRST}")).unwrap();
    advance(cx, POLL);
    advance(cx, DEBOUNCE);
    cx.update(|cx| assert_eq!(reader.read(cx).changes, 0));
}

#[gpui_kit::test]
fn examples_never_read_or_poll(cx: &mut TestAppContext) {
    let directory = tempfile::tempdir().unwrap();
    std::fs::write(directory.path().join("freshkube.yaml"), FIRST).unwrap();
    let stats = probe::count("setup.stat");
    let reads = probe::count("setup.read");
    let (_, reader) = mount(cx, directory.path(), true);
    for _ in 0..3 {
        advance(cx, POLL);
    }
    assert_eq!(probe::count("setup.stat"), stats);
    assert_eq!(probe::count("setup.read"), reads);
    cx.update(|cx| assert!(reader.read(cx).file.read(cx).setup.is_none()));
}

/// A workspace read from `workspace.json`, with a key this version
/// doesn't know.
fn legacy_workspace() -> freshkube_core::workspace::Workspace {
    serde_json::from_str(
        r#"{"clusters":[{"id":"cluster-a","role":"core","context":"cluster-a"}],"future_key":"kept"}"#,
    )
    .unwrap()
}

#[gpui_kit::test]
async fn the_first_workspace_save_moves_unknown_keys_beside_the_workspace(cx: &mut TestAppContext) {
    use freshkube_core::setup::{self, Setup};
    cx.executor().allow_parking();
    let directory = tempfile::tempdir().unwrap();
    let (_, reader) = mount(cx, directory.path(), false);
    let file = model(&reader, cx);
    let workspace = legacy_workspace();
    let save = cx.update(|cx| {
        file.update(cx, |file, cx| {
            file.save_workspace(workspace.clone(), Seen::Missing, cx)
        })
    });
    save.await.unwrap();
    let text = std::fs::read_to_string(directory.path().join("freshkube.yaml")).unwrap();
    // As `Setup::new` writes it, once, at the top level.
    assert_eq!(text.matches("future_key").count(), 1, "{text}");
    assert_eq!(
        setup::parse(text.as_bytes()).unwrap(),
        Setup::new(workspace)
    );
}

#[gpui_kit::test]
async fn a_first_coroot_save_writes_only_that_block_over_the_default_workspace(
    cx: &mut TestAppContext,
) {
    use freshkube_core::setup::{self, Coroot, SignIn};
    cx.executor().allow_parking();
    let directory = tempfile::tempdir().unwrap();
    let (_, reader) = mount(cx, directory.path(), false);
    let file = model(&reader, cx);
    let block = Coroot::new("https://coroot.acme.example", SignIn::Anonymous);
    let expected = block.clone();
    let save = cx.update(|cx| {
        file.update(cx, |file, cx| {
            file.save_with(Seen::Missing, move |setup| setup.coroot = Some(block), cx)
        })
    });
    save.await.unwrap();
    let written =
        setup::parse(&std::fs::read(directory.path().join("freshkube.yaml")).unwrap()).unwrap();
    assert_eq!(written.coroot, Some(expected));
    assert_eq!(
        written.workspace,
        freshkube_core::workspace::Workspace::default()
    );
    assert!(written.supply_chain.is_none());
}

#[gpui_kit::test]
async fn problem_changes_and_same_setup_recovery_advance_once(cx: &mut TestAppContext) {
    cx.executor().allow_parking();
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("freshkube.yaml");
    std::fs::write(&path, "version: [").unwrap();
    let (_, reader) = mount(cx, directory.path(), false);
    let file = model(&reader, cx);
    let revision = cx.read(|cx| reader.read(cx).changes);
    assert_eq!(
        revision, 0,
        "initial diagnostics publish through Ready, before any change event"
    );
    file.update(cx, |file, cx| file.reload(cx)).await;
    assert_eq!(cx.read(|cx| reader.read(cx).changes), revision);

    std::fs::write(&path, "version: 999").unwrap();
    file.update(cx, |file, cx| file.reload(cx)).await;
    assert_eq!(cx.read(|cx| reader.read(cx).changes), revision + 1);
    assert!(cx.read(|cx| file.read(cx).setup.is_none()));

    std::fs::write(&path, FIRST).unwrap();
    file.update(cx, |file, cx| file.reload(cx)).await;
    assert_eq!(cx.read(|cx| reader.read(cx).changes), revision + 2);
    std::fs::write(&path, "version: [").unwrap();
    advance(cx, POLL);
    advance(cx, DEBOUNCE);
    assert_eq!(cx.read(|cx| reader.read(cx).changes), revision + 3);

    // Polling the exact last-good setup clears the problem even though
    // neither the parsed setup nor its last-good bytes have changed.
    std::fs::write(&path, FIRST).unwrap();
    advance(cx, POLL);
    advance(cx, DEBOUNCE);
    assert_eq!(cx.read(|cx| reader.read(cx).changes), revision + 4);
    assert!(cx.read(|cx| file.read(cx).problem.is_none()));
    advance(cx, POLL);
    assert_eq!(cx.read(|cx| reader.read(cx).changes), revision + 4);
}

#[gpui_kit::test]
async fn saving_the_same_setup_after_deletion_clears_the_notice(cx: &mut TestAppContext) {
    cx.executor().allow_parking();
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("freshkube.yaml");
    std::fs::write(&path, FIRST).unwrap();
    let (_, reader) = mount(cx, directory.path(), false);
    let file = model(&reader, cx);
    std::fs::remove_file(&path).unwrap();
    file.update(cx, |file, cx| file.reload(cx)).await;
    let before = cx.read(|cx| file.read(cx).setup_revision());
    assert!(cx.read(|cx| file.read(cx).problem.is_some()));
    file.update(cx, |file, cx| file.save_with(Seen::Missing, |_| {}, cx))
        .await
        .unwrap();
    cx.read(|cx| {
        assert_eq!(file.read(cx).setup_revision(), before);
        assert!(file.read(cx).problem.is_none());
    });
}

#[gpui_kit::test]
async fn an_identical_explicit_reload_advances_the_setup_revision(cx: &mut TestAppContext) {
    cx.executor().allow_parking();
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("freshkube.yaml");
    std::fs::write(&path, FIRST).unwrap();
    let (_, reader) = mount(cx, directory.path(), false);
    let file = model(&reader, cx);
    let before = cx.read(|cx| file.read(cx).setup_revision());
    assert!(before > 0);
    let reload = file.update(cx, |file, cx| file.reload(cx));
    reload.await;
    assert_eq!(cx.read(|cx| file.read(cx).setup_revision()), before + 1);
    assert_eq!(
        cx.read(|cx| file.read(cx).setup.clone()),
        Some(setup::parse(FIRST.as_bytes()).unwrap())
    );
    // Quiet polls and a fatal refusal preserve the effective setup revision.
    advance(cx, POLL * 2);
    assert_eq!(cx.read(|cx| file.read(cx).setup_revision()), before + 1);
    std::fs::write(&path, "version: [broken").unwrap();
    file.update(cx, |file, cx| file.reload(cx)).await;
    assert_eq!(cx.read(|cx| file.read(cx).setup_revision()), before + 1);
    // A changed successful reload advances once, rather than once for
    // the changed bytes and again for the explicit action.
    std::fs::write(&path, SECOND).unwrap();
    file.update(cx, |file, cx| file.reload(cx)).await;
    assert_eq!(cx.read(|cx| file.read(cx).setup_revision()), before + 2);
}
#[gpui_kit::test]
async fn recovery_keeps_bytes_refuses_saves_and_updates_notices_without_read_invalidation(
    cx: &mut TestAppContext,
) {
    cx.executor().allow_parking();
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("freshkube.yaml");
    let partial = format!("{FIRST}destinations: [{{name: '', entry: cluster-a}}]\n");
    std::fs::write(&path, &partial).unwrap();
    let (_, reader) = mount(cx, directory.path(), false);
    let file = model(&reader, cx);
    let before = cx.read(|cx| {
        let file = file.read(cx);
        assert!(file.setup.is_some());
        assert_eq!(file.seen, Seen::Bytes(partial.as_bytes().to_vec()));
        assert!(file.notice().unwrap().contains("skipped destinations[0]"));
        assert!(file.saves_refused());
        file.setup_revision()
    });
    let expected = cx.read(|cx| file.read(cx).seen.clone());
    let error = file
        .update(cx, |file, cx| file.save_with(expected, |_| {}, cx))
        .await
        .unwrap_err();
    assert_eq!(error.to_string(), SKIPPED_SAVE);
    assert_eq!(std::fs::read_to_string(&path).unwrap(), partial);
    assert!(!path.with_extension("yaml.bak").exists());
    // A warning-only poll keeps effective reads alive.
    std::fs::write(
        &path,
        format!("{FIRST}destinations: [{{server: invalid-endpoint, entry: cluster-a}}]\n"),
    )
    .unwrap();
    advance(cx, POLL);
    advance(cx, DEBOUNCE);
    assert_eq!(cx.read(|cx| file.read(cx).setup_revision()), before);
    assert!(cx.read(|cx| file.read(cx).notice().unwrap().contains("http or https")));
    // A later fatal edit only shows today's refusal sentence.
    std::fs::write(&path, "version: 999").unwrap();
    advance(cx, POLL);
    advance(cx, DEBOUNCE);
    assert_eq!(cx.read(|cx| file.read(cx).setup_revision()), before);
    cx.read(|cx| {
        let file = file.read(cx);
        assert!(file.warnings.is_empty());
        assert!(
            file.notice()
                .unwrap()
                .starts_with("freshkube.yaml isn't used:")
        );
        assert!(file.setup.is_some());
    });
    std::fs::write(&path, FIRST).unwrap();
    advance(cx, POLL);
    advance(cx, DEBOUNCE);
    assert!(cx.read(|cx| file.read(cx).notice().is_none()));
    assert_eq!(cx.read(|cx| file.read(cx).setup_revision()), before);
    file.update(cx, |file, cx| file.reload(cx)).await;
    assert_eq!(cx.read(|cx| file.read(cx).setup_revision()), before + 1);
}

#[gpui_kit::test]
async fn all_skipped_and_deliberately_empty_have_distinct_effective_revisions(
    cx: &mut TestAppContext,
) {
    cx.executor().allow_parking();
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("freshkube.yaml");
    std::fs::write(&path, "version: 1\nclusters: [[]]").unwrap();
    let (_, reader) = mount(cx, directory.path(), false);
    let file = model(&reader, cx);
    let before = cx.read(|cx| {
        let file = file.read(cx);
        assert_eq!(file.setup, Some(Setup::default()));
        assert!(file.source_recovery.all_clusters_skipped);
        file.setup_revision()
    });
    std::fs::write(&path, "version: 1\nclusters: []").unwrap();
    advance(cx, POLL);
    advance(cx, DEBOUNCE);
    cx.read(|cx| {
        let file = file.read(cx);
        assert_eq!(file.setup, Some(Setup::default()));
        assert!(!file.source_recovery.all_clusters_skipped);
        assert!(file.warnings.is_empty());
        assert_eq!(file.setup_revision(), before + 1);
    });
}
