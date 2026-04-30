use std::sync::atomic::{AtomicBool, Ordering};

static SHOULD_STOP: AtomicBool = AtomicBool::new(false);

extern "C" fn handle_shutdown_signal(_: libc::c_int) {
    SHOULD_STOP.store(true, Ordering::Relaxed);
}

pub fn install() -> Result<(), String> {
    unsafe {
        let handler = handle_shutdown_signal as *const () as libc::sighandler_t;

        if libc::signal(libc::SIGINT, handler) == libc::SIG_ERR {
            return Err("failed to install SIGINT handler".to_string());
        }

        if libc::signal(libc::SIGTERM, handler) == libc::SIG_ERR {
            return Err("failed to install SIGTERM handler".to_string());
        }
    }

    Ok(())
}

pub fn is_stop_requested() -> bool {
    SHOULD_STOP.load(Ordering::Relaxed)
}

pub fn reset() {
    SHOULD_STOP.store(false, Ordering::Relaxed);
}
