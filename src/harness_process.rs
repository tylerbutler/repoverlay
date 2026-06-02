//! Managed harness process execution.

use anyhow::Result;
use process_wrap::std::{ChildWrapper, CommandWrap};
use std::process::{Command, ExitStatus};

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
