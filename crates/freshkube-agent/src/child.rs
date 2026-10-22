//! The agent's process and what ends it.
//!
//! The child runs in its own process group (a job object on Windows), so a
//! grandchild it starts goes with it. Ending a session sends SIGTERM to the
//! group and, if anything is left after [`KILL_AFTER`], SIGKILL.

use std::any::Any;
use std::io;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use tokio::io::{AsyncRead, AsyncWrite};
use tokio::task::AbortHandle;

/// How long a terminated group has before it is killed.
#[cfg(unix)]
pub const KILL_AFTER: Duration = Duration::from_secs(2);

/// Both ends of an agent's stdio, the workspace it works in, and what ends it.
pub struct Link {
    pub(crate) reader: Box<dyn AsyncRead + Send + Unpin>,
    pub(crate) writer: Box<dyn AsyncWrite + Send + Unpin>,
    pub(crate) workspace: PathBuf,
    pub(crate) guard: Guard,
    /// What the agent is called in messages, such as "opencode".
    pub(crate) name: &'static str,
}

impl Link {
    /// A link over pipes to an agent that runs in this process; `task` is
    /// aborted when the session ends.
    pub fn in_process(
        name: &'static str,
        reader: impl AsyncRead + Send + Unpin + 'static,
        writer: impl AsyncWrite + Send + Unpin + 'static,
        workspace: PathBuf,
        task: AbortHandle,
    ) -> Self {
        let guard = Guard::default();
        guard.0.abort.lock().unwrap().replace(task);
        Self {
            reader: Box::new(reader),
            writer: Box::new(writer),
            workspace,
            guard,
            name,
        }
    }
}

/// Ends the agent: the process group, the in-process task, and then removes
/// what the session made on disk. Ending twice does nothing.
#[derive(Clone, Default)]
pub struct Guard(Arc<Inner>);

#[derive(Default)]
struct Inner {
    ended: AtomicBool,
    group: Mutex<Option<ProcessGroup>>,
    abort: Mutex<Option<AbortHandle>>,
    /// Kept alive until the process is gone (the temporary directories).
    keep: Mutex<Vec<Box<dyn Any + Send>>>,
}

impl Guard {
    pub(crate) fn with_group(group: ProcessGroup, keep: Box<dyn Any + Send>) -> Self {
        let guard = Self::default();
        guard.0.group.lock().unwrap().replace(group);
        guard.0.keep.lock().unwrap().push(keep);
        guard
    }

    pub(crate) fn end(&self) {
        self.0.end();
    }

    /// Whether the session ended the agent, as opposed to the agent ending
    /// on its own.
    pub(crate) fn is_ended(&self) -> bool {
        self.0.ended.load(Ordering::SeqCst)
    }
}

impl Inner {
    fn end(&self) {
        self.ended.store(true, Ordering::SeqCst);
        if let Some(task) = self.abort.lock().unwrap().take() {
            task.abort();
        }
        let keep = std::mem::take(&mut *self.keep.lock().unwrap());
        match self.group.lock().unwrap().take() {
            Some(group) => group.end(keep),
            None => drop(keep),
        }
    }
}

impl Drop for Inner {
    fn drop(&mut self) {
        self.end();
    }
}

#[cfg(unix)]
pub(crate) use unix::ProcessGroup;
#[cfg(windows)]
pub(crate) use windows::ProcessGroup;

#[cfg(unix)]
mod unix {
    use super::*;

    const WATCHER: &str = r#"
parent=$1; group=$2
alive() { kill -s 0 -- "$1" 2>/dev/null; }
while alive "$parent" && alive "-$group"; do sleep 1; done
if alive "-$group"; then
  kill -s TERM -- "-$group" 2>/dev/null
  sleep 2
  kill -s KILL -- "-$group" 2>/dev/null
fi
"#;

    /// Starts the watcher for `group` over `parent`. Its failure to start
    /// leaves the session without that backstop, nothing worse.
    pub(crate) fn watch(parent: u32, group: i32) {
        use std::os::unix::process::CommandExt;
        let _ = std::process::Command::new("/bin/sh")
            .args(["-c", WATCHER, "freshkube-agent-watch"])
            .arg(parent.to_string())
            .arg(group.to_string())
            .env_clear()
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .process_group(0)
            .spawn()
            .map(|mut watcher| {
                // Reaped when it exits, so it never lingers as a zombie.
                std::thread::spawn(move || {
                    let _ = watcher.wait();
                });
            });
    }

    /// A process group whose leader is the child.
    pub(crate) struct ProcessGroup {
        pgid: i32,
    }

    impl ProcessGroup {
        /// The child must have been started with `process_group(0)`.
        pub(crate) fn of(child: &tokio::process::Child) -> io::Result<Self> {
            let pid = child
                .id()
                .ok_or_else(|| io::Error::other("the agent exited before it started"))?;
            Ok(Self { pgid: pid as i32 })
        }

        /// Ends the group if this process dies without ending it (a crash, a
        /// kill): a small shell, in a group of its own, that waits for the
        /// parent to be gone and then terminates the agent's group, and
        /// exits when either is gone. Best effort: the group ends within a
        /// few seconds of the parent's death.
        pub(crate) fn watch_parent(&self) {
            watch(std::process::id(), self.pgid);
        }

        fn signal(pgid: i32, signal: i32) {
            // SAFETY: kill(2) with a negative pid signals a process group.
            unsafe { libc::kill(-pgid, signal) };
        }

        fn gone(pgid: i32) -> bool {
            // SAFETY: signal 0 only checks that the group exists.
            let result = unsafe { libc::kill(-pgid, 0) };
            result != 0 && io::Error::last_os_error().raw_os_error() == Some(libc::ESRCH)
        }

        /// SIGTERM now; SIGKILL after [`KILL_AFTER`] if the group is still
        /// there. `keep` is dropped once it is gone.
        pub(crate) fn end(self, keep: Vec<Box<dyn Any + Send>>) {
            let pgid = self.pgid;
            Self::signal(pgid, libc::SIGTERM);
            std::thread::spawn(move || {
                let deadline = std::time::Instant::now() + KILL_AFTER;
                while !Self::gone(pgid) && std::time::Instant::now() < deadline {
                    std::thread::sleep(Duration::from_millis(10));
                }
                if !Self::gone(pgid) {
                    Self::signal(pgid, libc::SIGKILL);
                    let kill_deadline = std::time::Instant::now() + KILL_AFTER;
                    while !Self::gone(pgid) && std::time::Instant::now() < kill_deadline {
                        std::thread::sleep(Duration::from_millis(10));
                    }
                }
                drop(keep);
            });
        }
    }
}

#[cfg(windows)]
mod windows {
    use super::*;
    use windows_sys::Win32::Foundation::{CloseHandle, HANDLE};
    use windows_sys::Win32::System::JobObjects::{
        AssignProcessToJobObject, CreateJobObjectW, JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
        JOBOBJECT_EXTENDED_LIMIT_INFORMATION, JobObjectExtendedLimitInformation,
        SetInformationJobObject,
    };

    #[link(name = "ntdll")]
    unsafe extern "system" {
        /// Resumes every thread of a process created suspended.
        fn NtResumeProcess(process: HANDLE) -> i32;
    }

    /// A job object that kills every process in it when it closes.
    pub(crate) struct ProcessGroup {
        job: HANDLE,
    }

    // SAFETY: a job handle may be used and closed from any thread.
    unsafe impl Send for ProcessGroup {}

    impl ProcessGroup {
        pub(crate) fn of(child: &tokio::process::Child) -> io::Result<Self> {
            let process = child
                .raw_handle()
                .ok_or_else(|| io::Error::other("the agent exited before it started"))?;
            // SAFETY: plain Win32 calls on handles this function owns.
            unsafe {
                let job = CreateJobObjectW(std::ptr::null(), std::ptr::null());
                if job.is_null() {
                    return Err(io::Error::last_os_error());
                }
                let mut info: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = std::mem::zeroed();
                info.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
                let set = SetInformationJobObject(
                    job,
                    JobObjectExtendedLimitInformation,
                    std::ptr::addr_of!(info).cast(),
                    std::mem::size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32,
                );
                if set == 0 || AssignProcessToJobObject(job, process as HANDLE) == 0 {
                    let error = io::Error::last_os_error();
                    CloseHandle(job);
                    return Err(error);
                }
                Ok(Self { job })
            }
        }

        /// Lets the suspended child run, now that it is in the job: a process
        /// it starts is born in the job too and cannot escape it. If it can't
        /// be resumed the job is closed, which ends it.
        pub(crate) fn resume(self, child: &tokio::process::Child) -> io::Result<Self> {
            let Some(process) = child.raw_handle() else {
                self.end(Vec::new());
                return Err(io::Error::other("the agent exited before it started"));
            };
            // SAFETY: the handle is the child's, valid while `child` is.
            let status = unsafe { NtResumeProcess(process as HANDLE) };
            if status < 0 {
                self.end(Vec::new());
                return Err(io::Error::other(format!(
                    "the agent could not be resumed (status {status:#x})"
                )));
            }
            Ok(self)
        }

        /// Closing the job ends every process in it.
        pub(crate) fn end(self, keep: Vec<Box<dyn Any + Send>>) {
            // SAFETY: the handle is ours and is closed once.
            unsafe { CloseHandle(self.job) };
            std::thread::spawn(move || {
                std::thread::sleep(Duration::from_millis(200));
                drop(keep);
            });
        }
    }
}

#[cfg(all(test, target_os = "linux"))]
mod tests {
    use std::process::{Command, Stdio};
    use std::time::{Duration, Instant};

    use super::unix::watch;

    fn alive(pid: u32) -> bool {
        let stat = std::fs::read_to_string(format!("/proc/{pid}/stat")).unwrap_or_default();
        let state = stat
            .rsplit(')')
            .next()
            .and_then(|r| r.trim().chars().next());
        !stat.is_empty() && state != Some('Z')
    }

    /// Every process in the group, from /proc.
    fn members(group: i32) -> Vec<u32> {
        std::fs::read_dir("/proc")
            .unwrap()
            .flatten()
            .filter_map(|entry| {
                let pid: u32 = entry.file_name().to_string_lossy().parse().ok()?;
                let stat = std::fs::read_to_string(entry.path().join("stat")).ok()?;
                // pid (name) state ppid pgrp ...
                let rest = stat.rsplit(')').next()?;
                let pgrp: i32 = rest.split_whitespace().nth(2)?.parse().ok()?;
                (pgrp == group).then_some(pid)
            })
            .collect()
    }

    #[test]
    fn the_group_ends_when_the_parent_dies_without_ending_it() {
        use std::os::unix::process::CommandExt;
        // A stand-in for the app, and a group with a leader and a child of its own.
        let mut parent = Command::new("sleep").arg("600").spawn().unwrap();
        let mut leader = Command::new("sh")
            .args(["-c", "sleep 600 & wait"])
            .stdin(Stdio::null())
            .process_group(0)
            .spawn()
            .unwrap();
        let group = leader.id() as i32;
        std::thread::sleep(Duration::from_millis(300));
        let members = members(group);
        assert!(
            members.len() >= 2,
            "the group has a leader and a child: {members:?}"
        );
        watch(parent.id(), group);

        // While the parent lives, the group lives.
        std::thread::sleep(Duration::from_millis(1500));
        assert!(alive(leader.id()));

        parent.kill().unwrap();
        parent.wait().unwrap();
        let deadline = Instant::now() + Duration::from_secs(8);
        while alive(leader.id()) && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(50));
        }
        assert!(
            !alive(leader.id()),
            "the group's leader outlived its parent"
        );
        for pid in members {
            assert!(!alive(pid), "a member of the group outlived its parent");
        }
        let _ = leader.wait();
    }
}
