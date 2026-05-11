use chrono::{SecondsFormat, Utc};
use color_eyre::Result;
use std::{
    fs::{self, File, OpenOptions},
    io::Write,
    path::Path,
    sync::{Mutex, OnceLock},
};

static LOG_FILE: OnceLock<Mutex<File>> = OnceLock::new();

pub fn init(path: impl AsRef<Path>) -> Result<()> {
    let path = path.as_ref();
    if let Some(parent) = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
    {
        fs::create_dir_all(parent)?;
    }
    let file = OpenOptions::new().create(true).append(true).open(path)?;
    let _ = LOG_FILE.set(Mutex::new(file));
    event("log_initialized", format!("path={}", path.display()));
    Ok(())
}

pub fn enabled() -> bool {
    LOG_FILE.get().is_some()
}

pub fn event(name: &str, details: impl AsRef<str>) {
    let Some(file) = LOG_FILE.get() else {
        return;
    };
    let now = Utc::now().to_rfc3339_opts(SecondsFormat::Millis, true);
    if let Ok(mut file) = file.lock() {
        let _ = writeln!(file, "{now} {name} {}", details.as_ref());
        let _ = file.flush();
    }
}
