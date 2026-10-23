use crate::pool::Pool;
use anyhow::{Context, Result, ensure};
use directories::ProjectDirs;
use serde::{Deserialize, Serialize};
use std::{
    fs,
    io::Write,
    path::{Path, PathBuf},
};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Config {
    pub version: u32,
    #[serde(default = "default_retention")]
    pub log_retention_days: u16,
    pub pool: Pool,
    #[serde(default)]
    pub bitcoin_address: String,
    #[serde(default = "default_gpu_load")]
    pub gpu_load: u8,
    #[serde(default)]
    pub gpu_profile: GpuProfile,
    #[serde(default = "default_share_difficulty")]
    pub share_difficulty: f64,
    #[serde(default)]
    pub notifications: NotificationSettings,
    #[serde(default)]
    pub appearance: Appearance,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            version: 1,
            log_retention_days: default_retention(),
            pool: Pool::default(),
            bitcoin_address: String::new(),
            gpu_load: default_gpu_load(),
            gpu_profile: GpuProfile::default(),
            share_difficulty: default_share_difficulty(),
            notifications: NotificationSettings::default(),
            appearance: Appearance::default(),
        }
    }
}

impl Config {
    pub fn load(path: &Path) -> Result<Self> {
        let text = match fs::read_to_string(path) {
            Ok(text) => text,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                return Ok(Self::default());
            }
            Err(error) => return Err(error).context("Unable to read settings"),
        };
        let config: Self = toml::from_str(&text).context("Invalid TOML settings")?;
        ensure!(
            config.version == 1,
            "Unsupported settings version: {}",
            config.version
        );
        config.validate()?;
        Ok(config)
    }

    pub fn validate(&self) -> Result<()> {
        ensure!(
            [0, 7, 30].contains(&self.log_retention_days),
            "Invalid log retention"
        );
        ensure!((1..=100).contains(&self.gpu_load), "Invalid GPU load");
        ensure!(
            self.share_difficulty.is_finite() && (0.000001..=1e12).contains(&self.share_difficulty),
            "Share difficulty must be between 0.000001 and 1000000000000"
        );
        if self.pool == Pool::Solopool {
            ensure!(
                self.share_difficulty >= 1.0 && self.share_difficulty.fract() == 0.0,
                "SoloPool requires an integer share difficulty of at least 1"
            );
        }
        if !self.bitcoin_address.is_empty() {
            validate_address(&self.bitcoin_address)?;
        }
        Ok(())
    }

    pub fn save(&self, path: &Path) -> Result<()> {
        self.validate()?;
        let parent = path
            .parent()
            .context("Settings path has no parent directory")?;
        fs::create_dir_all(parent)?;
        let mut file = tempfile::NamedTempFile::new_in(parent)?;
        file.write_all(toml::to_string_pretty(self)?.as_bytes())?;
        file.as_file().sync_all()?;
        file.persist(path).context("Unable to save settings")?;
        Ok(())
    }
}

pub fn config_path() -> Result<PathBuf> {
    Ok(ProjectDirs::from("it", "solo-miner", "Solo Miner")
        .context("User directory unavailable")?
        .config_dir()
        .join("config.toml"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn selection_survives_atomic_replacement() -> Result<()> {
        let dir = tempfile::tempdir()?;
        let path = dir.path().join("nested/config.toml");
        assert_eq!(Config::load(&path)?.pool, Pool::Ckpool);
        for pool in Pool::ALL {
            Config {
                pool,
                ..Config::default()
            }
            .save(&path)?;
            assert_eq!(Config::load(&path)?.pool, pool);
        }
        Ok(())
    }

    #[test]
    fn invalid_config_is_rejected_and_preserved() -> Result<()> {
        let dir = tempfile::tempdir()?;
        let path = dir.path().join("config.toml");
        for input in [
            "broken [",
            "version = 9\npool = 'ckpool'",
            "version = 1\npool = 'unknown'",
        ] {
            fs::write(&path, input)?;
            assert!(Config::load(&path).is_err());
            assert_eq!(fs::read_to_string(&path)?, input);
        }
        Ok(())
    }
}

fn default_retention() -> u16 {
    30
}

fn default_gpu_load() -> u8 {
    25
}

pub fn validate_address(value: &str) -> Result<()> {
    use std::str::FromStr;
    bitcoin::Address::from_str(value)
        .context("Invalid Bitcoin address (checksum/format)")?
        .require_network(bitcoin::Network::Bitcoin)
        .context("A Bitcoin mainnet address is required")?;
    Ok(())
}

#[cfg(test)]
mod preference_tests {
    use super::*;
    #[test]
    fn validates_mainnet_checksum_and_network() {
        assert!(validate_address("1A1zP1eP5QGefi2DMPTfTL5SLmv7DivfNa").is_ok());
        assert!(validate_address("1A1zP1eP5QGefi2DMPTfTL5SLmv7DivfNb").is_err());
        assert!(validate_address("mipcBbFg9gMiCh81Kj8tqqdgoZub1ZJRfn").is_err());
        assert!(validate_address("").is_err());
    }
    #[test]
    fn old_config_migrates_and_preferences_survive() -> Result<()> {
        let dir = tempfile::tempdir()?;
        let path = dir.path().join("config.toml");
        fs::write(&path, "version = 1\npool = 'ckpool'")?;
        let mut config = Config::load(&path)?;
        assert_eq!(config.gpu_load, 25);
        assert!(config.bitcoin_address.is_empty());
        config.pool = Pool::Public;
        config.bitcoin_address = "1A1zP1eP5QGefi2DMPTfTL5SLmv7DivfNa".into();
        config.gpu_load = 37;
        config.share_difficulty = 2.5;
        config.notifications.accepted_share = true;
        config.notifications.error = true;
        config.save(&path)?;
        let loaded = Config::load(&path)?;
        assert_eq!(loaded.bitcoin_address, config.bitcoin_address);
        assert_eq!(loaded.gpu_load, 37);
        assert_eq!(loaded.share_difficulty, 2.5);
        assert!(loaded.notifications.accepted_share && loaded.notifications.error);
        assert!(!loaded.notifications.block_candidate);
        assert_eq!(loaded.pool, Pool::Public);
        let before = fs::read_to_string(&path)?;
        config.gpu_load = 0;
        assert!(config.save(&path).is_err());
        assert_eq!(fs::read_to_string(&path)?, before);
        Ok(())
    }
}

fn default_share_difficulty() -> f64 {
    1.0
}
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct NotificationSettings {
    pub accepted_share: bool,
    pub block_candidate: bool,
    pub error: bool,
}
impl NotificationSettings {
    pub fn enabled(&self) -> bool {
        self.accepted_share || self.block_candidate || self.error
    }
}

#[cfg(test)]
mod option_tests {
    use super::*;
    #[test]
    fn invalid_difficulty_and_slider_are_rejected() {
        for diff in [0.0, -1.0, f64::NAN, f64::INFINITY, 1e13] {
            assert!(
                Config {
                    share_difficulty: diff,
                    ..Config::default()
                }
                .validate()
                .is_err()
            );
        }
        for duty in [1, 37, 99, 100] {
            assert!(
                Config {
                    gpu_load: duty,
                    ..Config::default()
                }
                .validate()
                .is_ok()
            );
        }
        assert!(
            Config {
                gpu_load: 101,
                ..Config::default()
            }
            .validate()
            .is_err()
        );
        let old: Config = toml::from_str("version=1\npool='public_pool'\ngpu_load=75").unwrap();
        assert_eq!(old.share_difficulty, 1.0);
        assert!(!old.notifications.enabled());
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ThemeMode {
    Light,
    Dark,
    #[default]
    System,
}
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Palette {
    #[default]
    Graphite,
    Ocean,
    Forest,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Appearance {
    pub mode: ThemeMode,
    pub palette: Palette,
    pub accent: [u8; 3],
}
impl Default for Appearance {
    fn default() -> Self {
        Self {
            mode: ThemeMode::System,
            palette: Palette::Graphite,
            accent: [245, 158, 66],
        }
    }
}
impl Config {
    pub fn mining_changed(&self, other: &Self) -> bool {
        self.pool != other.pool
            || self.bitcoin_address != other.bitcoin_address
            || self.gpu_load != other.gpu_load
            || self.gpu_profile != other.gpu_profile
            || self.share_difficulty != other.share_difficulty
    }
}

#[cfg(test)]
mod appearance_tests {
    use super::*;
    #[test]
    fn old_config_keeps_wallet_and_new_appearance_round_trips() -> Result<()> {
        let old = "version=1\npool='solopool'\nbitcoin_address='1A1zP1eP5QGefi2DMPTfTL5SLmv7DivfNa'\ngpu_load=75\nshare_difficulty=10";
        let dir = tempfile::tempdir()?;
        let path = dir.path().join("config.toml");
        fs::write(&path, old)?;
        let mut config = Config::load(&path)?;
        let previous = config.clone();
        assert_eq!(config.appearance, Appearance::default());
        config.appearance = Appearance {
            mode: ThemeMode::Dark,
            palette: Palette::Ocean,
            accent: [64, 192, 220],
        };
        assert!(!previous.mining_changed(&config));
        config.save(&path)?;
        assert_eq!(Config::load(&path)?, config);
        assert_eq!(config.bitcoin_address, previous.bitcoin_address);
        config.gpu_load = 37;
        assert!(previous.mining_changed(&config));
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GpuProfile {
    Responsive,
    #[default]
    Balanced,
    Throughput,
}
impl GpuProfile {
    pub const ALL: [Self; 3] = [Self::Responsive, Self::Balanced, Self::Throughput];
    pub fn label(self) -> &'static str {
        match self {
            Self::Responsive => "Responsive",
            Self::Balanced => "Balanced",
            Self::Throughput => "Throughput",
        }
    }
    pub fn batch_size(self) -> u32 {
        match self {
            Self::Responsive => 65_536,
            Self::Balanced => 262_144,
            Self::Throughput => 1_048_576,
        }
    }
}

#[cfg(test)]
mod gpu_profile_tests {
    use super::*;
    #[test]
    fn workload_migrates_and_restarts_only_when_changed() -> Result<()> {
        let old: Config = toml::from_str("version=1\npool='public_pool'\ngpu_load=75")?;
        assert_eq!(old.gpu_profile, GpuProfile::Balanced);
        let dir = tempfile::tempdir()?;
        let path = dir.path().join("config.toml");
        for profile in GpuProfile::ALL {
            let mut next = old.clone();
            next.gpu_profile = profile;
            next.save(&path)?;
            assert_eq!(Config::load(&path)?, next);
            assert_eq!(old.mining_changed(&next), profile != old.gpu_profile);
            assert_eq!(next.gpu_load, 75);
        }
        assert!(
            toml::from_str::<Config>("version=1\npool='public_pool'\ngpu_profile='unknown'")
                .is_err()
        );
        Ok(())
    }
}
