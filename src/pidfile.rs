//! A pid file that also guarantees a single running instance.
//!
//! The file holds the PID of the running instance (scripts read it with `cat`, status bars
//! test for its existence) and the instance keeps a POSIX record lock on it for its whole
//! life. Who runs is therefore decided by the kernel, not by what the file says:
//!
//! * the kernel drops the lock when the holder dies, however it dies, so a stale file never
//!   blocks a new instance and never has to be "validated";
//! * `F_GETLK` reports the PID of the process that really holds the lock. Nothing is read
//!   from the file to decide whom to signal, so a PID left behind by a dead instance (and
//!   possibly recycled by an unrelated program since) can never be mistaken for a live one;
//! * checking the lock and taking it is one atomic step, so two instances started at the
//!   same instant cannot both win.
//!
//! POSIX locks belong to the process, not to the file descriptor: closing *any* descriptor
//! this process has on the file drops the lock. [`acquire`] is therefore the only place that
//! opens the pid file, and it is called once per run. Do not open the file anywhere else in
//! this process.

use std::fs::{File, OpenOptions};
use std::io;
use std::os::fd::AsRawFd;
use std::os::unix::fs::{FileExt, MetadataExt, OpenOptionsExt};
use std::path::{Path, PathBuf};

/// Attempts before giving up when the file keeps being replaced or released under us.
/// Each retry means another process won a race, so hitting the limit is not a real scenario.
const MAX_ATTEMPTS: usize = 100;

/// The result of trying to become the instance that owns a pid file.
pub enum Acquired {
    /// We hold the lock; the file now contains our PID.
    Owner(PidFile),
    /// Another live process holds the lock. Carries its PID, as reported by the kernel.
    Held(u32),
}

/// Ownership of a locked pid file. Dropping it removes the file and releases the lock.
pub struct PidFile {
    path: PathBuf,
    file: File,
}

/// Locks `path` and writes our PID to it, or reports who already holds it.
pub fn acquire(path: &Path) -> Result<Acquired, String> {
    let describe = |what: &str, err: io::Error| format!("{what} '{}': {err}", path.display());

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
            .map_err(|err| describe("failed to open PID file", err))?;

        if try_lock(&file).map_err(|err| describe("failed to lock PID file", err))? {
            if !names_same_file(&file, path) {
                // The previous owner unlinked the file between our open and our lock: we
                // locked an orphan. Start over on whatever is at `path` now.
                continue;
            }
            file.set_len(0)
                .and_then(|()| file.write_all_at(format!("{}\n", std::process::id()).as_bytes(), 0))
                .map_err(|err| describe("failed to write PID file", err))?;
            return Ok(Acquired::Owner(PidFile {
                path: path.to_owned(),
                file,
            }));
        }

        match holder(&file).map_err(|err| describe("failed to inspect lock on PID file", err))? {
            Some(0) => {
                return Err(format!(
                    "PID file '{}' is locked by a process outside this PID namespace",
                    path.display()
                ));
            }
            Some(pid) => return Ok(Acquired::Held(pid)),
            // The holder exited between our two calls: try to take the lock again.
            None => {}
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

/// True if `path` still names the very file `file` was opened from.
fn names_same_file(file: &File, path: &Path) -> bool {
    match (file.metadata(), std::fs::symlink_metadata(path)) {
        (Ok(a), Ok(b)) => a.dev() == b.dev() && a.ino() == b.ino(),
        _ => false,
    }
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
/// means the holder lives in another PID namespace and has no meaningful PID for us.
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

    // Cross-process behavior (contention, refusal, takeover after a crash) cannot be tested
    // here: POSIX locks never conflict inside one process. See tests/pid_file.rs.

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
            Acquired::Held(pid) => panic!("unexpectedly held by {pid}"),
        }
    }

    #[test]
    fn writes_our_pid_and_removes_the_file_on_drop() {
        let dir = TempDir::new("basic");
        let path = dir.join("w.pid");
        let guard = owner(&path);
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
}
