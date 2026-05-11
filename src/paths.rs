use std::path::PathBuf;

pub const APP_CONFIG_DIR_NAME: &str = "dumbgram";
pub const CONFIG_FILE_NAME: &str = "config.toml";
pub const SESSION_FILE_NAME: &str = "session.dat";

pub fn app_config_dir() -> PathBuf {
    if let Some(path) = non_empty_env_path("DUMBGRAM_CONFIG_HOME") {
        return path;
    }

    platform_config_root()
        .map(|root| root.join(APP_CONFIG_DIR_NAME))
        .unwrap_or_else(|| PathBuf::from(APP_CONFIG_DIR_NAME))
}

pub fn default_config_path() -> PathBuf {
    app_config_dir().join(CONFIG_FILE_NAME)
}

fn platform_config_root() -> Option<PathBuf> {
    if let Some(path) = non_empty_env_path("XDG_CONFIG_HOME") {
        return Some(path);
    }

    #[cfg(target_os = "windows")]
    {
        non_empty_env_path("APPDATA")
            .or_else(|| non_empty_env_path("HOME").map(|home| home.join(".config")))
    }

    #[cfg(not(target_os = "windows"))]
    {
        non_empty_env_path("HOME").map(|home| home.join(".config"))
    }
}

fn non_empty_env_path(key: &str) -> Option<PathBuf> {
    std::env::var_os(key)
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
}

#[cfg(test)]
mod tests {
    use super::{APP_CONFIG_DIR_NAME, CONFIG_FILE_NAME, default_config_path};

    #[test]
    fn default_config_path_uses_shared_file_name() {
        assert_eq!(
            default_config_path()
                .file_name()
                .and_then(|name| name.to_str()),
            Some(CONFIG_FILE_NAME)
        );
        assert!(
            default_config_path()
                .components()
                .any(|component| component.as_os_str() == APP_CONFIG_DIR_NAME)
        );
    }
}
