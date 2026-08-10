use std::{path::Path, process::Command};

#[cfg(target_os = "macos")]
pub(crate) fn opener_command(path: &Path) -> Command {
    let mut command = Command::new("open");
    command.arg(path);
    command
}

#[cfg(target_os = "windows")]
pub(crate) fn opener_command(path: &Path) -> Command {
    windows_opener_command(path)
}

#[cfg(any(test, target_os = "windows"))]
pub(crate) fn windows_opener_command(path: &Path) -> Command {
    let mut command = Command::new("rundll32.exe");
    command.arg("url.dll,FileProtocolHandler");
    command.arg(path);
    command
}

#[cfg(all(unix, not(target_os = "macos")))]
pub(crate) fn opener_command(path: &Path) -> Command {
    let mut command = Command::new("xdg-open");
    command.arg(path);
    command
}

#[cfg(test)]
mod tests {
    use super::windows_opener_command;
    use std::path::Path;

    #[test]
    fn windows_opener_is_shell_free_and_preserves_path_as_one_argument() {
        let target = Path::new(r"C:\Telegram\report & calc.exe");
        let command = windows_opener_command(target);
        let args = command.get_args().collect::<Vec<_>>();

        assert_eq!(command.get_program(), "rundll32.exe");
        assert_eq!(args.len(), 2);
        assert_eq!(args[0], "url.dll,FileProtocolHandler");
        assert_eq!(args[1], target.as_os_str());
        assert!(!args.iter().any(|arg| *arg == "cmd" || *arg == "/C"));
    }
}
