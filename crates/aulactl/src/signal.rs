//! Ctrl-C / SIGTERM as a flag, so the travel test and calibration are closed by their guards instead of the
//! process dying with the keyboard still streaming.

use std::sync::atomic::{AtomicBool, Ordering};

static INTERRUPTED: AtomicBool = AtomicBool::new(false);

extern "C" fn on_signal(_: libc::c_int) {
    INTERRUPTED.store(true, Ordering::SeqCst);
}

/// Catches SIGINT and SIGTERM from now on. Without SA_RESTART a blocking poll returns early. A wait for the
/// device lock gives up on the signal too, so nothing is started after Ctrl-C.
pub fn install() -> Result<(), String> {
    aula_core::hidraw::set_lock_abort(interrupted);
    for sig in [libc::SIGINT, libc::SIGTERM] {
        // SAFETY: a zeroed sigaction with a handler that only stores to an atomic is async-signal-safe.
        let r = unsafe {
            let mut sa: libc::sigaction = std::mem::zeroed();
            sa.sa_sigaction = on_signal as extern "C" fn(libc::c_int) as libc::sighandler_t;
            libc::sigemptyset(&mut sa.sa_mask);
            libc::sigaction(sig, &sa, std::ptr::null_mut())
        };
        if r != 0 {
            return Err(format!(
                "signal handler: {}",
                std::io::Error::last_os_error()
            ));
        }
    }
    Ok(())
}

pub fn interrupted() -> bool {
    INTERRUPTED.load(Ordering::SeqCst)
}
