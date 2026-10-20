//! The shell's last good `freshkube.yaml`, and the file's current problem.
//! File work runs on GPUI's background executor; timers use its clock.
use std::{io::ErrorKind, path::PathBuf, time::Duration};

use freshkube_core::{
    BoundedReadError, read_bounded_regular_file,
    setup::{self, Setup},
    workspace::{MAX_BYTES, Seen, Workspace},
};
use gpui_kit::{AppContext, Context, EventEmitter, Task};

pub(crate) const SKIPPED_SAVE: &str =
    "Fix the skipped items in freshkube.yaml and Reload before saving from Settings.";

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
    pub(crate) warnings: Vec<setup::Warning>,
    pub(crate) all_clusters_skipped: bool,
    setup_revision: u64,
    pub(crate) receipt: Option<String>,
    receipt_label: String,
    file: Option<PathBuf>,
    last_written: Option<Seen>,
    pub(crate) coroot_reconnect: bool,
    stat: FileStat,
    write_generation: u64,
    /// Setup or problem changes and successful explicit reloads, including
    /// identical bytes: referenced key files can change independently.
    read_revision: u64,
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
            warnings: Vec::new(),
            all_clusters_skipped: false,
            setup_revision: 0,
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
            read_revision: 0,
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
                                    Outcome::Parsed(setup::read(&bytes).map(Box::new), bytes)
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
                                    this.apply(stat.clone(), outcome, initial, false, cx);
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
        if self.setup.as_ref() != Some(&setup) || self.all_clusters_skipped {
            self.setup_revision += 1;
        }
        if self.setup.as_ref() != Some(&setup)
            || self.problem.is_some()
            || !self.warnings.is_empty()
        {
            self.read_revision += 1;
        }
        self.setup = Some(setup);
        self.problem = None;
        self.warnings.clear();
        self.all_clusters_skipped = false;
        self.last_written = Some(seen.clone());
        self.seen = seen;
        self.stat = stat;
        self.write_generation = self.write_generation.wrapping_add(1);
    }

    pub(crate) fn is_missing(&self) -> bool {
        self.stat == FileStat::Missing
    }

    pub(crate) fn saves_refused(&self) -> bool {
        !self.warnings.is_empty() || (self.problem.is_some() && !self.is_missing())
    }

    pub(crate) fn notice(&self) -> Option<String> {
        self.problem
            .clone()
            .or_else(|| setup::warning_words(&self.warnings))
    }

    pub(crate) fn setup_revision(&self) -> u64 {
        self.setup_revision
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
        if !self.warnings.is_empty() {
            return Task::ready(Err(SaveError::Failed(SKIPPED_SAVE.into())));
        }
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
                    self.apply(stat, outcome, false, false, cx);
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
                    this.apply(stat, disk, false, true, cx);
                }
            });
        })
    }

    /// The delivery hand-off consumes each accepted read only once.
    #[cfg(test)]
    pub(crate) fn read_revision(&self) -> u64 {
        self.read_revision
    }

    fn apply(
        &mut self,
        stat: FileStat,
        outcome: Outcome,
        initial: bool,
        explicit: bool,
        cx: &mut Context<Self>,
    ) {
        let reloaded = explicit && matches!(&outcome, Outcome::Parsed(Ok(_), _));
        self.stat = stat;
        let old_notice = self.notice();
        let mut setup_changed = false;
        self.problem = match outcome {
            Outcome::Parsed(Ok(setup), bytes) => {
                let read = *setup;
                let setup = read.setup;
                setup_changed = self.setup.as_ref() != Some(&setup)
                    || self.all_clusters_skipped != read.all_clusters_skipped;
                self.warnings = read.warnings;
                self.all_clusters_skipped = read.all_clusters_skipped;
                // Recognize canonical output from a prior save or migration.
                if initial && setup::to_text(&setup).as_bytes() == bytes {
                    self.last_written = Some(Seen::Bytes(bytes.clone()));
                }
                if self.seen != Seen::Bytes(bytes.clone()) {
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
        if self.problem.is_some() {
            self.warnings.clear();
        }
        if self.problem.is_some() || matches!(self.stat, FileStat::Missing) {
            self.receipt = None;
            self.receipt_label.clear();
        }
        if setup_changed || reloaded {
            self.setup_revision += 1;
        }
        if setup_changed || reloaded || old_notice != self.notice() {
            self.read_revision += 1;
        }
        if initial {
            cx.emit(Event::Ready);
        } else if setup_changed || reloaded || old_notice != self.notice() {
            if setup_changed || reloaded {
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
    if matches!(&disk, Outcome::Parsed(Ok(read), _) if !read.warnings.is_empty()) {
        return Err(Refused {
            error: SaveError::Failed(SKIPPED_SAVE.into()),
            disk: Some(Box::new((stat, disk))),
        });
    }
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
        Ok(bytes) => Outcome::Parsed(setup::read(&bytes).map(Box::new), bytes),
        Err(BoundedReadError::NotFound) => Outcome::Missing,
        Err(BoundedReadError::TooLarge) => Outcome::Problem("it is larger than 256 KiB".into()),
        Err(_) => Outcome::Problem("it could not be read as a regular file".into()),
    };
    (stat, outcome)
}

enum Outcome {
    Parsed(Result<Box<setup::Recovered>, setup::Invalid>, Vec<u8>),
    Missing,
    Problem(String),
}

#[cfg(test)]
mod tests;
