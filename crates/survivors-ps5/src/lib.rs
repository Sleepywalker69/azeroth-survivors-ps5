//! The PlayStation 5 launcher: what `warcraft-survivors`'s `main` is on a computer, as a C entry
//! point. The title's own `main` (`ps5/src/main.c`) has already sent standard error to klog and
//! set `$WOW_DATA` and `$BENILLA_HOME` to the title's folder before it calls in here.

use benilla_app::{AppExit, BuildId};

/// Runs Azeroth Survivors to its end and returns its exit status.
#[unsafe(no_mangle)]
pub extern "C" fn survivors_ps5_main() -> i32 {
    let exit = benilla_app::run_survivors(BuildId {
        version: env!("CARGO_PKG_VERSION"),
        describe: env!("BENILLA_GIT_DESCRIBE"),
        sha: env!("BENILLA_GIT_SHA"),
        short: env!("BENILLA_GIT_SHORT"),
        date: env!("BENILLA_GIT_DATE"),
        profile: env!("BENILLA_PROFILE"),
        project_dir: env!("BENILLA_PROJECT_DIR"),
        ..Default::default()
    });
    match exit {
        AppExit::Success => 0,
        AppExit::Error(code) => i32::from(code.get()),
    }
}
