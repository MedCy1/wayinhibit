use std::process::{Child, Command, ExitStatus, Stdio};
use std::thread;
use std::time::{Duration, Instant};

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

        send_signal(self.child.id(), libc::SIGTERM)?;

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

        send_signal(self.child.id(), libc::SIGKILL)?;

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

fn send_signal(pid: u32, signal: libc::c_int) -> Result<(), String> {
    let pid = i32::try_from(pid).map_err(|_| "child process id is out of range".to_string())?;

    let result = unsafe { libc::kill(pid, signal) };
    if result == -1 {
        return Err(format!(
            "failed to send signal {signal} to child process {pid}: {}",
            std::io::Error::last_os_error()
        ));
    }

    Ok(())
}
