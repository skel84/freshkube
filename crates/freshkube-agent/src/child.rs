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
