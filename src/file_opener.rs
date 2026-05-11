use color_eyre::{Result, eyre::eyre};
use std::{path::Path, process::Command};

pub(crate) fn open_path(path: &Path) -> Result<()> {
    let status = opener_command(path).status()?;
    if status.success() {
        Ok(())
    } else {
        Err(eyre!("file opener exited with status {status}"))
    }
}

#[cfg(target_os = "macos")]
fn opener_command(path: &Path) -> Command {
    let mut command = Command::new("open");
    command.arg(path);
    command
}

#[cfg(target_os = "windows")]
fn opener_command(path: &Path) -> Command {
    let mut command = Command::new("cmd");
    command.args(["/C", "start", ""]);
    command.arg(path);
    command
}

#[cfg(all(unix, not(target_os = "macos")))]
fn opener_command(path: &Path) -> Command {
    let mut command = Command::new("xdg-open");
    command.arg(path);
    command
}
