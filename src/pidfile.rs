//! A pid file that also tells which instance owns it.
//!
//! The file holds the PID of the running instance (scripts read it with `cat`, status bars
//! test for its existence) and the owner keeps a POSIX record lock on it for its whole life.
//! Who owns the file is therefore decided by the kernel, not by what the file says:
//!
//! * the kernel drops the lock when the owner dies, however it dies, so a stale file never
//!   blocks a new instance and never has to be "validated";
//! * `F_GETLK` reports the PID of the process that really holds the lock. Nothing is read
//!   from the file to decide whom to signal, so a PID left behind by a dead instance (and
//!   possibly recycled by an unrelated program since) can never be mistaken for a live one;
//! * checking the lock and taking it is one atomic step, so two instances started at the
//!   same instant cannot both own the file.
//!
//! POSIX locks belong to the process, not to the file descriptor: closing *any* descriptor
//! this process has on the file drops the lock. [`acquire`] is therefore the only place that
//! opens the pid file, and it is called once per run. Do not open the file anywhere else in
//! this process.
//!
//! Some filesystems (certain network or FUSE mounts) cannot lock at all. Rather than refuse
//! to work there, [`acquire`] falls back to a best-effort check of the PID recorded in the
//! file, and the returned [`PidFile`] says so through [`PidFile::lock_problem`].

use std::fs::{File, OpenOptions};
use std::io;
use std::os::fd::AsRawFd;
use std::os::unix::fs::{FileExt, MetadataExt, OpenOptionsExt};
use std::path::{Path, PathBuf};

const BIN_NAME: &str = env!("CARGO_PKG_NAME");

/// Attempts before giving up when the file keeps being replaced or released under us.
/// Each retry means another process won a race, so hitting the limit is not a real scenario.
const MAX_ATTEMPTS: usize = 100;

/// The result of trying to become the owner of a pid file.
pub enum Acquired {
    /// We own the file; it now contains our PID.
    Owner(PidFile),
    /// Another live instance owns it. Carries its PID, or `None` when it lives in another PID
    /// namespace and the kernel cannot give us a PID that means anything here.
    Held(Option<u32>),
}

/// Ownership of a pid file. Dropping it removes the file and releases the lock.
pub struct PidFile {
    path: PathBuf,
    file: File,
    /// Why the file could not be locked, if that is the case.
    lock_problem: Option<String>,
}

impl PidFile {
    /// `Some(reason)` when the filesystem does not support locking: the file is written, but
    /// nothing guarantees that it has a single owner.
    pub fn lock_problem(&self) -> Option<&str> {
        self.lock_problem.as_deref()
    }
}

/// Locks `path` and writes our PID to it, or reports who already owns it.
pub fn acquire(path: &Path) -> Result<Acquired, String> {
    acquire_with(path, try_lock, holder)
}

/// [`acquire`] with the two kernel lock operations injected, so that outcomes that cannot be
/// provoked on a local filesystem can be tested.
fn acquire_with(
    path: &Path,
    try_lock: impl Fn(&File) -> io::Result<bool>,
    holder: impl Fn(&File) -> io::Result<Option<u32>>,
) -> Result<Acquired, String> {
    let describe =
        |what: &str, err: &dyn std::fmt::Display| format!("{what} '{}': {err}", path.display());

    for _ in 0..MAX_ATTEMPTS {
        // Not truncated, not followed through a symlink: an attacker-planted link in a shared
        // directory must not get its target emptied, and a live owner's PID must stay readable
        // until we really own the file.
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .custom_flags(libc::O_NOFOLLOW)
            .open(path)
            .map_err(|err| describe("failed to open PID file", &err))?;

        match try_lock(&file) {
            Ok(true) => {
                if !names_same_file(&file, path) {
                    // The previous owner unlinked the file between our open and our lock: we
                    // locked an orphan. Start over on whatever is at `path` now.
                    continue;
                }
                write_own_pid(&file).map_err(|err| describe("failed to write PID file", &err))?;
                return Ok(Acquired::Owner(PidFile {
                    path: path.to_owned(),
                    file,
                    lock_problem: None,
                }));
            }
            Ok(false) => {
                match holder(&file)
                    .map_err(|err| describe("failed to inspect lock on PID file", &err))?
                {
                    // 0: the holder lives in another PID namespace.
                    Some(0) => return Ok(Acquired::Held(None)),
                    Some(pid) => return Ok(Acquired::Held(Some(pid))),
                    // The holder exited between our two calls: try to take the lock again.
                    None => {}
                }
            }
            Err(err) if lock_unsupported(&err) => {
                return without_locks(path, file, &err)
                    .map_err(|err| describe("failed to use PID file", &err));
            }
            Err(err) => return Err(describe("failed to lock PID file", &err)),
        }
    }

    Err(format!(
        "PID file '{}' kept changing while trying to lock it",
        path.display()
    ))
}

impl Drop for PidFile {
    fn drop(&mut self) {
        // Only remove what is still ours: someone may have deleted the file and a new
        // instance created another one at the same path. Unlink before the lock is released
        // (the descriptor closes after this body), so nobody can lock the doomed file and
        // mistake it for a live one; a latecomer that already opened it will notice in
        // `names_same_file` and retry.
        if names_same_file(&self.file, &self.path) {
            let _ = std::fs::remove_file(&self.path);
        }
    }
}

fn write_own_pid(file: &File) -> io::Result<()> {
    file.set_len(0)?;
    file.write_all_at(format!("{}\n", std::process::id()).as_bytes(), 0)
}

/// True if `path` still names the very file `file` was opened from.
fn names_same_file(file: &File, path: &Path) -> bool {
    match (file.metadata(), std::fs::symlink_metadata(path)) {
        (Ok(a), Ok(b)) => a.dev() == b.dev() && a.ino() == b.ino(),
        _ => false,
    }
}

/// The errors a filesystem answers with when it has no locking at all (as opposed to a lock
/// being held, or a genuine failure).
fn lock_unsupported(err: &io::Error) -> bool {
    matches!(
        err.raw_os_error(),
        Some(libc::ENOLCK | libc::ENOSYS | libc::EOPNOTSUPP)
    )
}

/// Best effort for filesystems that cannot lock: believe the PID recorded in the file only
/// if it names a live wayinhibit process. Racy by nature (two simultaneous starts can both
/// pass), which is why the caller is told that locking is unavailable.
fn without_locks(path: &Path, file: File, reason: &io::Error) -> io::Result<Acquired> {
    let mut buf = [0u8; 32];
    let len = file.read_at(&mut buf, 0)?;
    let recorded = std::str::from_utf8(&buf[..len])
        .ok()
        .and_then(|text| text.trim().parse::<u32>().ok());
    if let Some(pid) = recorded {
        // A recorded PID equal to ours is a stale file whose PID we happen to have been given.
        if pid != std::process::id() && looks_like_wayinhibit(pid) {
            return Ok(Acquired::Held(Some(pid)));
        }
    }
    write_own_pid(&file)?;
    Ok(Acquired::Owner(PidFile {
        path: path.to_owned(),
        file,
        lock_problem: Some(reason.to_string()),
    }))
}

/// True if `pid` is a live process running this program. Used only when locking is
/// unavailable. The resolved executable path is compared first, which sees through
/// symlinks; it is unreadable for other users' processes and suffixed " (deleted)" after an
/// upgrade, so a mismatch falls back to the process name. The kernel keeps only the first 15
/// bytes of that name in `comm`, hence the truncation.
fn looks_like_wayinhibit(pid: u32) -> bool {
    let own_path = std::env::current_exe().ok();
    if let (Ok(exe), Some(own)) = (std::fs::read_link(format!("/proc/{pid}/exe")), &own_path) {
        if exe == *own {
            return true;
        }
    }
    let Ok(comm) = std::fs::read_to_string(format!("/proc/{pid}/comm")) else {
        return false;
    };
    let comm = comm.trim_end();
    let own_name = own_path.and_then(|p| p.file_name().map(|n| n.to_string_lossy().into_owned()));
    [Some(BIN_NAME.to_owned()), own_name]
        .into_iter()
        .flatten()
        .any(|name| comm.as_bytes() == &name.as_bytes()[..name.len().min(15)])
}

fn whole_file_write_lock() -> libc::flock {
    // SAFETY: `flock` is a plain C struct for which all-zero bytes is a valid value.
    let mut request: libc::flock = unsafe { std::mem::zeroed() };
    request.l_type = libc::F_WRLCK as _;
    request.l_whence = libc::SEEK_SET as _;
    request.l_start = 0;
    request.l_len = 0; // 0 means "to the end of the file, however long it grows"
    request
}

/// Takes the lock without waiting. `Ok(false)` means someone else holds it.
fn try_lock(file: &File) -> io::Result<bool> {
    let request = whole_file_write_lock();
    loop {
        // SAFETY: `file` is an open descriptor and `request` outlives the call.
        if unsafe { libc::fcntl(file.as_raw_fd(), libc::F_SETLK, &request) } == 0 {
            return Ok(true);
        }
        let err = io::Error::last_os_error();
        match err.raw_os_error() {
            Some(libc::EAGAIN | libc::EACCES) => return Ok(false),
            Some(libc::EINTR) => {}
            _ => return Err(err),
        }
    }
}

/// PID of the process holding the lock, or `None` if nobody does (any more). A PID of 0
/// means the holder lives in another PID namespace.
fn holder(file: &File) -> io::Result<Option<u32>> {
    let mut request = whole_file_write_lock();
    loop {
        // SAFETY: as in `try_lock`; the kernel writes the blocking lock back into `request`.
        if unsafe { libc::fcntl(file.as_raw_fd(), libc::F_GETLK, &mut request) } == 0 {
            break;
        }
        let err = io::Error::last_os_error();
        if err.raw_os_error() != Some(libc::EINTR) {
            return Err(err);
        }
    }
    if request.l_type == libc::F_UNLCK as _ {
        return Ok(None);
    }
    Ok(Some(u32::try_from(request.l_pid).unwrap_or(0)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::symlink;
    use std::process::{Child, Command};
    use std::time::Duration;

    // Cross-process behavior (contention, takeover after a crash) cannot be tested here:
    // POSIX locks never conflict inside one process. See tests/pid_file.rs. The unsupported
    // filesystem case is exercised by injecting the lock failure.

    struct TempDir(PathBuf);

    impl TempDir {
        fn new(name: &str) -> Self {
            let dir =
                std::env::temp_dir().join(format!("wayinhibit-{}-{name}", std::process::id()));
            let _ = std::fs::remove_dir_all(&dir);
            std::fs::create_dir_all(&dir).unwrap();
            Self(dir)
        }

        fn join(&self, name: &str) -> PathBuf {
            self.0.join(name)
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn owner(path: &Path) -> PidFile {
        match acquire(path).unwrap() {
            Acquired::Owner(pid_file) => pid_file,
            Acquired::Held(pid) => panic!("unexpectedly held by {pid:?}"),
        }
    }

    // --- with working locks ---

    #[test]
    fn writes_our_pid_and_removes_the_file_on_drop() {
        let dir = TempDir::new("basic");
        let path = dir.join("w.pid");
        let guard = owner(&path);
        assert_eq!(guard.lock_problem(), None);
        assert_eq!(
            std::fs::read_to_string(&path).unwrap(),
            format!("{}\n", std::process::id())
        );
        drop(guard);
        assert!(!path.exists());
    }

    #[test]
    fn takes_over_a_stale_file_and_replaces_all_of_its_content() {
        let dir = TempDir::new("stale");
        let path = dir.join("w.pid");
        std::fs::write(
            &path,
            "99999999999999999999 stale and much longer than a PID\n",
        )
        .unwrap();
        let _guard = owner(&path);
        assert_eq!(
            std::fs::read_to_string(&path).unwrap(),
            format!("{}\n", std::process::id())
        );
    }

    #[test]
    fn fails_cleanly_when_the_directory_is_missing() {
        let dir = TempDir::new("nodir");
        let err = acquire(&dir.join("missing/w.pid")).err().unwrap();
        assert!(err.contains("failed to open PID file"), "{err}");
    }

    #[test]
    fn fails_cleanly_when_the_path_is_a_directory() {
        let dir = TempDir::new("isdir");
        assert!(acquire(&dir.0).is_err());
    }

    #[test]
    fn refuses_to_follow_a_symlink_and_leaves_its_target_alone() {
        let dir = TempDir::new("symlink");
        let target = dir.join("precious");
        std::fs::write(&target, "do not touch").unwrap();
        let path = dir.join("w.pid");
        symlink(&target, &path).unwrap();
        assert!(acquire(&path).is_err());
        assert_eq!(std::fs::read_to_string(&target).unwrap(), "do not touch");
    }

    #[test]
    fn drop_does_not_remove_a_file_that_replaced_ours() {
        let dir = TempDir::new("replaced");
        let path = dir.join("w.pid");
        let guard = owner(&path);
        std::fs::remove_file(&path).unwrap();
        std::fs::write(&path, "4242\n").unwrap(); // a newer instance's file
        drop(guard);
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "4242\n");
    }

    #[test]
    fn drop_copes_with_the_file_having_been_deleted() {
        let dir = TempDir::new("deleted");
        let path = dir.join("w.pid");
        let guard = owner(&path);
        std::fs::remove_file(&path).unwrap();
        drop(guard);
        assert!(!path.exists());
    }

    #[test]
    fn names_same_file_tells_a_replacement_from_the_original() {
        let dir = TempDir::new("same");
        let path = dir.join("w.pid");
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(&path)
            .unwrap();
        assert!(names_same_file(&file, &path));
        std::fs::remove_file(&path).unwrap();
        assert!(!names_same_file(&file, &path), "unlinked");
        std::fs::write(&path, "").unwrap();
        assert!(!names_same_file(&file, &path), "replaced by another inode");
    }

    #[test]
    fn nobody_holds_a_lock_that_this_process_owns() {
        // F_GETLK never reports the caller's own locks, which is what lets a fresh
        // instance distinguish "someone else" from "me".
        let dir = TempDir::new("self");
        let path = dir.join("w.pid");
        let _guard = owner(&path);
        let probe = OpenOptions::new()
            .read(true)
            .write(true)
            .open(&path)
            .unwrap();
        assert_eq!(holder(&probe).unwrap(), None);
        // `probe` closes here and, being the same process, drops the lock: the reason why
        // the module documentation forbids opening the file anywhere else.
    }

    // --- lock outcomes, injected ---

    fn errno(code: i32) -> io::Error {
        io::Error::from_raw_os_error(code)
    }

    #[test]
    fn a_pid_in_another_namespace_is_reported_as_held_without_a_pid() {
        let dir = TempDir::new("namespace");
        let path = dir.join("w.pid");
        let result = acquire_with(&path, |_| Ok(false), |_| Ok(Some(0))).unwrap();
        assert!(matches!(result, Acquired::Held(None)));
    }

    #[test]
    fn a_holder_that_vanishes_between_the_two_calls_is_retried() {
        let dir = TempDir::new("vanish");
        let path = dir.join("w.pid");
        let attempts = std::cell::Cell::new(0);
        let result = acquire_with(
            &path,
            |_| {
                attempts.set(attempts.get() + 1);
                Ok(attempts.get() > 1) // locked by someone, then free
            },
            |_| Ok(None),
        )
        .unwrap();
        assert!(matches!(result, Acquired::Owner(_)));
        assert_eq!(attempts.get(), 2);
    }

    #[test]
    fn gives_up_instead_of_looping_forever() {
        let dir = TempDir::new("forever");
        let path = dir.join("w.pid");
        let err = acquire_with(&path, |_| Ok(false), |_| Ok(None))
            .err()
            .unwrap();
        assert!(err.contains("kept changing"), "{err}");
    }

    #[test]
    fn other_lock_errors_are_real_errors() {
        let dir = TempDir::new("eio");
        let path = dir.join("w.pid");
        let err = acquire_with(&path, |_| Err(errno(libc::EIO)), |_| Ok(None))
            .err()
            .unwrap();
        assert!(err.contains("failed to lock PID file"), "{err}");
    }

    #[test]
    fn only_the_no_locking_errnos_count_as_unsupported() {
        for code in [libc::ENOLCK, libc::ENOSYS, libc::EOPNOTSUPP] {
            assert!(lock_unsupported(&errno(code)), "{code}");
        }
        for code in [
            libc::EAGAIN,
            libc::EACCES,
            libc::EPERM,
            libc::EIO,
            libc::EBADF,
        ] {
            assert!(!lock_unsupported(&errno(code)), "{code}");
        }
    }

    // --- filesystems without locking ---

    fn no_locks(path: &Path) -> Result<Acquired, String> {
        acquire_with(path, |_| Err(errno(libc::ENOLCK)), |_| Ok(None))
    }

    /// Not a real test: the process the tests below run as a live "wayinhibit", i.e. this very
    /// test binary, as opposed to an unrelated program.
    #[test]
    #[ignore = "helper process for the tests below"]
    fn sleep_helper() {
        std::thread::sleep(Duration::from_secs(30));
    }

    struct Guarded(Child);

    impl Drop for Guarded {
        fn drop(&mut self) {
            let _ = self.0.kill();
            let _ = self.0.wait();
        }
    }

    fn run_helper(program: impl AsRef<std::ffi::OsStr>) -> Guarded {
        Guarded(
            Command::new(program)
                .args(["--ignored", "--exact", "pidfile::tests::sleep_helper"])
                .spawn()
                .unwrap(),
        )
    }

    fn unrelated_process() -> Guarded {
        Guarded(Command::new("sleep").arg("30").spawn().unwrap())
    }

    fn assert_owner_without_locks(path: &Path) {
        match no_locks(path).unwrap() {
            Acquired::Owner(guard) => {
                assert!(
                    guard
                        .lock_problem()
                        .is_some_and(|reason| !reason.is_empty())
                );
                assert_eq!(
                    std::fs::read_to_string(path).unwrap(),
                    format!("{}\n", std::process::id())
                );
            }
            Acquired::Held(pid) => panic!("unexpectedly held by {pid:?}"),
        }
        assert!(!path.exists(), "file removed on drop");
    }

    #[test]
    fn without_locks_a_fresh_start_still_works_and_says_so() {
        let dir = TempDir::new("nolock-fresh");
        assert_owner_without_locks(&dir.join("w.pid"));
    }

    #[test]
    fn without_locks_a_live_instance_named_in_the_file_is_respected() {
        let dir = TempDir::new("nolock-live");
        let path = dir.join("w.pid");
        let other = run_helper(std::env::current_exe().unwrap());
        std::fs::write(&path, format!("{}\n", other.0.id())).unwrap();
        match no_locks(&path).unwrap() {
            Acquired::Held(Some(pid)) => assert_eq!(pid, other.0.id()),
            _ => panic!("expected the live instance to be reported"),
        }
        assert_eq!(
            std::fs::read_to_string(&path).unwrap(),
            format!("{}\n", other.0.id()),
            "its file must be left alone"
        );
    }

    #[test]
    fn without_locks_an_instance_started_through_a_symlink_is_recognised() {
        let dir = TempDir::new("nolock-symlink");
        let path = dir.join("w.pid");
        let link = dir.join("link-to-self");
        symlink(std::env::current_exe().unwrap(), &link).unwrap();
        let other = run_helper(&link);
        // Wait for exec: comm is then the link's name, which differs from the real binary's.
        let comm = format!("/proc/{}/comm", other.0.id());
        while std::fs::read_to_string(&comm)
            .unwrap_or_default()
            .trim_end()
            != "link-to-self"
        {
            std::thread::sleep(Duration::from_millis(10));
        }
        std::fs::write(&path, format!("{}\n", other.0.id())).unwrap();
        assert!(matches!(no_locks(&path).unwrap(), Acquired::Held(Some(_))));
    }

    #[test]
    fn without_locks_an_unrelated_process_in_the_file_is_not_an_instance() {
        let dir = TempDir::new("nolock-unrelated");
        let path = dir.join("w.pid");
        let stranger = unrelated_process();
        std::fs::write(&path, format!("{}\n", stranger.0.id())).unwrap();
        assert_owner_without_locks(&path);
    }

    #[test]
    fn without_locks_dead_or_unreadable_content_is_taken_over() {
        let dir = TempDir::new("nolock-stale");
        let path = dir.join("w.pid");
        let mut finished = Command::new("true").spawn().unwrap();
        let dead_pid = finished.id();
        finished.wait().unwrap();
        for content in [
            format!("{dead_pid}\n"),
            String::new(),
            "abc".into(),
            "-1".into(),
            "4294967296".into(),
            "\0\0\0".into(),
        ] {
            std::fs::write(&path, &content).unwrap();
            assert_owner_without_locks(&path);
        }
    }

    #[test]
    fn without_locks_our_own_pid_in_the_file_is_stale_not_an_instance() {
        // The PID we were given may be the one a dead instance used: never "stop" ourselves.
        let dir = TempDir::new("nolock-self");
        let path = dir.join("w.pid");
        std::fs::write(&path, format!("{}\n", std::process::id())).unwrap();
        assert_owner_without_locks(&path);
    }
}
