use super::*;

const LEVELS: [Protection; 4] = Protection::ALL;

fn file_with(level: Protection) -> String {
    format!(
        "version: 1\nclusters:\n  - id: prod\n    role: core\n    context: prod-ctx\n    protection: {}\n",
        level.key()
    )
}

fn entry(id: &str) -> Subject {
    Subject::Entry(id.to_owned())
}

fn in_file(text: &str, subject: Subject) -> Result<Protection, Unreadable> {
    SetupLevels::new("freshkube.yaml", subject).level_in(Some(text.as_bytes()))
}

/// The file may only tighten: of all sixteen held and file levels, a
/// stricter file refuses with its own level and anything else sends under
/// the held one, never a looser file's.
#[test]
fn safety_g28_the_file_only_ever_tightens_the_level_a_write_is_sent_under() {
    for held in LEVELS {
        for file in LEVELS {
            let fresh = in_file(&file_with(file), entry("prod"));
            assert_eq!(fresh, Ok(file));
            let decided = at_send(held, fresh);
            if file < held {
                assert_eq!(
                    decided,
                    Err(SendRefusal::Stricter(file)),
                    "{held:?} {file:?}"
                );
            } else {
                assert_eq!(decided, Ok(held), "{held:?} {file:?}");
            }
        }
    }
}

/// A file that can't say refuses at every held level, `open` included: it
/// never falls back to the held level.
#[test]
fn safety_g28_a_file_that_cannot_say_refuses_at_every_held_level() {
    let cases = [
        (None, entry("prod"), Unreadable::Missing),
        (
            Some("clusters: [[[".to_owned()),
            entry("prod"),
            Unreadable::Invalid,
        ),
        (
            Some(file_with(Protection::Open)),
            entry("gone"),
            Unreadable::EntryGone,
        ),
        // A typo on the entry's own row skips it, so it is not listed.
        (
            Some(file_with(Protection::Open).replace("open", "opne")),
            entry("prod"),
            Unreadable::EntryGone,
        ),
    ];
    for (text, subject, why) in cases {
        let fresh = SetupLevels::new("freshkube.yaml", subject)
            .level_in(text.as_deref().map(str::as_bytes));
        assert_eq!(fresh, Err(why));
        for held in LEVELS {
            assert_eq!(at_send(held, fresh), Err(SendRefusal::Unreadable(why)));
        }
    }
    // A file too large, one that is not a regular file, and a live window with
    // no file at all can't say either.
    let dir = tempfile::tempdir().unwrap();
    let large = dir.path().join("large.yaml");
    std::fs::write(&large, vec![b'#'; MAX_BYTES as usize + 1]).unwrap();
    let directory = dir.path().join("directory");
    std::fs::create_dir(&directory).unwrap();
    for levels in [
        SetupLevels::new(large, entry("prod")),
        SetupLevels::new(directory, entry("prod")),
        SetupLevels::unavailable(entry("prod")),
    ] {
        let fresh = levels.read();
        assert_eq!(fresh, Err(Unreadable::NotReadable));
        for held in LEVELS {
            assert_eq!(
                at_send(held, fresh),
                Err(SendRefusal::Unreadable(Unreadable::NotReadable))
            );
        }
    }
}

/// A row skipped for a wrong `protection` locks its context read-only, so a
/// window on that context is refused, not left at `guarded`.
#[test]
fn safety_g28_a_context_whose_row_was_skipped_reads_as_read_only() {
    let text = file_with(Protection::Open).replace("open", "opne");
    assert_eq!(
        in_file(&text, Subject::Context("prod-ctx".into())),
        Ok(Protection::ReadOnly)
    );
    assert_eq!(
        at_send(
            Protection::Open,
            in_file(&text, Subject::Context("prod-ctx".into()))
        ),
        Err(SendRefusal::Stricter(Protection::ReadOnly))
    );
}

/// A window on a context no entry named catches a file that now has an entry
/// naming it, at every file level, and stays `guarded` when none does or the
/// file is missing.
#[test]
fn safety_g28_a_context_subject_sees_an_entry_that_now_names_it() {
    let context = || Subject::Context("prod-ctx".into());
    for held in LEVELS {
        for file in LEVELS {
            let fresh = in_file(&file_with(file), context());
            assert_eq!(fresh, Ok(file));
            let decided = at_send(held, fresh);
            if file < held {
                assert_eq!(decided, Err(SendRefusal::Stricter(file)));
            } else {
                assert_eq!(decided, Ok(held));
            }
        }
    }
    // Another entry's context, and a missing file, are guarded.
    assert_eq!(
        in_file(
            &file_with(Protection::ReadOnly),
            Subject::Context("elsewhere".into())
        ),
        Ok(Protection::Guarded)
    );
    assert_eq!(
        SetupLevels::new("freshkube.yaml", context()).level_in(None),
        Ok(Protection::Guarded)
    );
    // So an `open` window on a context nothing names is held to guarded, which
    // is stricter than what it held.
    assert_eq!(
        at_send(Protection::Open, Ok(Protection::Guarded)),
        Err(SendRefusal::Stricter(Protection::Guarded))
    );
}

/// The stricter of two entries on one context applies to an entry's subject.
#[test]
fn an_entrys_level_is_the_stricter_of_the_entries_on_its_context() {
    let text = "version: 1\nclusters:\n  - {id: a, role: core, context: shared, protection: open}\n  - {id: b, role: cicd, context: shared, protection: protected}\n";
    assert_eq!(in_file(text, entry("a")), Ok(Protection::Protected));
    assert_eq!(in_file(text, entry("b")), Ok(Protection::Protected));
}

/// One unrelated bad row does not refuse the rest of the file.
#[test]
fn an_unrelated_skipped_row_leaves_the_entry_readable() {
    let text = "version: 1\nclusters:\n  - {id: a, role: core, context: a, protection: protected}\n  - {id: z, role: core, context: z, protection: nonsense}\n";
    assert_eq!(in_file(text, entry("a")), Ok(Protection::Protected));
}

/// The notice names the level only: nothing of the file, a path or a parse
/// error.
#[test]
fn the_notice_names_the_level_only() {
    assert_eq!(
        SendRefusal::Stricter(Protection::Protected).text(),
        "This cluster is now protected; review again."
    );
    assert_eq!(
        SendRefusal::Stricter(Protection::ReadOnly).text(),
        "This cluster is now read-only; review again."
    );
    for why in [
        Unreadable::Missing,
        Unreadable::NotReadable,
        Unreadable::Invalid,
    ] {
        let text = SendRefusal::Unreadable(why).text();
        assert_eq!(
            text,
            "This cluster's level can't be read from freshkube.yaml; review again."
        );
    }
    assert!(
        SendRefusal::Unreadable(Unreadable::EntryGone)
            .text()
            .contains("no longer in freshkube.yaml")
    );
}

/// The read itself, on real files: a missing file, a directory, an oversized
/// file and a file that changes between two reads.
#[test]
fn read_follows_the_file_on_disk() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("freshkube.yaml");
    let levels = SetupLevels::new(&file, entry("prod"));
    assert_eq!(levels.read(), Err(Unreadable::Missing));
    std::fs::write(&file, file_with(Protection::Guarded)).unwrap();
    assert_eq!(levels.read(), Ok(Protection::Guarded));
    std::fs::write(&file, file_with(Protection::ReadOnly)).unwrap();
    assert_eq!(levels.read(), Ok(Protection::ReadOnly));
    std::fs::write(&file, vec![b'#'; MAX_BYTES as usize + 1]).unwrap();
    assert_eq!(levels.read(), Err(Unreadable::NotReadable));
    std::fs::remove_file(&file).unwrap();
    std::fs::create_dir(&file).unwrap();
    assert_eq!(levels.read(), Err(Unreadable::NotReadable));
    // A live window with no setup file can't say, for an entry or a context.
    for subject in [entry("prod"), Subject::Context("prod-ctx".into())] {
        assert_eq!(
            SetupLevels::unavailable(subject).read(),
            Err(Unreadable::NotReadable)
        );
    }
    // A directory is no file for a context either: not "missing".
    assert_eq!(
        SetupLevels::new(&file, Subject::Context("prod-ctx".into())).read(),
        Err(Unreadable::NotReadable)
    );
}

/// A read that doesn't answer in time is a file that can't say, and one that
/// does answer is passed on.
#[tokio::test]
async fn a_read_that_takes_too_long_cannot_say() {
    let slow = read_within(std::time::Duration::from_millis(20), || {
        std::thread::sleep(std::time::Duration::from_millis(400));
        Ok(Protection::Open)
    })
    .await;
    assert_eq!(slow, Err(Unreadable::NotReadable));
    let quick = read_within(std::time::Duration::from_secs(5), || Ok(Protection::Open)).await;
    assert_eq!(quick, Ok(Protection::Open));
}

/// Entries the file doesn't hold, for a legacy user, have no level to read.
#[test]
fn a_legacy_user_has_nothing_to_read_for_a_destination() {
    let dir = tempfile::tempdir().unwrap();
    let levels = SetupLevels::legacy(dir.path().join("freshkube.yaml"), "prod-ctx");
    assert_eq!(
        levels.read_with(&["prod-ams".into()]),
        Ok((Protection::Guarded, vec![None]))
    );
    // With the entries in the file, an unlisted one is gone.
    let listed = SetupLevels::new(dir.path().join("freshkube.yaml"), entry("prod"));
    assert_eq!(
        listed.read_with(&["prod-ams".into()]),
        Err(Unreadable::Missing)
    );
}

// The team's floors reach the level a write is sent under (#849, slice 5C).
mod team_floors {
    use std::path::Path;

    use super::*;
    use crate::team::TeamFloors;
    use crate::team::floors::tests::{TEAM as TEAM_FILE, applied, saved_text};

    /// A setup file whose `prod` entry is `open` on `prod-ctx`, and the
    /// team's file beside it.
    fn directory_with(setup: &str) -> tempfile::TempDir {
        let directory = tempfile::tempdir().unwrap();
        std::fs::write(directory.path().join("freshkube.yaml"), setup).unwrap();
        directory
    }

    fn levels(directory: &Path, subject: Subject) -> SetupLevels {
        SetupLevels::new(directory.join("freshkube.yaml"), subject)
    }

    /// The team file of a team that holds `prod-ams` (read-only) on `ctx`.
    fn team_holding(ctx: &str) -> String {
        saved_text(&applied(TEAM_FILE, &[("prod-ams", ctx)]))
    }

    fn floors_holding(ctx: &str) -> TeamFloors {
        TeamFloors::from_membership(&applied(TEAM_FILE, &[("prod-ams", ctx)]))
    }

    #[test]
    fn safety_g32_a_team_floor_holds_the_level_a_write_is_sent_under() {
        let directory = directory_with(&file_with(Protection::Open));
        // The file says open and the window held open; the team holds the
        // context read-only.
        let open = levels(directory.path(), entry("prod"));
        assert_eq!(open.read(), Ok(Protection::Open));
        let held = open.with_team(floors_holding("prod-ctx"), None);
        assert_eq!(held.read(), Ok(Protection::ReadOnly));
        assert_eq!(
            at_send(Protection::Open, held.read()),
            Err(SendRefusal::Stricter(Protection::ReadOnly))
        );
        // A context subject, with no entry for it, is held too.
        let by_context = levels(directory.path(), Subject::Context("elsewhere".into()))
            .with_team(floors_holding("elsewhere"), None);
        assert_eq!(by_context.read(), Ok(Protection::ReadOnly));
    }

    #[test]
    fn safety_g32_a_floor_that_appears_while_the_dialog_is_open_stops_the_send() {
        let directory = directory_with(&file_with(Protection::Open));
        let team_file = directory.path().join("team.json");
        // The dialog opened with no team: the window held no floor.
        let at_opening = levels(directory.path(), entry("prod"))
            .with_team(TeamFloors::none(), Some(team_file.clone()));
        assert_eq!(at_opening.read(), Ok(Protection::Open));
        // Joining and applying in Settings saved a team meanwhile.
        std::fs::write(&team_file, team_holding("prod-ctx")).unwrap();
        assert_eq!(at_opening.read(), Ok(Protection::ReadOnly));
        assert_eq!(
            at_send(Protection::Open, at_opening.read()),
            Err(SendRefusal::Stricter(Protection::ReadOnly))
        );
    }

    #[test]
    fn safety_g32_a_floor_that_disappears_while_the_dialog_is_open_does_not_loosen_it() {
        let directory = directory_with(&file_with(Protection::Open));
        let team_file = directory.path().join("team.json");
        std::fs::write(&team_file, team_holding("prod-ctx")).unwrap();
        let at_opening = levels(directory.path(), entry("prod"))
            .with_team(floors_holding("prod-ctx"), Some(team_file.clone()));
        assert_eq!(at_opening.read(), Ok(Protection::ReadOnly));
        // The person left the team meanwhile: what the dialog held stands.
        std::fs::remove_file(&team_file).unwrap();
        assert_eq!(at_opening.read(), Ok(Protection::ReadOnly));
    }

    #[test]
    fn safety_g32_a_team_file_that_cannot_be_read_holds_every_context_read_only() {
        let directory = directory_with(&file_with(Protection::Open));
        let team_file = directory.path().join("team.json");
        for text in [
            "{ not json".to_string(),
            r#"{"version":99,"membership":null}"#.to_string(),
            {
                // An address joining refuses is not taken up.
                let mut value: serde_json::Value =
                    serde_json::from_str(&team_holding("prod-ctx")).unwrap();
                value["membership"]["source"]["address"] = "http://git.acme.example/x.git".into();
                value.to_string()
            },
        ] {
            std::fs::write(&team_file, text).unwrap();
            for subject in [entry("prod"), Subject::Context("elsewhere".into())] {
                let held = levels(directory.path(), subject)
                    .with_team(TeamFloors::none(), Some(team_file.clone()));
                assert_eq!(held.read(), Ok(Protection::ReadOnly));
                assert_eq!(
                    at_send(Protection::Open, held.read()),
                    Err(SendRefusal::Stricter(Protection::ReadOnly))
                );
            }
        }
        // A missing file is no team: the file's own level.
        std::fs::remove_file(&team_file).unwrap();
        let held =
            levels(directory.path(), entry("prod")).with_team(TeamFloors::none(), Some(team_file));
        assert_eq!(held.read(), Ok(Protection::Open));
    }

    #[test]
    fn safety_g32_a_floor_is_never_a_looser_level_than_the_file_says() {
        // The setup file holds read-only; a team that holds the same context
        // at protected does not move it up to protected.
        let directory = directory_with(&file_with(Protection::ReadOnly));
        let floors = TeamFloors::from_membership(&applied(TEAM_FILE, &[("stage-lon", "prod-ctx")]));
        let held = levels(directory.path(), entry("prod")).with_team(floors.clone(), None);
        assert_eq!(held.read(), Ok(Protection::ReadOnly));
        // The same by context, where only `level_of` decides.
        let by_context =
            levels(directory.path(), Subject::Context("prod-ctx".into())).with_team(floors, None);
        assert_eq!(by_context.read(), Ok(Protection::ReadOnly));
        // And the window's held level is never loosened by any of it.
        for held_level in LEVELS {
            let decided = at_send(held_level, held.read());
            assert!(
                matches!(decided, Ok(level) if level == held_level.stricter(Protection::ReadOnly))
                    || decided == Err(SendRefusal::Stricter(Protection::ReadOnly))
            );
        }
    }
}
