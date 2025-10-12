use color_eyre::Result;
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::PathBuf;

#[allow(dead_code)]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    pub layout: LayoutConfig,
    pub theme: ThemeConfig,
}

#[allow(dead_code)]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LayoutConfig {
    pub mode: String,
    pub left_width_ratio: f32,
}

#[allow(dead_code)]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ThemeConfig {
    pub name: String,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            layout: LayoutConfig {
                mode: "normal".to_string(),
                left_width_ratio: 0.3,
            },
            theme: ThemeConfig {
                name: "catppuccin-mocha".to_string(),
            },
        }
    }
}

pub fn load_config() -> Result<Config> {
    let config_path = get_config_path()?;
    
    if !config_path.exists() {
        return Ok(Config::default());
    }

    let content = fs::read_to_string(config_path)?;
    let config: Config = toml::from_str(&content)?;
    Ok(config)
}

fn get_config_path() -> Result<PathBuf> {
    let home = std::env::var("HOME")?;
    Ok(PathBuf::from(home).join(".config/dumbgram/config.toml"))
}
