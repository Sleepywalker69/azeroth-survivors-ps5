//! The host on a desktop. On the console the crate is linked into the title instead.

fn main() {
    #[cfg(windows)]
    std::process::exit(aethus_host::desktop::run());
    #[cfg(not(windows))]
    {
        eprintln!("aethus-host: this platform has no desktop host; on the PS5 the crate is linked into the title");
        std::process::exit(2);
    }
}
