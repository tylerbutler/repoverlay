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

/// Decide whether terminal foreground ownership should be handed to the child.
#[cfg(unix)]
#[allow(dead_code)]
const fn should_hand_off_terminal(stdin_is_tty: bool) -> bool {
    stdin_is_tty
}

#[cfg(unix)]
#[allow(dead_code)]
pub(crate) struct TerminalForeground {
    fd: BorrowedFd<'static>,
    previous_pgid: Pid,
}

#[cfg(unix)]
#[allow(dead_code)]
impl TerminalForeground {
    pub(crate) fn acquire(child_pgid: u32) -> Option<Self> {
        // SAFETY: STDIN_FILENO is a process-global file descriptor. The guard does
        // not close it and only borrows it while the process is alive.
        #[allow(unsafe_code)]
        let fd = unsafe { BorrowedFd::borrow_raw(nix::libc::STDIN_FILENO) };
        if !should_hand_off_terminal(isatty(fd).unwrap_or(false)) {
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
#[allow(dead_code)]
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

// Task 6 wires this into Copilot; allow dead_code until then.
#[allow(dead_code)]
pub(crate) struct HarnessProcess {
    child: Box<dyn ChildWrapper>,
}

#[allow(dead_code)]
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
        self.id()
    }

    pub(crate) fn try_wait(&mut self) -> Result<Option<ExitStatus>> {
        Ok(self.child.try_wait()?)
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
    fn terminal_handoff_disabled_without_tty() {
        assert!(!super::should_hand_off_terminal(false));
    }

    #[cfg(unix)]
    #[test]
    fn terminal_handoff_enabled_with_tty() {
        assert!(super::should_hand_off_terminal(true));
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
