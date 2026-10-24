//! Android: what the standard library does not do for us.
use std::ffi::CString;
use std::io::{BufRead, BufReader};
use std::os::fd::FromRawFd;

/// Sends what the program prints on stderr to logcat (tag `sunpetal`), where nothing else shows it.
pub fn redirect_stderr() {
    let mut fds = [0; 2];
    // SAFETY: plain libc calls on descriptors this function has just made
    unsafe {
        if libc::pipe(fds.as_mut_ptr()) != 0 || libc::dup2(fds[1], 2) < 0 {
            return;
        }
        let read = std::fs::File::from_raw_fd(fds[0]);
        std::thread::spawn(move || {
            let tag = CString::new("sunpetal").unwrap();
            for line in BufReader::new(read).lines().map_while(Result::ok) {
                if let Ok(text) = CString::new(line) {
                    android_log_sys::__android_log_write(android_log_sys::LogPriority::INFO as i32, tag.as_ptr(), text.as_ptr());
                }
            }
        });
    }
}
