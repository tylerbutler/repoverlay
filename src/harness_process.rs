//! Managed harness process execution.

use anyhow::Result;
use process_wrap::std::{ChildWrapper, CommandWrap};
use std::process::{Command, ExitStatus};

#[cfg(unix)]
use std::os::fd::BorrowedFd;

#[cfg(unix)]
use nix::sys::signal::{SaFlags, SigAction, SigHandler, SigSet, Signal, sigaction};
#[cfg(unix)]
use nix::unistd::{Pid, isatty, tcgetpgrp, tcsetpgrp};

#[cfg(unix)]
pub(crate) struct TerminalForeground {
    fd: BorrowedFd<'static>,
    previous_pgid: Pid,
}

#[cfg(unix)]
impl TerminalForeground {
    pub(crate) fn acquire(child_pgid: u32) -> Option<Self> {
        // SAFETY: STDIN_FILENO is a process-global file descriptor. The guard does
        // not close it and only borrows it while the process is alive.
        #[allow(unsafe_code)]
        let fd = unsafe { BorrowedFd::borrow_raw(nix::libc::STDIN_FILENO) };
        // Only take terminal foreground ownership when stdin is a TTY.
        if !isatty(fd).unwrap_or(false) {
            return None;
        }

        let previous_pgid = tcgetpgrp(fd).ok()?;
        let child_pgid = Pid::from_raw(i32::try_from(child_pgid).ok()?);
        with_ignored_sigttou(|| tcsetpgrp(fd, child_pgid)).ok()?;

        Some(Self { fd, previous_pgid })
    }
}

#[cfg(unix)]
impl Drop for TerminalForeground {
    fn drop(&mut self) {
        let _ = with_ignored_sigttou(|| tcsetpgrp(self.fd, self.previous_pgid));
    }
}

#[cfg(unix)]
fn with_ignored_sigttou<T>(operation: impl FnOnce() -> nix::Result<T>) -> nix::Result<T> {
    let ignore = SigAction::new(SigHandler::SigIgn, SaFlags::empty(), SigSet::empty());
    // SAFETY: Temporarily changing the SIGTTOU disposition around tcsetpgrp
    // matches POSIX job-control requirements and restores the previous action
    // before returning.
    #[allow(unsafe_code)]
    let previous = unsafe { sigaction(Signal::SIGTTOU, &ignore)? };
    let result = operation();
    // SAFETY: Restores the exact signal action returned above.
    #[allow(unsafe_code)]
    unsafe {
        sigaction(Signal::SIGTTOU, &previous)?
    };
    result
}

pub(crate) struct HarnessProcess {
    child: Box<dyn ChildWrapper>,
}

impl HarnessProcess {
    pub(crate) fn spawn(command: Command) -> Result<Self> {
        let mut command = CommandWrap::from(command);

        #[cfg(unix)]
        {
            command.wrap(process_wrap::std::ProcessGroup::leader());
        }

        #[cfg(windows)]
        {
            command.wrap(process_wrap::std::JobObject);
        }

        let child = command.spawn()?;
        Ok(Self { child })
    }

    pub(crate) fn id(&self) -> u32 {
        self.child.id()
    }

    pub(crate) fn process_group_id(&self) -> u32 {
        // On Unix, `spawn` always wraps the child with `ProcessGroup::leader()`, which
        // makes the child its own process-group leader, so its PGID equals its PID.
        // Kept as a distinct method (rather than inlining `id()` at call sites) so
        // terminal/signal code reads in PGID terms and stays correct if that
        // invariant ever changes.
        self.id()
    }

    pub(crate) fn try_wait(&mut self) -> Result<Option<ExitStatus>> {
        // SAFETY: We call only `try_wait` on the raw child, which polls the OS for
        // the direct harness process exit status without modifying wrapper state.
        // This avoids process-wrap's `ProcessGroup::try_wait`, which waits for the
        // entire process group and can block or lose the harness exit status when a
        // same-group descendant is still alive.
        #[allow(unsafe_code)]
        let status = unsafe { self.child.inner_child_mut() }.try_wait()?;
        Ok(status)
    }

    pub(crate) fn terminate(&mut self) {
        #[cfg(unix)]
        {
            if self
                .child
                .signal(nix::sys::signal::Signal::SIGTERM as i32)
                .is_ok()
            {
                return;
            }
        }

        let _ = self.child.start_kill();
    }
}

#[cfg(test)]
mod tests {
    use super::HarnessProcess;
    use std::process::Command;

    #[test]
    fn harness_process_reports_child_id() {
        let mut command = Command::new("sh");
        command.args(["-c", "exit 0"]);

        let process = HarnessProcess::spawn(command).expect("spawn process");

        assert!(process.id() > 0);
        assert_eq!(process.process_group_id(), process.id());
    }

    #[cfg(unix)]
    #[test]
    fn harness_process_reports_parent_exit_with_lingering_group_child() {
        let mut command = Command::new("sh");
        command.args(["-c", "sleep 5 & exit 7"]);

        let mut process = HarnessProcess::spawn(command).expect("spawn process");

        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
        let status = loop {
            if let Some(status) = process.try_wait().expect("wait process") {
                break status;
            }
            assert!(
                std::time::Instant::now() < deadline,
                "parent exit was blocked by lingering child"
            );
            std::thread::sleep(std::time::Duration::from_millis(10));
        };

        process.terminate();

        assert_eq!(status.code(), Some(7));
    }

    #[test]
    fn harness_process_try_wait_returns_exit_status() {
        let mut command = Command::new("sh");
        command.args(["-c", "exit 7"]);

        let mut process = HarnessProcess::spawn(command).expect("spawn process");

        let status = loop {
            if let Some(status) = process.try_wait().expect("wait process") {
                break status;
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        };

        assert_eq!(status.code(), Some(7));
    }
}
