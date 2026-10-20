//! The shell's last good `freshkube.yaml`, and the file's current problem.
//! File work runs on GPUI's background executor; timers use its clock.
use std::{io::ErrorKind, path::PathBuf, time::Duration};

use freshkube_core::{
    BoundedReadError, read_bounded_regular_file,
    setup::{self, Setup},
    workspace::{MAX_BYTES, Seen, Workspace},
};
use gpui_kit::{AppContext, Context, EventEmitter, Task};

const POLL: Duration = Duration::from_secs(2);
const DEBOUNCE: Duration = Duration::from_millis(300);

/// Metadata alone can miss same-size edits and editor renames that keep mtime.
/// Compare bounded contents too; cloning a stat shares its bytes.
#[derive(Clone, Debug, PartialEq, Eq)]
enum FileStat {
    Missing,
    Unreadable(ErrorKind),
    Present {
        len: u64,
        modified: Option<std::time::SystemTime>,
        regular: bool,
        contents: Option<std::sync::Arc<[u8]>>,
        #[cfg(unix)]
        inode: u64,
    },
}

impl FileStat {
    fn read(file: &std::path::Path) -> Self {
        freshkube_probe::probe::hit("setup.stat");
        match std::fs::metadata(file) {
            Ok(metadata) => Self::Present {
                len: metadata.len(),
                modified: metadata.modified().ok(),
                regular: metadata.is_file(),
                contents: if metadata.is_file() && metadata.len() <= MAX_BYTES {
                    read_bounded_regular_file(file, MAX_BYTES)
                        .ok()
                        .map(Into::into)
                } else {
                    None
                },
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
    receipt_label: String,
    file: Option<PathBuf>,
    last_written: Option<Seen>,
    pub(crate) coroot_reconnect: bool,
    stat: FileStat,
    write_generation: u64,
    writing: bool,
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
            receipt_label: String::new(),
            file: preferences
                .as_ref()
                .filter(|_| !example)
                .map(|path| path.with_file_name("freshkube.yaml")),
            last_written: None,
            coroot_reconnect: true,
            stat: FileStat::Missing,
            write_generation: 0,
            writing: false,
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
            let preferences = file.with_file_name("preferences.json");
            let reconnect = cx
                .background_spawn(async move {
                    freshkube_prefs::Document::<serde_json::Map<String, serde_json::Value>>::at(
                        &preferences,
                        freshkube_prefs::Layout::Pretty,
                        0o666,
                    )
                    .limited_to(MAX_BYTES)
                    .load()
                    .value
                    .get("coroot_reconnect")
                    .and_then(serde_json::Value::as_bool)
                    .unwrap_or(true)
                })
                .await;
            if this
                .update(cx, |this, _| this.coroot_reconnect = reconnect)
                .is_err()
            {
                return;
            }
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
                                if generation == this.write_generation && !this.writing {
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

    /// Invalidates an in-flight read as well as suppressing the next poll.
    fn record_write(&mut self, setup: Setup, seen: Seen, stat: FileStat) {
        self.setup = Some(setup);
        self.problem = None;
        self.last_written = Some(seen.clone());
        self.seen = seen;
        self.stat = stat;
        self.write_generation = self.write_generation.wrapping_add(1);
    }

    pub(crate) fn is_missing(&self) -> bool {
        self.stat == FileStat::Missing
    }

    pub(crate) fn saves_refused(&self) -> bool {
        self.problem.is_some() && !self.is_missing()
    }

    /// The compact status text; the full receipt stays in its tooltip.
    pub(crate) fn receipt_label(&self) -> &str {
        &self.receipt_label
    }

    fn set_receipt(&mut self, action: &str, backup: bool) {
        let now = chrono::Local::now();
        let time = now.format("%H:%M");
        let short_note = if backup { " · backup .bak" } else { "" };
        let note = if backup {
            " · the hand-edited file is freshkube.yaml.bak"
        } else {
            ""
        };
        self.receipt_label = format!("{action}{short_note} · {time}");
        self.receipt = Some(format!("{action} freshkube.yaml{note} · {time}"));
    }

    /// Saves a Settings edit without replacing any of the setup's other parts.
    /// The page retains the delivery task until the write has answered.
    pub(crate) fn save_workspace(
        &mut self,
        workspace: Workspace,
        expected: Seen,
        cx: &mut Context<Self>,
    ) -> Task<Result<(), SaveError>> {
        let missing = self.setup.is_none();
        self.save_with(
            expected,
            move |setup| {
                if missing {
                    *setup = Setup::new(workspace);
                } else {
                    setup.workspace = workspace;
                }
            },
            cx,
        )
    }

    /// Saves one Settings edit to the setup, keeping its other parts.
    pub(crate) fn save_with(
        &mut self,
        expected: Seen,
        edit: impl FnOnce(&mut Setup) + Send + 'static,
        cx: &mut Context<Self>,
    ) -> Task<Result<(), SaveError>> {
        let Some(file) = self.file.clone() else {
            return Task::ready(Err(SaveError::Failed("There is nowhere to save".into())));
        };
        self.writing = true;
        self.write_generation = self.write_generation.wrapping_add(1);
        let mut setup = self
            .setup
            .clone()
            .unwrap_or_else(|| Setup::new(Workspace::default()));
        edit(&mut setup);
        let own = self.last_written.clone();
        cx.spawn(async move |this, cx| {
            let result = cx
                .background_spawn(
                    async move { save_on_disk(&file, &expected, own.as_ref(), setup) },
                )
                .await;
            this.update(cx, |this, cx| this.finish_save(result, cx))
                .unwrap_or_else(|_| Err(SaveError::Failed("The setup owner closed".into())))
        })
    }

    fn finish_save(
        &mut self,
        result: Result<Write, Refused>,
        cx: &mut Context<Self>,
    ) -> Result<(), SaveError> {
        self.writing = false;
        match result {
            Ok(write) => {
                self.record_write(write.setup, write.seen, write.stat);
                self.set_receipt("Saved", write.backup);
                cx.emit(Event::Changed {
                    setup_changed: true,
                });
                Ok(())
            }
            Err(refused) => {
                if let Some(disk) = refused.disk {
                    let (stat, outcome) = *disk;
                    self.apply(stat, outcome, false, cx);
                }
                Err(refused.error)
            }
        }
    }

    /// An explicit Reload bypasses the polling interval, off the UI thread.
    pub(crate) fn reload(&mut self, cx: &mut Context<Self>) -> Task<()> {
        let Some(file) = self.file.clone() else {
            return Task::ready(());
        };
        // An older poll must not publish after this explicit read.
        self.write_generation = self.write_generation.wrapping_add(1);
        let generation = self.write_generation;
        cx.spawn(async move |this, cx| {
            let (stat, disk) = cx.background_spawn(async move { read_now(&file) }).await;
            _ = this.update(cx, |this, cx| {
                if this.write_generation == generation {
                    this.apply(stat, disk, false, cx);
                }
            });
        })
    }

    fn apply(&mut self, stat: FileStat, outcome: Outcome, initial: bool, cx: &mut Context<Self>) {
        self.stat = stat;
        let old_problem = self.problem.clone();
        let mut setup_changed = false;
        self.problem = match outcome {
            Outcome::Parsed(Ok(setup), bytes) => {
                let setup = *setup;
                // Recognize canonical output from a prior save or migration.
                if initial && setup::to_text(&setup).as_bytes() == bytes {
                    self.last_written = Some(Seen::Bytes(bytes.clone()));
                }
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
        if self.problem.is_some() || matches!(self.stat, FileStat::Missing) {
            self.receipt = None;
            self.receipt_label.clear();
        }
        if initial {
            cx.emit(Event::Ready);
        } else if setup_changed || old_problem != self.problem {
            if setup_changed {
                self.set_receipt("Reloaded", false);
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

#[derive(Debug)]
pub(crate) enum SaveError {
    Changed,
    Failed(String),
}

impl std::fmt::Display for SaveError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Changed => f.write_str("freshkube.yaml changed on disk since it was read. Nothing was written; reload to see the file as it is."),
            Self::Failed(why) => f.write_str(why),
        }
    }
}

struct Write {
    setup: Setup,
    seen: Seen,
    stat: FileStat,
    backup: bool,
}

struct Refused {
    error: SaveError,
    disk: Option<Box<(FileStat, Outcome)>>,
}

fn save_on_disk(
    file: &std::path::Path,
    expected: &Seen,
    own: Option<&Seen>,
    setup: Setup,
) -> Result<Write, Refused> {
    let (stat, disk) = read_now(file);
    let current = match &disk {
        Outcome::Parsed(Ok(_), bytes) => Seen::Bytes(bytes.clone()),
        Outcome::Missing => Seen::Missing,
        _ => {
            return Err(Refused {
                error: SaveError::Failed(
                    "freshkube.yaml isn't usable. Nothing was written.".into(),
                ),
                disk: Some(Box::new((stat, disk))),
            });
        }
    };
    // A deleted file may be recreated by the next Settings save.
    if current != *expected && current != Seen::Missing {
        return Err(Refused {
            error: SaveError::Changed,
            disk: Some(Box::new((stat, disk))),
        });
    }
    let backup = matches!(current, Seen::Bytes(_)) && own != Some(&current);
    match setup::commit_with_backup(file, &current, &setup, backup) {
        Ok(seen) => {
            let mut stat = FileStat::read(file);
            // Never acknowledge an editor's later stat as our own write.
            if let Seen::Bytes(bytes) = &seen
                && read_bounded_regular_file(file, MAX_BYTES).ok().as_ref() != Some(bytes)
            {
                stat = FileStat::Unreadable(ErrorKind::WouldBlock);
            }
            Ok(Write {
                setup,
                seen,
                stat,
                backup,
            })
        }
        Err(error) => Err(match error {
            setup::SaveError::Changed => Refused {
                error: SaveError::Changed,
                disk: Some(Box::new(read_now(file))),
            },
            setup::SaveError::Invalid(why) => Refused {
                error: SaveError::Failed(why.to_string()),
                disk: None,
            },
            setup::SaveError::Write(_) => Refused {
                error: SaveError::Failed(
                    "Couldn't save freshkube.yaml or its backup. The setup file is as it was."
                        .into(),
                ),
                disk: None,
            },
        }),
    }
}

fn read_now(file: &std::path::Path) -> (FileStat, Outcome) {
    let stat = FileStat::read(file);
    let outcome = match read_bounded_regular_file(file, MAX_BYTES) {
        Ok(bytes) => Outcome::Parsed(setup::parse(&bytes).map(Box::new), bytes),
        Err(BoundedReadError::NotFound) => Outcome::Missing,
        Err(BoundedReadError::TooLarge) => Outcome::Problem("it is larger than 256 KiB".into()),
        Err(_) => Outcome::Problem("it could not be read as a regular file".into()),
    };
    (stat, outcome)
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
    async fn the_first_workspace_save_moves_unknown_keys_beside_the_workspace(
        cx: &mut TestAppContext,
    ) {
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
}
