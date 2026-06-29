use std::sync::atomic::{AtomicBool, Ordering};

static SHOULD_STOP: AtomicBool = AtomicBool::new(false);

extern "C" fn handle_shutdown_signal(_: libc::c_int) {
    SHOULD_STOP.store(true, Ordering::Relaxed);
}

pub fn install() -> Result<(), String> {
    unsafe {
        let mut action: libc::sigaction = std::mem::zeroed();
        action.sa_sigaction = handle_shutdown_signal as *const () as libc::sighandler_t;
        action.sa_flags = libc::SA_RESTART;

        if libc::sigaction(libc::SIGINT, &action, std::ptr::null_mut()) == -1 {
            return Err(format!(
                "failed to install SIGINT handler: {}",
                std::io::Error::last_os_error()
            ));
        }
        if libc::sigaction(libc::SIGTERM, &action, std::ptr::null_mut()) == -1 {
            return Err(format!(
                "failed to install SIGTERM handler: {}",
                std::io::Error::last_os_error()
            ));
        }
    }

    Ok(())
}

pub fn is_stop_requested() -> bool {
    SHOULD_STOP.load(Ordering::Relaxed)
}
