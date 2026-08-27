use std::process::{Child, Command, ExitStatus, Stdio};
use std::thread;
use std::time::{Duration, Instant};

use std::os::unix::process::CommandExt;
#[cfg(unix)]
use std::os::unix::process::ExitStatusExt;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommandSpec {
    pub program: String,
    pub args: Vec<String>,
}

impl CommandSpec {
    pub fn spawn(&self) -> Result<ManagedChild, String> {
        let child = Command::new(&self.program)
            .args(&self.args)
            .stdin(Stdio::inherit())
            .stdout(Stdio::inherit())
            .stderr(Stdio::inherit())
            // Run in its own process group so terminate_and_wait can signal the whole
            // group (e.g. a shell script's own background children), not just this PID.
            .process_group(0)
            .spawn()
            .map_err(|err| format!("failed to start '{}': {err}", self.program))?;

        Ok(ManagedChild { child })
    }
}

pub struct ManagedChild {
    child: Child,
}

impl ManagedChild {
    pub fn try_wait(&mut self) -> Result<Option<u8>, String> {
        self.child
            .try_wait()
            .map_err(|err| format!("failed to wait for child process: {err}"))
            .map(|status| status.map(exit_code_from_status))
    }

    pub fn terminate_and_wait(&mut self, grace_period: Duration) -> Result<u8, String> {
        if let Some(code) = self.try_wait()? {
            return Ok(code);
        }

        send_signal_to_group(self.child.id(), libc::SIGTERM)?;

        let deadline = Instant::now() + grace_period;
        loop {
            if let Some(code) = self.try_wait()? {
                return Ok(code);
            }

            if Instant::now() >= deadline {
                break;
            }

            thread::sleep(Duration::from_millis(50));
        }

        send_signal_to_group(self.child.id(), libc::SIGKILL)?;

        self.child
            .wait()
            .map(exit_code_from_status)
            .map_err(|err| format!("failed to wait for child process after SIGKILL: {err}"))
    }
}

fn exit_code_from_status(status: ExitStatus) -> u8 {
    if let Some(code) = status.code() {
        return u8::try_from(code).unwrap_or(1);
    }

    #[cfg(unix)]
    if let Some(signal) = status.signal() {
        return 128u8.saturating_add(u8::try_from(signal).unwrap_or(0));
    }

    1
}

fn kill(target: i32, signal: libc::c_int) -> Result<(), String> {
    if unsafe { libc::kill(target, signal) } == -1 {
        return Err(format!(
            "failed to send signal {signal} to {target}: {}",
            std::io::Error::last_os_error()
        ));
    }
    Ok(())
}

pub(crate) fn send_signal(pid: u32, signal: libc::c_int) -> Result<(), String> {
    let pid = i32::try_from(pid).map_err(|_| "process id is out of range".to_string())?;
    kill(pid, signal)
}

/// Sends `signal` to every process in the group led by `pgid` (a `process_group(0)`
/// child's PID doubles as its own pgid). A negative PID targets the whole group.
fn send_signal_to_group(pgid: u32, signal: libc::c_int) -> Result<(), String> {
    let pgid = i32::try_from(pgid).map_err(|_| "process id is out of range".to_string())?;
    kill(-pgid, signal)
}
