//! Spawning external tools (git, pandoc, TeX, llama.cpp, …).
//!
//! The desktop app is a GUI-subsystem program, so on Windows every console
//! tool it starts would flash its own terminal window. Always build commands
//! through these helpers instead of `Command::new`.

use std::ffi::OsStr;

#[cfg(windows)]
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

/// `tokio::process::Command::new` without a console window on Windows.
pub fn cmd(program: impl AsRef<OsStr>) -> tokio::process::Command {
    #[allow(unused_mut)]
    let mut c = tokio::process::Command::new(program);
    #[cfg(windows)]
    c.creation_flags(CREATE_NO_WINDOW);
    c
}

/// `std::process::Command::new` without a console window on Windows.
pub fn std_cmd(program: impl AsRef<OsStr>) -> std::process::Command {
    #[allow(unused_mut)]
    let mut c = std::process::Command::new(program);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        c.creation_flags(CREATE_NO_WINDOW);
    }
    c
}
