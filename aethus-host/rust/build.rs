// On Windows the test host links the JS side itself: the host library (QuickJS, host.c, wasi.c
// and the generated bridge) and the wasm2c translation of the game, both built by the scripts in
// ../tools into one build folder. The PlayStation 5 title is linked by its own link step, which
// takes these archives and the Rust static library side by side.
use std::env;
use std::path::PathBuf;

fn main() {
    println!("cargo:rerun-if-env-changed=AETHUS_BUILD_DIR");
    let target = env::var("TARGET").unwrap_or_default();
    if !target.contains("windows") {
        return;
    }
    let manifest = PathBuf::from(env::var("CARGO_MANIFEST_DIR").unwrap());
    let dir = env::var_os("AETHUS_BUILD_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| manifest.join("../../aethus-build/0.96.0"));
    for lib in ["aethus_host.lib", "wenilla.lib"] {
        println!("cargo:rerun-if-changed={}", dir.join(lib).display());
    }
    println!("cargo:rustc-link-search=native={}", dir.display());
    // Not bundled into the rlib: the translation's archive is a quarter of a gigabyte, and only
    // the executable's link needs it.
    println!("cargo:rustc-link-lib=static:-bundle=aethus_host");
    println!("cargo:rustc-link-lib=static:-bundle=wenilla");
    // The translated wasm runs on the native stack, as deep as the game's recursion goes.
    println!("cargo:rustc-link-arg-bins=/STACK:268435456");
}
