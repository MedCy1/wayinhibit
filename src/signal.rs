use std::sync::atomic::{AtomicU32, Ordering};

/// How many times a stop was requested (SIGINT, SIGTERM or SIGHUP). The first request asks
/// wayinhibit to stop; a second one also cuts short whatever is slowing the stop down.
static STOP_REQUESTS: AtomicU32 = AtomicU32::new(0);

extern "C" fn handle_shutdown_signal(_: libc::c_int) {
    STOP_REQUESTS.fetch_add(1, Ordering::Relaxed);
}

/// True if `signal` was set to be ignored when the program started, as `nohup` does for SIGHUP.
fn ignored_at_startup(signal: libc::c_int) -> Result<bool, String> {
    // SAFETY: `sigaction` is a plain C struct for which all-zero bytes is a valid value, and
    // passing a null new action only reads the current one.
    unsafe {
        let mut current: libc::sigaction = std::mem::zeroed();
        if libc::sigaction(signal, std::ptr::null(), &mut current) == -1 {
            return Err(format!(
                "failed to read the handler of signal {signal}: {}",
                std::io::Error::last_os_error()
            ));
        }
        Ok(current.sa_sigaction == libc::SIG_IGN)
    }
}

pub fn install() -> Result<(), String> {
    for signal in [libc::SIGINT, libc::SIGTERM, libc::SIGHUP] {
        // A hangup that was ignored on purpose (`nohup wayinhibit ...`) stays ignored: that is
        // the whole point of the wrapper.
        if signal == libc::SIGHUP && ignored_at_startup(signal)? {
            continue;
        }

        // SAFETY: as above; the handler only touches an atomic, which is async-signal-safe.
        unsafe {
            let mut action: libc::sigaction = std::mem::zeroed();
            action.sa_sigaction = handle_shutdown_signal as *const () as libc::sighandler_t;
            action.sa_flags = libc::SA_RESTART;
            if libc::sigaction(signal, &action, std::ptr::null_mut()) == -1 {
                return Err(format!(
                    "failed to install handler for signal {signal}: {}",
                    std::io::Error::last_os_error()
                ));
            }
        }
    }

    Ok(())
}

pub fn stop_requests() -> u32 {
    STOP_REQUESTS.load(Ordering::Relaxed)
}

pub fn is_stop_requested() -> bool {
    stop_requests() > 0
}
