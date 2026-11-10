//! Terminal behavior of a wrapped command, on a real pseudo-terminal.
//!
//! The terminal is what makes interactive programs (editors, ssh, prompts) work or stall, and
//! job control (Ctrl-C, Ctrl-Z) depends on which process group owns it. A pty with a session
//! leader on it reproduces what a user's terminal does.

mod common;

use std::ffi::OsStr;
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};
use std::os::unix::process::CommandExt;
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

use common::bin;

const PATIENCE: Duration = Duration::from_secs(10);

struct Pty {
    master: OwnedFd,
    slave: OwnedFd,
}

fn open_pty() -> Pty {
    let (mut master, mut slave) = (0, 0);
    let result = unsafe {
        libc::openpty(
            &mut master,
            &mut slave,
            std::ptr::null_mut(),
            std::ptr::null(),
            std::ptr::null(),
        )
    };
    assert_eq!(result, 0, "openpty failed");
    for fd in [master, slave] {
        unsafe { libc::fcntl(fd, libc::F_SETFD, libc::FD_CLOEXEC) };
    }
    unsafe {
        Pty {
            master: OwnedFd::from_raw_fd(master),
            slave: OwnedFd::from_raw_fd(slave),
        }
    }
}

impl Pty {
    fn write(&self, bytes: &[u8]) {
        let written =
            unsafe { libc::write(self.master.as_raw_fd(), bytes.as_ptr().cast(), bytes.len()) };
        assert_eq!(written, bytes.len() as isize);
    }

    /// Reads what the terminal printed until `needle` shows up. Returns everything read.
    fn read_until(&self, seen: &mut String, needle: &str) {
        let deadline = Instant::now() + PATIENCE;
        while !seen.contains(needle) {
            let left = deadline.saturating_duration_since(Instant::now());
            assert!(
                !left.is_zero(),
                "timed out waiting for {needle:?}; terminal output so far: {seen:?}"
            );
            let mut poll = libc::pollfd {
                fd: self.master.as_raw_fd(),
                events: libc::POLLIN,
                revents: 0,
            };
            let ready = unsafe { libc::poll(&mut poll, 1, left.as_millis().min(200) as i32) };
            if ready <= 0 {
                continue;
            }
            let mut buffer = [0u8; 4096];
            let count = unsafe {
                libc::read(
                    self.master.as_raw_fd(),
                    buffer.as_mut_ptr().cast(),
                    buffer.len(),
                )
            };
            if count > 0 {
                seen.push_str(&String::from_utf8_lossy(&buffer[..count as usize]));
            } else if count < 0 {
                // EIO once every holder of the slave side is gone.
                break;
            }
        }
    }
}

/// Starts `program` as the session leader of the pty, like a terminal emulator starts a shell.
fn spawn_on(pty: &Pty, program: impl AsRef<OsStr>, args: &[&str], envs: &[(&str, &str)]) -> Child {
    let slave = |pty: &Pty| Stdio::from(pty.slave.try_clone().unwrap());
    let mut command = Command::new(program);
    command
        .args(args)
        .envs(envs.iter().copied())
        .stdin(slave(pty))
        .stdout(slave(pty))
        .stderr(slave(pty));
    unsafe {
        command.pre_exec(|| {
            if libc::setsid() == -1 {
                return Err(std::io::Error::last_os_error());
            }
            if libc::ioctl(0, libc::TIOCSCTTY as _, 0) == -1 {
                return Err(std::io::Error::last_os_error());
            }
            Ok(())
        });
    }
    command.spawn().unwrap()
}

fn wait_code(child: &mut Child) -> Option<i32> {
    let deadline = Instant::now() + PATIENCE;
    while Instant::now() < deadline {
        if let Some(status) = child.try_wait().unwrap() {
            return status.code();
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    let _ = child.kill();
    let _ = child.wait();
    None
}

#[test]
fn a_command_can_read_the_terminal() {
    let pty = open_pty();
    let mut child = spawn_on(
        &pty,
        bin(),
        &["--dry-run", "-q", "--", "sh", "-c", "read x; echo got:$x"],
        &[],
    );
    std::thread::sleep(Duration::from_millis(500));

    pty.write(b"hello\n");
    let mut seen = String::new();
    pty.read_until(&mut seen, "got:hello");
    assert_eq!(wait_code(&mut child), Some(0));
}

#[test]
fn ctrl_c_reaches_the_command_and_its_status_is_passed_on() {
    let pty = open_pty();
    let mut child = spawn_on(
        &pty,
        bin(),
        &[
            "--dry-run",
            "-q",
            "--",
            "sh",
            "-c",
            "echo ready; exec sleep 30",
        ],
        &[],
    );
    let mut seen = String::new();
    pty.read_until(&mut seen, "ready");
    std::thread::sleep(Duration::from_millis(300));

    pty.write(&[0x03]); // Ctrl-C
    // The command was interrupted by SIGINT itself: 128 + 2.
    assert_eq!(wait_code(&mut child), Some(130));
}

#[test]
fn the_terminal_is_wayinhibits_again_once_the_command_has_ended() {
    let dir = std::env::temp_dir().join(format!("wayinhibit-tty-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let foreground = dir.join("foreground");
    let own_group = dir.join("own-group");
    // The release hook runs after the command ended: it records who owns the terminal and
    // which group wayinhibit is in.
    let hook = format!(
        "ps -o tpgid= -p $$ > '{}'; ps -o pgid= -p $PPID > '{}'",
        foreground.display(),
        own_group.display()
    );

    let pty = open_pty();
    let mut child = spawn_on(
        &pty,
        bin(),
        &["--dry-run", "-q", "--on-release", &hook, "--", "true"],
        &[],
    );
    assert_eq!(wait_code(&mut child), Some(0));

    let read = |path: &std::path::Path| std::fs::read_to_string(path).unwrap().trim().to_owned();
    let (foreground_group, wayinhibit_group) = (read(&foreground), read(&own_group));
    let _ = std::fs::remove_dir_all(&dir);
    assert_eq!(foreground_group, wayinhibit_group);
}

#[test]
fn ctrl_z_suspends_the_job_and_fg_resumes_it_with_the_terminal() {
    let Some(bash) = std::env::var_os("PATH").and_then(|paths| {
        std::env::split_paths(&paths)
            .map(|dir| dir.join("bash"))
            .find(|candidate| candidate.is_file())
    }) else {
        eprintln!("skipped: bash not found");
        return;
    };

    let pty = open_pty();
    let mut shell = spawn_on(
        &pty,
        bash,
        &["--norc", "--noprofile", "-i"],
        &[("PS1", "PROMPT> "), ("TERM", "dumb"), ("LC_ALL", "C")],
    );
    let mut seen = String::new();
    pty.read_until(&mut seen, "PROMPT> ");

    let command = format!(
        "{} --dry-run -q -- sh -c 'echo ready; read x; echo got:$x'\n",
        bin()
    );
    pty.write(command.as_bytes());
    pty.read_until(&mut seen, "ready");
    std::thread::sleep(Duration::from_millis(300));

    pty.write(&[0x1a]); // Ctrl-Z
    pty.read_until(&mut seen, "Stopped");
    pty.read_until(&mut seen, "PROMPT> ");

    seen.clear();
    pty.write(b"fg\n");
    std::thread::sleep(Duration::from_millis(500));
    pty.write(b"hi\n");
    pty.read_until(&mut seen, "got:hi");

    pty.write(b"exit\n");
    assert_eq!(wait_code(&mut shell), Some(0));
}
