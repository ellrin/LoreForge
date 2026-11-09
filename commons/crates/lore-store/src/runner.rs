//! Injectable external commands (`brctl download …`). Tests use a fake runner; the
//! real one is [`SystemRunner`].

use std::process::Command;

#[derive(Debug, Clone, Default)]
pub struct CommandOutput {
    pub status: i32,
    pub stdout: String,
    pub stderr: String,
}

pub trait CommandRunner: Send + Sync {
    fn run(&self, program: &str, args: &[&str]) -> std::io::Result<CommandOutput>;
}

/// Runs the real program. **Not tested on real hardware** for `brctl` (tests mock it).
#[derive(Debug, Default, Clone, Copy)]
pub struct SystemRunner;

impl CommandRunner for SystemRunner {
    fn run(&self, program: &str, args: &[&str]) -> std::io::Result<CommandOutput> {
        let out = Command::new(program).args(args).output()?;
        Ok(CommandOutput {
            status: out.status.code().unwrap_or(-1),
            stdout: String::from_utf8_lossy(&out.stdout).into_owned(),
            stderr: String::from_utf8_lossy(&out.stderr).into_owned(),
        })
    }
}
