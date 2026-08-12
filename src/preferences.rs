use crate::{
    state::{AppState, DEFAULT_SPLIT_RATIO, MAX_SPLIT_RATIO, MIN_SPLIT_RATIO},
    telegram::session_file::BoundPrivateDirectory,
};
use color_eyre::Result;
use serde::{Deserialize, Serialize};
use std::{
    io::Read,
    path::{Path, PathBuf},
};

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct AppPreferences {
    #[serde(default)]
    pub ui: UiPreferences,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct UiPreferences {
    #[serde(default = "default_show_help_bar")]
    pub show_help_bar: bool,
    #[serde(default = "default_split_ratio")]
    pub split_ratio: f32,
}

impl Default for UiPreferences {
    fn default() -> Self {
        Self {
            show_help_bar: default_show_help_bar(),
            split_ratio: default_split_ratio(),
        }
    }
}

impl AppPreferences {
    pub fn load(path: &Path) -> Result<Self> {
        let parent = path
            .parent()
            .filter(|parent| !parent.as_os_str().is_empty())
            .unwrap_or(Path::new("."));
        let name = path
            .file_name()
            .ok_or_else(|| color_eyre::eyre::eyre!("preferences path must name a file"))?;
        let directory = BoundPrivateDirectory::bind(parent)?;
        let Some(mut file) = directory.open_file_optional(name)? else {
            return Ok(Self::default());
        };
        let mut content = String::new();
        file.read_to_string(&mut content)?;
        let mut preferences: Self = toml::from_str(&content)?;
        preferences.ui.split_ratio = clamp_split_ratio(preferences.ui.split_ratio);
        Ok(preferences)
    }

    pub fn save(&self, path: &Path) -> Result<()> {
        let parent = path
            .parent()
            .filter(|parent| !parent.as_os_str().is_empty())
            .unwrap_or(Path::new("."));
        let name = path
            .file_name()
            .ok_or_else(|| color_eyre::eyre::eyre!("preferences path must name a file"))?;
        let directory = BoundPrivateDirectory::bind(parent)?;
        let stage = directory.stage("dumbgram-preferences")?;
        let file = stage.write_file("preferences", toml::to_string_pretty(self)?.as_bytes())?;
        let published = stage.publish_replace("preferences", name)?;
        crate::telegram::session_file::verify_private_file_identity(&file, &published)?;
        Ok(())
    }

    pub fn apply_to_state(&self, state: &mut AppState) {
        state.show_help_bar = self.ui.show_help_bar;
        state.split_ratio = clamp_split_ratio(self.ui.split_ratio);
    }

    pub fn from_state(state: &AppState) -> Self {
        Self {
            ui: UiPreferences {
                show_help_bar: state.show_help_bar,
                split_ratio: clamp_split_ratio(state.split_ratio),
            },
        }
    }
}

pub fn state_path_for_config(config_path: &str) -> PathBuf {
    let path = Path::new(config_path);
    let file_name = path
        .file_stem()
        .and_then(|stem| stem.to_str())
        .filter(|stem| !stem.is_empty())
        .map_or_else(|| "dumbgram".to_string(), ToString::to_string);
    path.with_file_name(format!("{file_name}.state.toml"))
}

fn default_show_help_bar() -> bool {
    true
}

fn default_split_ratio() -> f32 {
    DEFAULT_SPLIT_RATIO
}

fn clamp_split_ratio(split_ratio: f32) -> f32 {
    if split_ratio.is_finite() {
        split_ratio.clamp(MIN_SPLIT_RATIO, MAX_SPLIT_RATIO)
    } else {
        DEFAULT_SPLIT_RATIO
    }
}

#[cfg(test)]
mod tests {
    use super::{AppPreferences, state_path_for_config};
    use crate::state::{AppState, MAX_SPLIT_RATIO};
    use std::{
        path::PathBuf,
        time::{SystemTime, UNIX_EPOCH},
    };

    fn private_test_dir(label: &str) -> PathBuf {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path = PathBuf::from(std::env::var_os("HOME").unwrap()).join(format!(
            ".dumbgram-preferences-{label}-{}-{unique}",
            std::process::id()
        ));
        std::fs::create_dir(&path).unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o700)).unwrap();
        }
        path
    }

    #[test]
    fn state_path_sits_next_to_config_path() {
        assert_eq!(
            state_path_for_config("config.toml").to_string_lossy(),
            "config.state.toml"
        );
        assert_eq!(
            state_path_for_config("/tmp/dumbgram/config.toml").to_string_lossy(),
            "/tmp/dumbgram/config.state.toml"
        );
    }

    #[test]
    fn preferences_round_trip_ui_state() {
        let mut state = AppState::new();
        state.show_help_bar = false;
        state.split_ratio = 0.75;

        let preferences = AppPreferences::from_state(&state);
        let mut restored = AppState::new();
        preferences.apply_to_state(&mut restored);

        assert!(!restored.show_help_bar);
        assert_eq!(restored.split_ratio, 0.75);
    }

    #[test]
    fn preferences_save_replaces_symlink_without_truncating_target() {
        let root = private_test_dir("symlink");
        let target = root.join("target");
        std::fs::write(&target, b"keep me").unwrap();
        let path = root.join("config.state.toml");
        assert_eq!(
            AppPreferences::load(&path).unwrap(),
            AppPreferences::default()
        );
        #[cfg(unix)]
        std::os::unix::fs::symlink(&target, &path).unwrap();
        #[cfg(not(unix))]
        std::fs::write(&path, b"old").unwrap();
        let preferences = AppPreferences::default();

        preferences.save(&path).unwrap();

        assert_eq!(std::fs::read(&target).unwrap(), b"keep me");
        assert!(
            !std::fs::symlink_metadata(&path)
                .unwrap()
                .file_type()
                .is_symlink()
        );
        assert_eq!(AppPreferences::load(&path).unwrap(), preferences);
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                std::fs::metadata(&path).unwrap().permissions().mode() & 0o777,
                0o600
            );
        }
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn preferences_clamp_invalid_split_ratio() {
        let preferences = AppPreferences {
            ui: super::UiPreferences {
                show_help_bar: true,
                split_ratio: f32::INFINITY,
            },
        };
        let mut state = AppState::new();
        preferences.apply_to_state(&mut state);
        assert_eq!(state.split_ratio, crate::state::DEFAULT_SPLIT_RATIO);

        state.split_ratio = 42.0;
        let preferences = AppPreferences::from_state(&state);
        assert_eq!(preferences.ui.split_ratio, MAX_SPLIT_RATIO);
    }
}
