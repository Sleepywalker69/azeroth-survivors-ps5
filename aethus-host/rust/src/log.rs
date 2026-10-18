//! The host's log: standard error (klog on the console), and a file in the player's folder when
//! one is open.

use std::fs::File;
use std::io::Write;
use std::path::Path;
use std::sync::Mutex;

static FILE: Mutex<Option<File>> = Mutex::new(None);

/// Also writes the log to `path`, replacing what it held.
pub fn to_file(path: &Path) {
    match File::create(path) {
        Ok(file) => *FILE.lock().unwrap_or_else(|e| e.into_inner()) = Some(file),
        Err(e) => line(2, &format!("host: the log file {} could not be created: {e}", path.display())),
    }
}

/// One line. `level`: 0 debug, 1 log, 2 warning, 3 error.
pub fn line(level: i32, text: &str) {
    let tag = match level {
        0 => "debug",
        1 => "log",
        2 => "warn",
        _ => "error",
    };
    let t = crate::ffi::now_ms() / 1000.0;
    let text = format!("[{t:9.3}] [{tag}] {text}\n");
    let _ = std::io::stderr().write_all(text.as_bytes());
    if let Some(file) = FILE.lock().unwrap_or_else(|e| e.into_inner()).as_mut() {
        let _ = file.write_all(text.as_bytes());
    }
}

macro_rules! say {
    ($($t:tt)*) => { $crate::log::line(1, &format!($($t)*)) };
}
// Named apart from the `warn` lint attribute, which a plain `warn` here would be ambiguous with.
macro_rules! warning {
    ($($t:tt)*) => { $crate::log::line(2, &format!($($t)*)) };
}
macro_rules! fail {
    ($($t:tt)*) => { $crate::log::line(3, &format!($($t)*)) };
}
pub(crate) use {fail, say, warning as warn};
