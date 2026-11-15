//! The runner reads `freshkube.yaml` again at the send step (#1055): a file
//! edited while a dialog was open is seen at Confirm, whatever the writer.

use super::*;

fn file(dir: &std::path::Path, entries: &[(&str, Protection)]) -> std::path::PathBuf {
    let path = dir.join("freshkube.yaml");
    let mut text = String::from("version: 1\nclusters:\n");
    for (id, level) in entries {
        text.push_str(&format!(
            "  - {{id: {id}, role: core, context: {id}-ctx, protection: {}}}\n",
            level.key()
        ));
    }
    std::fs::write(&path, text).unwrap();
    path
}

fn runner(
    cluster: &Cluster,
    dir: &std::path::Path,
    held: Protection,
) -> (Runner, std::path::PathBuf) {
    let path = file(
        dir,
        &[("prod-fra", held), ("prod-ams", Protection::Guarded)],
    );
    let runner = fake::runner(cluster, dir, held).rechecking(Some(SetupLevels::new(
        &path,
        Subject::Entry("prod-fra".into()),
    )));
    (runner, path)
}

fn typed(level: Protection, action: &ActionDescription) -> Confirmation {
    match level {
        Protection::Protected => Confirmation::Typed(action.subject_names()[0].to_owned()),
        _ => Confirmation::Confirmed,
    }
}

/// Every tier's writer goes through the one send step: a file edited to a
/// stricter level after the dialog was prepared sends nothing, and the level
/// the app holds follows it, so the next dialog is asked under it.
#[tokio::test(start_paused = true)]
async fn safety_g28_a_file_edited_stricter_after_the_dialog_sends_nothing_for_every_tier() {
    for (action, to) in [
        (fake::scale(3), Protection::ReadOnly),
        (fake::scale(3), Protection::Protected),
        (fake::set_image("x"), Protection::ReadOnly),
        (fake::delete_web(), Protection::ReadOnly),
        (fake::delete_web(), Protection::Protected),
    ] {
        let (cluster, dir) = (Cluster::new(), tmp());
        let (runner, path) = runner(&cluster, dir.path(), Protection::Guarded);
        let prepared = ready(&runner, action).await;
        // Edited outside the app while the dialog was open.
        file(dir.path(), &[("prod-fra", to)]);
        let _ = path;
        let finished = run(&runner, prepared, Confirmation::Confirmed).await;
        assert_eq!(
            finished.status,
            Status::NotSent(NotSent::LevelChanged(to)),
            "{to:?}"
        );
        cluster.assert_nothing_written();
        assert_eq!(
            runner.level_for(&fake::scale(3)),
            to,
            "the next dialog asks under it"
        );
        assert!(
            AuditStore::new(dir.path().join("audit"))
                .read()
                .unwrap()
                .records
                .is_empty(),
            "nothing is audited as sent"
        );
    }
}

/// A file that can't say sends nothing, at every held level.
#[tokio::test(start_paused = true)]
async fn safety_g28_a_file_that_cannot_say_sends_nothing_at_every_held_level() {
    for held in Protection::ALL {
        for edit in ["gone", "garbled", "no entry"] {
            let (cluster, dir) = (Cluster::new(), tmp());
            let (runner, path) = runner(&cluster, dir.path(), held);
            let action = fake::scale(3);
            let confirmation = typed(held, &action);
            let Verdict::Ready(prepared) = runner.preflight(action).await else {
                // Read-only blocks before there is anything to send.
                assert_eq!(held, Protection::ReadOnly);
                continue;
            };
            match edit {
                "gone" => std::fs::remove_file(&path).unwrap(),
                "garbled" => std::fs::write(&path, "clusters: [[[").unwrap(),
                _ => {
                    file(dir.path(), &[("other", Protection::Open)]);
                }
            }
            let finished = run(&runner, prepared, confirmation).await;
            let Status::NotSent(NotSent::Unchecked(text)) = &finished.status else {
                panic!("{held:?} {edit}: {:?}", finished.status)
            };
            assert!(
                text.contains("can't be read from freshkube.yaml")
                    || text.contains("no longer in freshkube.yaml"),
                "{text}"
            );
            assert!(text.ends_with("Nothing was sent."), "{text}");
            cluster.assert_nothing_written();
        }
    }
}

/// An unchanged file, and a looser one, send as before: the file only ever
/// tightens.
#[tokio::test(start_paused = true)]
async fn an_unchanged_or_looser_file_sends_as_before() {
    for to in [Protection::Guarded, Protection::Open] {
        let (cluster, dir) = (Cluster::new(), tmp());
        let (runner, _) = runner(&cluster, dir.path(), Protection::Guarded);
        let prepared = ready(&runner, fake::scale(3)).await;
        file(dir.path(), &[("prod-fra", to)]);
        let finished = run(&runner, prepared, Confirmation::Confirmed).await;
        assert_eq!(finished.headline(), Headline::Done, "{to:?}");
        assert_eq!(runner.level_for(&fake::scale(3)), Protection::Guarded);
    }
}

/// The destination an action also reaches is read from the same parse: its
/// entry edited stricter, or dropped, sends nothing.
#[tokio::test(start_paused = true)]
async fn safety_g28_a_destination_edited_stricter_or_dropped_sends_nothing() {
    let cell = LevelCell::new(Protection::Guarded);
    let action = || {
        let mut action = fake::scale(3);
        action.destinations = vec![Destination {
            entry: "prod-ams".into(),
            protection: Protection::Guarded,
            cell: Some(cell.clone()),
            covers: vec![],
        }];
        action
    };
    for (edit, expect) in [
        (
            vec![
                ("prod-fra", Protection::Guarded),
                ("prod-ams", Protection::Protected),
            ],
            Status::NotSent(NotSent::LevelChanged(Protection::Protected)),
        ),
        (
            vec![
                ("prod-fra", Protection::Guarded),
                ("prod-ams", Protection::ReadOnly),
            ],
            Status::NotSent(NotSent::LevelChanged(Protection::ReadOnly)),
        ),
    ] {
        let (cluster, dir) = (Cluster::new(), tmp());
        let (runner, _) = runner(&cluster, dir.path(), Protection::Guarded);
        let prepared = ready(&runner, action()).await;
        file(dir.path(), &edit);
        let finished = run(&runner, prepared, Confirmation::Confirmed).await;
        assert_eq!(finished.status, expect);
        cluster.assert_nothing_written();
        cell.set(Protection::Guarded);
    }
    // The destination's entry is gone from the file.
    let (cluster, dir) = (Cluster::new(), tmp());
    let (runner, _) = runner(&cluster, dir.path(), Protection::Guarded);
    let prepared = ready(&runner, action()).await;
    file(dir.path(), &[("prod-fra", Protection::Guarded)]);
    let finished = run(&runner, prepared, Confirmation::Confirmed).await;
    assert!(
        matches!(finished.status, Status::NotSent(NotSent::Unchecked(_))),
        "{:?}",
        finished.status
    );
    cluster.assert_nothing_written();
}

/// Undo is a write like any other: a file edited to `read-only` while its
/// toast was up refuses it.
#[tokio::test(start_paused = true)]
async fn safety_g28_an_undo_after_the_file_went_read_only_sends_nothing() {
    let (cluster, dir) = (Cluster::new(), tmp());
    let (runner, _) = runner(&cluster, dir.path(), Protection::Guarded);
    let prepared = ready(&runner, fake::scale(3)).await;
    let offer = run(&runner, prepared, Confirmation::Confirmed)
        .await
        .undo
        .unwrap();
    let prepared = runner
        .prepare_undo(&offer, Duration::from_secs(10))
        .await
        .unwrap();
    file(dir.path(), &[("prod-fra", Protection::ReadOnly)]);
    let writes = cluster.writes().len();
    let finished = run(&runner, Box::new(prepared), Confirmation::Confirmed).await;
    assert_eq!(
        finished.status,
        Status::NotSent(NotSent::LevelChanged(Protection::ReadOnly))
    );
    assert_eq!(cluster.writes().len(), writes, "the undo wrote nothing");
    assert_eq!(cluster.object(fake::DEPLOYMENT)["spec"]["replicas"], 3);
}
