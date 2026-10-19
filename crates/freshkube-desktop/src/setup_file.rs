//! The shell's last good `freshkube.yaml`, and the file's current problem.
//! File work runs on GPUI's background executor; timers use its clock.
use std::{io::ErrorKind, path::PathBuf, time::Duration};

use freshkube_core::{
    BoundedReadError, read_bounded_regular_file,
    setup::{self, Setup},
    workspace::{MAX_BYTES, Seen},
};
use gpui_kit::{AppContext, Context, EventEmitter, Task};

const POLL: Duration = Duration::from_secs(2);
const DEBOUNCE: Duration = Duration::from_millis(300);

/// Includes the inode because editors commonly replace a file by rename.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum FileStat {
    Missing,
    Unreadable(ErrorKind),
    Present {
        len: u64,
        modified: Option<std::time::SystemTime>,
        regular: bool,
        #[cfg(unix)]
        inode: u64,
    },
}

impl FileStat {
    pub(crate) fn read(file: &std::path::Path) -> Self {
        freshkube_probe::probe::hit("setup.stat");
        match std::fs::metadata(file) {
            Ok(metadata) => Self::Present {
                len: metadata.len(),
                modified: metadata.modified().ok(),
                regular: metadata.is_file(),
                #[cfg(unix)]
                inode: std::os::unix::fs::MetadataExt::ino(&metadata),
            },
            Err(error) if error.kind() == ErrorKind::NotFound => Self::Missing,
            Err(error) => Self::Unreadable(error.kind()),
        }
    }
}

pub(crate) struct SetupFile {
    pub(crate) setup: Option<Setup>,
    /// Bytes of the last good setup, never the broken edit on disk.
    pub(crate) seen: Seen,
    pub(crate) problem: Option<String>,
    pub(crate) receipt: Option<String>,
    stat: FileStat,
    write_generation: u64,
    _poll: Option<Task<()>>,
}

pub(crate) enum Event {
    Ready,
    Changed { setup_changed: bool },
}

impl EventEmitter<Event> for SetupFile {}

impl SetupFile {
    pub(crate) fn new(preferences: Option<PathBuf>, example: bool, cx: &mut Context<Self>) -> Self {
        let mut this = Self {
            setup: None,
            seen: Seen::Missing,
            problem: None,
            receipt: None,
            stat: FileStat::Missing,
            write_generation: 0,
            _poll: None,
        };
        if let Some(preferences) = preferences.filter(|_| !example) {
            this._poll = Some(Self::watch(
                preferences.with_file_name("freshkube.yaml"),
                cx,
            ));
        }
        this
    }

    fn watch(file: PathBuf, cx: &Context<Self>) -> Task<()> {
        cx.spawn(async move |this, cx| {
            let mut initial = true;
            loop {
                if !initial {
                    cx.background_executor().timer(POLL).await;
                }
                let path = file.clone();
                let mut stat = cx
                    .background_spawn(async move { FileStat::read(&path) })
                    .await;
                let Ok(accepted) = this.update(cx, |this, _| this.stat.clone()) else {
                    return;
                };
                if !initial && stat == accepted {
                    continue;
                }
                loop {
                    if !initial {
                        cx.background_executor().timer(DEBOUNCE).await;
                        let path = file.clone();
                        let next = cx
                            .background_spawn(async move { FileStat::read(&path) })
                            .await;
                        if next != stat {
                            stat = next;
                            continue;
                        }
                    }
                    let Ok((generation, own_stat)) =
                        this.update(cx, |this, _| (this.write_generation, this.stat.clone()))
                    else {
                        return;
                    };
                    // A save can finish while the debounce is waiting.
                    if !initial && stat == own_stat {
                        break;
                    }
                    let path = file.clone();
                    let expected = stat.clone();
                    let read = cx
                        .background_spawn(async move {
                            if FileStat::read(&path) != expected {
                                return None;
                            }
                            freshkube_probe::probe::hit("setup.read");
                            let read = read_bounded_regular_file(&path, MAX_BYTES);
                            let outcome = match read {
                                Ok(bytes) => {
                                    Outcome::Parsed(setup::parse(&bytes).map(Box::new), bytes)
                                }
                                Err(BoundedReadError::NotFound) => Outcome::Missing,
                                Err(BoundedReadError::TooLarge) => {
                                    Outcome::Problem("it is larger than 256 KiB".into())
                                }
                                Err(_) => Outcome::Problem(
                                    "it could not be read as a regular file".into(),
                                ),
                            };
                            (FileStat::read(&path) == expected).then_some(outcome)
                        })
                        .await;
                    if let Some(outcome) = read {
                        if this
                            .update(cx, |this, cx| {
                                if generation == this.write_generation {
                                    this.apply(stat.clone(), outcome, initial, cx);
                                }
                            })
                            .is_err()
                        {
                            return;
                        }
                        break;
                    }
                    // The file moved during the read. Start another quiet
                    // interval before trying it again, including at launch.
                    let path = file.clone();
                    stat = cx
                        .background_spawn(async move { FileStat::read(&path) })
                        .await;
                    cx.background_executor().timer(DEBOUNCE).await;
                }
                initial = false;
            }
        })
    }

    /// Slice 4 calls this with the bytes and stat returned by its save.
    /// Invalidates an in-flight read as well as suppressing the next poll.
    #[allow(dead_code)] // The Settings write path is the next slice.
    pub(crate) fn record_write(&mut self, setup: Setup, seen: Seen, stat: FileStat) {
        self.setup = Some(setup);
        self.problem = None;
        self.seen = seen;
        self.stat = stat;
        self.write_generation = self.write_generation.wrapping_add(1);
    }

    fn apply(&mut self, stat: FileStat, outcome: Outcome, initial: bool, cx: &mut Context<Self>) {
        self.stat = stat;
        let old_problem = self.problem.clone();
        let mut setup_changed = false;
        self.problem = match outcome {
            Outcome::Parsed(Ok(setup), bytes) => {
                let setup = *setup;
                if self.seen != Seen::Bytes(bytes.clone()) {
                    setup_changed = self.setup.as_ref() != Some(&setup);
                    self.setup = Some(setup);
                    self.seen = Seen::Bytes(bytes);
                }
                None
            }
            Outcome::Parsed(Err(why), _) => Some(self.problem_words(&why.to_string())),
            Outcome::Problem(why) => Some(self.problem_words(&why)),
            Outcome::Missing if self.setup.is_some() => {
                Some("freshkube.yaml is gone. The last good setup stays in use.".into())
            }
            Outcome::Missing => None,
        };
        if initial {
            cx.emit(Event::Ready);
        } else if setup_changed || old_problem != self.problem {
            if setup_changed {
                self.receipt = Some(format!(
                    "Reloaded freshkube.yaml · {}",
                    chrono::Local::now().format("%H:%M")
                ));
            }
            cx.emit(Event::Changed { setup_changed });
        }
    }

    fn problem_words(&self, why: &str) -> String {
        let rest = if self.setup.is_some() {
            "The last good setup stays in use."
        } else {
            "Freshkube starts as with no file."
        };
        format!("freshkube.yaml isn't used: {why}. {rest}")
    }
}

enum Outcome {
    Parsed(Result<Box<Setup>, setup::Invalid>, Vec<u8>),
    Missing,
    Problem(String),
}

#[cfg(test)]
mod tests {
    use super::*;
    use freshkube_probe::probe;
    use gpui_kit::test::TestWindowExt;
    use gpui_kit::{
        AnyWindowHandle, Entity, IntoElement, Render, Subscription, TestAppContext, Window, div,
        px, size,
    };

    const FIRST: &str =
        "version: 1\nclusters:\n- {id: cluster-a, role: core, context: cluster-a}\n";
    const SECOND: &str =
        "version: 1\nclusters:\n- {id: cluster-b, role: core, context: cluster-b}\n";

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
        std::fs::write(&path, "version:").unwrap();
        advance(cx, POLL);
        advance(cx, DEBOUNCE / 2);
        std::fs::write(&path, SECOND).unwrap();
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
    fn an_editor_can_rename_over_the_file(cx: &mut TestAppContext) {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("freshkube.yaml");
        std::fs::write(&path, FIRST).unwrap();
        let (_, reader) = mount(cx, directory.path(), false);
        let replacement = directory.path().join("editor.tmp");
        std::fs::write(&replacement, SECOND).unwrap();
        // Same length, and the same mtime on filesystems that support it:
        // only the inode tells these two files apart on Unix.
        #[cfg(unix)]
        {
            let original = std::fs::metadata(&path).unwrap();
            std::fs::File::options()
                .write(true)
                .open(&replacement)
                .unwrap()
                .set_times(std::fs::FileTimes::new().set_modified(original.modified().unwrap()))
                .unwrap();
        }
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
}
