//! Download: the toolbar's menu that saves the visible or every retained
//! line to a file the user names. The text is what Copy would copy of the
//! same lines, taken when the menu item is clicked; the file is written
//! off the UI thread, to a temporary file renamed into place, so a failed
//! write never leaves a partial file under the chosen name.

use std::{
    fs,
    io::{self, Write},
    path::{Path, PathBuf},
};

use gpui_kit::assets::IconName;
use gpui_kit::{
    Context, IntoElement, ParentElement,
    component::{
        Sizable,
        button::Button,
        menu::{DropdownMenu, PopupMenuItem},
    },
    div,
};

use super::review::{CopyAs, LogReview, copied};
use super::{Feedback, LogSource, LogView};

/// Which lines a download takes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DownloadLines {
    /// The lines the filters show, as the list shows them.
    Visible,
    /// Every retained line, whatever the filters show.
    Retained,
}

impl<S: LogSource> LogView<S> {
    pub(super) fn render_download(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let view = cx.entity().downgrade();
        Button::new("logs-download")
            .outline()
            .small()
            .icon(IconName::Save)
            .dropdown_caret(true)
            // Kit draws a button with an icon alone as a square that clips
            // the chevron: an empty child keeps its padding.
            .child(div())
            .accessibility_label("Download")
            .tooltip("Save lines to a file")
            .dropdown_menu(move |mut menu, _, cx| {
                // Counted when the menu opens, not on every frame.
                let Some((visible, retained)) =
                    view.upgrade().map(|view| view.read(cx).download_counts())
                else {
                    return menu;
                };
                for (lines, label, count) in [
                    (DownloadLines::Visible, "Visible lines", visible),
                    (DownloadLines::Retained, "All retained lines", retained),
                ] {
                    let view = view.clone();
                    menu = menu.item(
                        PopupMenuItem::new(format!("{label} ({count})"))
                            .disabled(count == 0)
                            .on_click(move |_, _, cx| {
                                _ = view.update(cx, |view, cx| view.download(lines, cx));
                            }),
                    );
                }
                menu
            })
    }

    /// How many lines each menu item saves: the visible ones and every
    /// retained one, markers aside.
    fn download_counts(&self) -> (usize, usize) {
        let entries = self.review.logs.buffer().entries();
        let visible = self
            .review
            .visible
            .iter()
            .filter(|&&ix| !entries[ix].is_marker())
            .count();
        let retained = entries.iter().filter(|entry| !entry.is_marker()).count();
        (visible, retained)
    }

    /// The file's text: what Copy copies of the same lines, one line each.
    pub(super) fn download_text(&self, lines: DownloadLines) -> (String, usize) {
        download_text(&self.review, lines, self.copy_as())
    }

    /// Asks where to save the lines, then writes them there. While a save
    /// dialog is open, another download does nothing.
    pub(super) fn download(&mut self, lines: DownloadLines, cx: &mut Context<Self>) {
        if self.choosing_file {
            return;
        }
        let (text, count) = self.download_text(lines);
        if count == 0 {
            return;
        }
        let name = file_name(
            &S::download_name(self, lines),
            &chrono::Local::now().format("%Y%m%d-%H%M%S").to_string(),
        );
        let chosen = cx.prompt_for_new_path(&default_folder(), Some(&name));
        self.choosing_file = true;
        let executor = cx.background_executor().clone();
        self.download = Some(cx.spawn(async move |this, cx| {
            let chosen = chosen.await;
            _ = this.update(cx, |view, _| view.choosing_file = false);
            let path = match chosen {
                Ok(Ok(Some(path))) => path,
                // Cancelled, or the view went first.
                Ok(Ok(None)) | Err(_) => return,
                Ok(Err(error)) => {
                    _ = this.update(cx, |view, cx| {
                        view.feedback = Some(Feedback::failed(format!(
                            "Couldn't open the save dialog: {error}"
                        )));
                        cx.notify();
                    });
                    return;
                }
            };
            let written = {
                let path = path.clone();
                executor
                    .spawn(async move { write_whole(&path, text.as_bytes()) })
                    .await
            };
            _ = this.update(cx, |view, cx| {
                // The note names the file; its tooltip, the folder too.
                let shown = shown_path(&path);
                let name = path
                    .file_name()
                    .map(|name| name.to_string_lossy().into_owned())
                    .unwrap_or_else(|| shown.clone());
                let lines = if count == 1 { "line" } else { "lines" };
                view.feedback = Some(match written {
                    Ok(()) => Feedback::from(format!("Saved {count} {lines} to {name}"))
                        .with_whole(format!("Saved {count} {lines} to {shown}")),
                    Err(error) => Feedback::failed(format!("Couldn't save {name}: {error}"))
                        .with_whole(format!("Couldn't save {shown}: {error}")),
                });
                cx.notify();
            });
        }));
    }
}

/// The lines' text as Copy writes it, one line each, and how many. Built
/// on the UI thread when the menu item is clicked: at most about 8 MiB,
/// which takes a few milliseconds (docs/PERFORMANCE.md).
pub(super) fn download_text(
    review: &LogReview,
    lines: DownloadLines,
    as_: CopyAs,
) -> (String, usize) {
    let _span = freshkube_probe::perf::span(match lines {
        DownloadLines::Visible => "logs.download_text.visible",
        DownloadLines::Retained => "logs.download_text.retained",
    });
    let entries = review.logs.buffer().entries();
    let chosen: Box<dyn Iterator<Item = _>> = match lines {
        DownloadLines::Visible => Box::new(review.visible.iter().map(|&ix| &entries[ix])),
        DownloadLines::Retained => Box::new(entries.iter()),
    };
    let mut text = String::new();
    let mut count = 0;
    for entry in chosen.filter(|entry| !entry.is_marker()) {
        copied(entry, as_, &mut text);
        text.push('\n');
        count += 1;
    }
    (text, count)
}

/// `<base>-<time>.log`, with characters a file name can't hold replaced.
pub(super) fn file_name(base: &str, time: &str) -> String {
    let base: String = base
        .chars()
        .map(|c| match c {
            '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|' => '-',
            c if c.is_whitespace() || c.is_control() => '-',
            c => c,
        })
        .collect();
    let base = base.trim_matches(|c| c == '-' || c == '.');
    let base = if base.is_empty() { "logs" } else { base };
    format!("{base}-{time}.log")
}

/// The Downloads folder, else the home folder.
pub(super) fn default_folder() -> PathBuf {
    dirs_next::download_dir()
        .filter(|dir| dir.is_dir())
        .or_else(dirs_next::home_dir)
        .unwrap_or_else(|| PathBuf::from("."))
}

/// The path as the note shows it, with the home folder as `~`.
fn shown_path(path: &Path) -> String {
    if let Some(home) = dirs_next::home_dir()
        && let Ok(rest) = path.strip_prefix(&home)
    {
        return Path::new("~").join(rest).display().to_string();
    }
    path.display().to_string()
}

/// Writes `bytes` to a temporary file beside `path` and renames it into
/// place, so `path` holds either the whole text or what it held before.
pub(super) fn write_whole(path: &Path, bytes: &[u8]) -> io::Result<()> {
    write_whole_with(path, |file| file.write_all(bytes))
}

/// `write_whole`, with what goes into the temporary file given by `write`.
/// The temporary file, `.<name>.freshkube-<pid>.part`, is removed on every
/// way out but a successful rename: a failed write or rename, or a panic.
fn write_whole_with(
    path: &Path,
    write: impl FnOnce(&mut fs::File) -> io::Result<()>,
) -> io::Result<()> {
    let name = path
        .file_name()
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "no file name"))?;
    let folder = path
        .parent()
        .filter(|folder| !folder.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    let temporary = Temporary(Some(folder.join(format!(
        ".{}.freshkube-{}.part",
        name.to_string_lossy(),
        std::process::id()
    ))));
    let at = temporary.path();
    let mut file = fs::File::create(at)?;
    write(&mut file)?;
    file.sync_all()?;
    drop(file);
    fs::rename(at, path)?;
    temporary.renamed();
    Ok(())
}

/// A temporary file that is removed when dropped, unless renamed first.
struct Temporary(Option<PathBuf>);

impl Temporary {
    fn path(&self) -> &Path {
        self.0.as_deref().expect("a temporary path until renamed")
    }

    /// It is the saved file now: nothing to remove.
    fn renamed(mut self) {
        self.0 = None;
    }
}

impl Drop for Temporary {
    fn drop(&mut self) {
        if let Some(path) = self.0.take() {
            _ = fs::remove_file(path);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_file_name_drops_what_a_path_cant_hold() {
        assert_eq!(
            file_name("payments/api pod:1", "20261007-101500"),
            "payments-api-pod-1-20261007-101500.log"
        );
        assert_eq!(file_name("../", "t"), "logs-t.log");
    }

    #[test]
    fn a_failed_write_leaves_neither_the_file_nor_its_temporary() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("lines.log");
        write_whole(&path, b"one\ntwo\n").unwrap();
        assert_eq!(fs::read_to_string(&path).unwrap(), "one\ntwo\n");

        // The rename fails onto a folder: the folder keeps what it held,
        // and no temporary file stays behind.
        let folder = dir.path().join("taken");
        fs::create_dir(&folder).unwrap();
        fs::write(folder.join("kept"), "kept").unwrap();
        assert!(write_whole(&folder, b"three\n").is_err());
        assert!(folder.is_dir());
        let mut names: Vec<_> = fs::read_dir(dir.path())
            .unwrap()
            .map(|entry| entry.unwrap().file_name().into_string().unwrap())
            .collect();
        names.sort();
        assert_eq!(names, ["lines.log", "taken"]);
    }

    #[test]
    fn an_error_mid_write_leaves_no_temporary_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("lines.log");
        let failed = write_whole_with(&path, |file| {
            file.write_all(b"half of it")?;
            // Its name says whose it is, while it lasts.
            let names: Vec<_> = fs::read_dir(dir.path())
                .unwrap()
                .map(|entry| entry.unwrap().file_name().into_string().unwrap())
                .collect();
            assert_eq!(
                names,
                [format!(".lines.log.freshkube-{}.part", std::process::id())]
            );
            Err(io::Error::other("disk full"))
        });
        assert_eq!(failed.unwrap_err().to_string(), "disk full");
        assert_eq!(fs::read_dir(dir.path()).unwrap().count(), 0);
    }

    /// How long a full buffer's text takes to build, for each kind of
    /// download. A measurement, not a check: run it in release with
    /// `cargo test --release -p freshkube-logs download_text_timing --
    /// --ignored --nocapture`.
    #[test]
    #[ignore]
    fn download_text_timing() {
        use freshkube_core::logs::LogEvent;
        let services = ["apid", "kubelet", "etcd", "containerd", "machined"];
        // Lines of about 140 bytes, as a busy node writes them, then lines
        // long enough that the 5,000 the buffer keeps fill its 8 MiB.
        for padding in [0, 1580] {
            let mut review = LogReview::new("node");
            for batch in 0..10 {
                review.append((0..1000).map(|line| {
                    let ix = batch * 1000 + line;
                    LogEvent::new(
                        services[ix % services.len()],
                        format!(
                            "2026-10-07T10:{:02}:{:02}.{:06}Z level=info msg=\"rpc request\" \\
                             method=/machine.MachineService/ServiceList id={ix} {}",
                            (ix / 60) % 60,
                            ix % 60,
                            ix % 1_000_000,
                            "x".repeat(padding)
                        ),
                    )
                }));
            }
            let entries = review.logs.buffer().entries();
            let bytes: usize = entries.iter().map(|entry| entry.raw.len()).sum();
            println!(
                "{} lines, {:.2} MiB retained",
                entries.len(),
                bytes as f64 / (1024. * 1024.)
            );
            for lines in [DownloadLines::Visible, DownloadLines::Retained] {
                for (time, tagged) in [(false, false), (true, false), (false, true), (true, true)] {
                    let as_ = CopyAs { time, tagged };
                    let mut runs: Vec<f64> = (0..15)
                        .map(|_| {
                            let started = std::time::Instant::now();
                            let (text, _) = download_text(&review, lines, as_);
                            let taken = started.elapsed().as_secs_f64() * 1000.;
                            std::hint::black_box(text);
                            taken
                        })
                        .collect();
                    runs.sort_by(f64::total_cmp);
                    println!(
                        "  {lines:?} time={time} tagged={tagged}: median {:.2} ms, max {:.2} ms",
                        runs[runs.len() / 2],
                        runs[runs.len() - 1]
                    );
                }
            }
        }
    }
}
