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
fn safety_g27_the_file_only_ever_tightens_the_level_a_write_is_sent_under() {
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
fn safety_g27_a_file_that_cannot_say_refuses_at_every_held_level() {
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
}

/// A row skipped for a wrong `protection` locks its context read-only, so a
/// window on that context is refused, not left at `guarded`.
#[test]
fn safety_g27_a_context_whose_row_was_skipped_reads_as_read_only() {
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
fn safety_g27_a_context_subject_sees_an_entry_that_now_names_it() {
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
    // A directory is no file for a context either: not "missing".
    assert_eq!(
        SetupLevels::new(&file, Subject::Context("prod-ctx".into())).read(),
        Err(Unreadable::NotReadable)
    );
}
