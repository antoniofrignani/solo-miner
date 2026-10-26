use crate::pool::{MiningMode, Pool};
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
    // Kept outside config.toml so an older app can still read settings after rollback.
    #[serde(skip, default = "default_solo_percent")]
    pub solo_percent: u8,
    #[serde(skip)]
    pub mining_mode: MiningMode,
    #[serde(skip)]
    pub braiins_account: String,
    #[serde(skip)]
    pub f2pool_account: String,
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
            solo_percent: default_solo_percent(),
            mining_mode: MiningMode::Solo,
            braiins_account: String::new(),
            f2pool_account: String::new(),
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
        let mut config: Self = toml::from_str(&text).context("Invalid TOML settings")?;
        let options = load_pool_options(path)?;
        if let Some(pool) = options.pool {
            config.pool = pool;
        }
        config.solo_percent = options.solo_percent;
        config.mining_mode = options.mode;
        config.braiins_account = options.braiins_account;
        config.f2pool_account = options.f2pool_account;
        ensure!(
            config.version == 1,
            "Unsupported settings version: {}",
            config.version
        );
        config.validate_inner(true)?;
        Ok(config)
    }

    pub fn validate(&self) -> Result<()> {
        self.validate_inner(false)
    }

    fn validate_inner(&self, allow_disabled_pool: bool) -> Result<()> {
        ensure!(
            [0, 7, 30].contains(&self.log_retention_days),
            "Invalid log retention"
        );
        ensure!((1..=100).contains(&self.gpu_load), "Invalid GPU load");
        ensure!(
            self.solo_percent <= 100,
            "Solo percentage must be between 0 and 100"
        );
        ensure!(
            self.pool.supports(self.mining_mode)
                || (allow_disabled_pool
                    && self.mining_mode == MiningMode::Pool
                    && self.pool.needs_account()),
            "Pool unavailable for the selected mining mode"
        );
        if self.pool == Pool::Solopool && self.mining_mode == MiningMode::Pool {
            ensure!(
                self.solo_percent < 100,
                "Pool mode requires less than 100% solo"
            );
        }
        for account in [&self.braiins_account, &self.f2pool_account] {
            ensure!(
                account.len() <= 64
                    && account
                        .bytes()
                        .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-'),
                "Pool account must contain only letters, numbers, '-' or '_'"
            );
        }
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
        let mode_path = path.with_file_name("pool-options.json");
        let mut mode = tempfile::NamedTempFile::new_in(parent)?;
        mode.write_all(&serde_json::to_vec(&PoolOptions {
            pool: Some(self.pool),
            mode: self.mining_mode,
            solo_percent: self.solo_percent,
            braiins_account: self.braiins_account.clone(),
            f2pool_account: self.f2pool_account.clone(),
        })?)?;
        mode.as_file().sync_all()?;
        mode.persist(&mode_path)
            .context("Unable to save pool options")?;
        let mut file = tempfile::NamedTempFile::new_in(parent)?;
        let mut legacy = self.clone();
        if !matches!(
            legacy.pool,
            Pool::Ckpool | Pool::Braiins | Pool::Solopool | Pool::Public
        ) {
            legacy.pool = Pool::Ckpool;
        }
        file.write_all(toml::to_string_pretty(&legacy)?.as_bytes())?;
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
                mining_mode: if pool.supports(MiningMode::Solo) {
                    MiningMode::Solo
                } else {
                    MiningMode::Pool
                },
                solo_percent: if pool == Pool::Solopool { 0 } else { 100 },
                ..Config::default()
            }
            .save(&path)?;
            assert_eq!(Config::load(&path)?.pool, pool);
        }
        Ok(())
    }
    #[test]
    fn disabled_account_pools_load_but_cannot_start_or_save() -> Result<()> {
        let dir = tempfile::tempdir()?;
        let path = dir.path().join("config.toml");
        let config = Config {
            pool: Pool::Ocean,
            mining_mode: MiningMode::Pool,
            bitcoin_address: "1A1zP1eP5QGefi2DMPTfTL5SLmv7DivfNa".into(),
            ..Config::default()
        };
        config.save(&path)?;
        let old: Config = toml::from_str(&fs::read_to_string(&path)?)?;
        assert_eq!(old.pool, Pool::Ckpool);
        assert_eq!(old.bitcoin_address, config.bitcoin_address);
        for pool in [Pool::BraiinsPool, Pool::F2pool] {
            let mut options: PoolOptions =
                serde_json::from_slice(&fs::read(path.with_file_name("pool-options.json"))?)?;
            options.pool = Some(pool);
            options.braiins_account = "worker_account".into();
            options.f2pool_account = "worker_account".into();
            fs::write(
                path.with_file_name("pool-options.json"),
                serde_json::to_vec(&options)?,
            )?;
            let loaded = Config::load(&path)?;
            assert_eq!(loaded.pool, pool);
            assert_eq!(loaded.mining_username()?, "worker_account.mac");
            assert!(loaded.validate().is_err());
            assert!(loaded.save(&path).is_err());
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
fn default_solo_percent() -> u8 {
    100
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
struct PoolOptions {
    pool: Option<Pool>,
    mode: MiningMode,
    solo_percent: u8,
    braiins_account: String,
    f2pool_account: String,
}
impl Default for PoolOptions {
    fn default() -> Self {
        Self {
            pool: None,
            mode: MiningMode::Solo,
            solo_percent: 100,
            braiins_account: String::new(),
            f2pool_account: String::new(),
        }
    }
}
fn load_pool_options(path: &Path) -> Result<PoolOptions> {
    match fs::read(path.with_file_name("pool-options.json")) {
        Ok(bytes) => serde_json::from_slice(&bytes).context("Invalid pool options"),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(PoolOptions::default()),
        Err(error) => Err(error).context("Unable to read pool options"),
    }
}
impl Config {
    pub fn endpoint(&self) -> &'static str {
        self.pool.endpoint_for(self.mining_mode)
    }
    pub fn requires_address(&self) -> bool {
        !self.pool.needs_account()
    }
    pub fn mining_username(&self) -> Result<String> {
        if self.requires_address() {
            validate_address(&self.bitcoin_address)?;
            Ok(format!("{}.mac", self.bitcoin_address))
        } else {
            let account = match self.pool {
                Pool::BraiinsPool => &self.braiins_account,
                Pool::F2pool => &self.f2pool_account,
                _ => unreachable!(),
            };
            ensure!(
                !account.is_empty(),
                "Enter the {} account name",
                self.pool.name()
            );
            Ok(format!("{account}.mac"))
        }
    }
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
    #[test]
    fn solopool_mix_round_trips_without_changing_legacy_toml() -> Result<()> {
        let dir = tempfile::tempdir()?;
        let path = dir.path().join("config.toml");
        fs::write(&path, "version=1\npool='solopool'\nshare_difficulty=5")?;
        let mut config = Config::load(&path)?;
        assert_eq!(config.solo_percent, 100);
        config.mining_mode = MiningMode::Pool;
        for percent in [0, 37] {
            config.solo_percent = percent;
            config.save(&path)?;
            assert_eq!(Config::load(&path)?.solo_percent, percent);
            assert!(!fs::read_to_string(&path)?.contains("solo_percent"));
        }
        config.mining_mode = MiningMode::Solo;
        config.solo_percent = 100;
        config.save(&path)?;
        let mut changed = config.clone();
        changed.solo_percent = 37;
        assert!(config.mining_changed(&changed));
        changed.pool = Pool::Public;
        config.pool = Pool::Public;
        assert!(!config.mining_changed(&changed));
        config.solo_percent = 101;
        assert!(config.save(&path).is_err());
        assert_eq!(Config::load(&path)?.solo_percent, 100);
        fs::write(
            path.with_file_name("pool-options.json"),
            r#"{"solo_percent":101}"#,
        )?;
        assert!(Config::load(&path).is_err());
        Ok(())
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
            || self.mining_mode != other.mining_mode
            || (self.pool == Pool::Solopool && self.solo_percent != other.solo_percent)
            || (self.pool == Pool::BraiinsPool && self.braiins_account != other.braiins_account)
            || (self.pool == Pool::F2pool && self.f2pool_account != other.f2pool_account)
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
