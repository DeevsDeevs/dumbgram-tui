use crate::paths;
use color_eyre::Result;
use serde::{Deserialize, Deserializer};
use std::{
    fs,
    path::{Path, PathBuf},
};

#[derive(Debug, Deserialize, Clone)]
pub struct TelegramConfig {
    #[serde(deserialize_with = "deserialize_api_id")]
    pub api_id: i32,
    pub api_hash: String,
    #[serde(alias = "session_path", default = "default_session_file")]
    pub session_file: String,
}

fn default_session_file() -> String {
    paths::SESSION_FILE_NAME.to_string()
}

fn deserialize_api_id<'de, D>(deserializer: D) -> std::result::Result<i32, D::Error>
where
    D: Deserializer<'de>,
{
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum ApiId {
        Number(i32),
        String(String),
    }

    match ApiId::deserialize(deserializer)? {
        ApiId::Number(value) => Ok(value),
        ApiId::String(value) => value
            .trim()
            .parse::<i32>()
            .map_err(|_| serde::de::Error::custom("telegram.api_id must be an integer")),
    }
}

#[derive(Debug, Deserialize)]
pub struct Config {
    pub telegram: TelegramConfig,
}

impl Config {
    pub fn load(path: &str) -> Result<Self> {
        let content = fs::read_to_string(path)?;
        let config: Self = toml::from_str(&content)?;
        Ok(config)
    }
}

impl TelegramConfig {
    pub fn session_path_for_config(&self, config_path: &Path) -> PathBuf {
        let session_path = expand_tilde(&self.session_file);
        if session_path.is_absolute() || self.session_file.starts_with('~') {
            session_path
        } else {
            config_path
                .parent()
                .filter(|parent| !parent.as_os_str().is_empty())
                .map(|parent| parent.join(&session_path))
                .unwrap_or(session_path)
        }
    }
}

fn expand_tilde(path: &str) -> PathBuf {
    let home = std::env::var_os("HOME").map(PathBuf::from);
    expand_tilde_with_home(path, home.as_deref())
}

fn expand_tilde_with_home(path: &str, home: Option<&Path>) -> PathBuf {
    match (path, home) {
        ("~", Some(home)) => home.to_path_buf(),
        (path, Some(home)) if path.starts_with("~/") => home.join(&path[2..]),
        _ => PathBuf::from(path),
    }
}

impl Default for TelegramConfig {
    fn default() -> Self {
        Self {
            api_id: 0,
            api_hash: String::new(),
            session_file: default_session_file(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{Config, default_session_file, expand_tilde_with_home};
    use std::path::{Path, PathBuf};

    fn parse_test_config(config: &str) -> Config {
        toml::from_str(config).expect("Telegram test config should parse")
    }

    #[test]
    fn telegram_config_accepts_numeric_api_id_and_session_file() {
        let config = parse_test_config(
            r#"
            [telegram]
            api_id = 12345
            api_hash = "hash"
            session_file = "session.dat"
            "#,
        );

        assert_eq!(config.telegram.api_id, 12345);
        assert_eq!(config.telegram.api_hash, "hash");
        assert_eq!(config.telegram.session_file, "session.dat");
    }

    #[test]
    fn telegram_config_defaults_session_file_when_omitted() {
        let config = parse_test_config(
            r#"
            [telegram]
            api_id = 12345
            api_hash = "hash"
            "#,
        );

        assert_eq!(config.telegram.session_file, default_session_file());
    }

    #[test]
    fn telegram_config_accepts_string_api_id_and_session_path_alias() {
        let config = parse_test_config(
            r#"
            [telegram]
            api_id = "12345"
            api_hash = "hash"
            session_path = "legacy-session.dat"
            "#,
        );

        assert_eq!(config.telegram.api_id, 12345);
        assert_eq!(config.telegram.session_file, "legacy-session.dat");
    }

    #[test]
    fn telegram_config_rejects_non_integer_api_id_string() {
        let error = toml::from_str::<Config>(
            r#"
            [telegram]
            api_id = "not-a-number"
            api_hash = "hash"
            session_file = "session.dat"
            "#,
        )
        .expect_err("invalid api_id should fail");

        assert!(
            error
                .to_string()
                .contains("telegram.api_id must be an integer")
        );
    }

    #[test]
    fn session_path_resolves_relative_to_config_directory() {
        let config = parse_test_config(
            r#"
            [telegram]
            api_id = 12345
            api_hash = "hash"
            session_file = "session.dat"
            "#,
        );

        assert_eq!(
            config
                .telegram
                .session_path_for_config(Path::new("/home/alice/.config/dumbgram/config.toml")),
            PathBuf::from("/home/alice/.config/dumbgram/session.dat")
        );
        assert_eq!(
            config
                .telegram
                .session_path_for_config(Path::new("config.toml")),
            PathBuf::from("session.dat")
        );
    }

    #[test]
    fn session_path_expands_home_prefix() {
        let home = Path::new("/home/alice");

        assert_eq!(
            expand_tilde_with_home("~/.config/dumbgram/session.dat", Some(home)),
            PathBuf::from("/home/alice/.config/dumbgram/session.dat")
        );
        assert_eq!(
            expand_tilde_with_home("~", Some(home)),
            PathBuf::from("/home/alice")
        );
        assert_eq!(
            expand_tilde_with_home("session.dat", Some(home)),
            PathBuf::from("session.dat")
        );
        assert_eq!(
            expand_tilde_with_home("~/.config/dumbgram/session.dat", None),
            PathBuf::from("~/.config/dumbgram/session.dat")
        );
    }
}
