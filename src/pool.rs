use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MiningMode {
    #[default]
    Solo,
    Pool,
}

/// Stratum endpoints listed in docs/pools.md.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Pool {
    #[default]
    Ckpool,
    Braiins,
    Solopool,
    #[serde(rename = "public_pool")]
    Public,
    Ocean,
    Onpool,
    BraiinsPool,
    F2pool,
}

impl Pool {
    pub const ALL: [Self; 6] = [
        Self::Ckpool,
        Self::Braiins,
        Self::Solopool,
        Self::Public,
        Self::Ocean,
        Self::Onpool,
    ];
    pub const SOLO: [Self; 5] = [
        Self::Ckpool,
        Self::Braiins,
        Self::Solopool,
        Self::Public,
        Self::Onpool,
    ];
    pub const SHARED: [Self; 4] = [Self::Solopool, Self::Public, Self::Ocean, Self::Onpool];

    pub fn supports(self, mode: MiningMode) -> bool {
        match mode {
            MiningMode::Solo => Self::SOLO.contains(&self),
            MiningMode::Pool => Self::SHARED.contains(&self),
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Self::Ckpool => "CKPool Solo",
            Self::Braiins => "Braiins Solo",
            Self::Solopool => "SoloPool",
            Self::Public => "Public Pool",
            Self::Ocean => "OCEAN",
            Self::Onpool => "OnPool",
            Self::BraiinsPool => "Braiins Pool",
            Self::F2pool => "f2pool",
        }
    }

    pub fn endpoint(self) -> &'static str {
        match self {
            Self::Ckpool => "stratum+tcp://stratum.ckpool.org:3333",
            Self::Braiins => "stratum+tcp://solo.stratum.braiins.com:3333",
            Self::Solopool => "stratum+tcp://stratum.solopool.com:3333",
            Self::Public => "stratum+tcp://public-pool.io:3333",
            Self::Ocean => "stratum+tcp://mine.ocean.xyz:3334",
            Self::Onpool => "stratum+tcp://btc.onpool.net:3344",
            Self::BraiinsPool => "stratum+tcp://stratum.braiins.com:3333",
            Self::F2pool => "stratum+tcp://btc.f2pool.com:3333",
        }
    }
    pub fn endpoint_for(self, mode: MiningMode) -> &'static str {
        match (self, mode) {
            (Self::Onpool, MiningMode::Solo) => "stratum+tcp://btc.onpool.net:3333",
            (Self::Public, MiningMode::Pool) => "stratum+tcp://public-pool.io:13333",
            _ => self.endpoint(),
        }
    }

    pub fn account_url(self, address: &str) -> anyhow::Result<String> {
        if !self.needs_account() {
            crate::config::validate_address(address)?;
        }
        Ok(match self {
            Self::Ckpool => format!("https://stats.ckpool.org/users/{address}"),
            Self::Braiins => format!("https://hashpower.braiins.com/solo/{address}"),
            Self::Solopool => format!("https://solopool.com/user.html?addr={address}"),
            Self::Public => format!("https://web.public-pool.io/#/app/{address}"),
            Self::Ocean => format!("https://ocean.xyz/stats/{address}"),
            Self::Onpool => "https://onpool.net/dashboard".into(),
            Self::BraiinsPool => "https://pool.braiins.com/".into(),
            Self::F2pool => "https://www.f2pool.com/".into(),
        })
    }
    pub fn needs_account(self) -> bool {
        matches!(self, Self::BraiinsPool | Self::F2pool)
    }
    pub fn suggests_difficulty_before_auth(self) -> bool {
        matches!(self, Self::Public | Self::Onpool | Self::BraiinsPool)
    }
    pub fn suggests_difficulty_after_auth(self) -> bool {
        self == Self::Solopool
    }
    pub fn accepts_difficulty_suggestion(self) -> bool {
        self.suggests_difficulty_before_auth() || self.suggests_difficulty_after_auth()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn account_urls_include_validated_wallet() {
        let wallet = "1A1zP1eP5QGefi2DMPTfTL5SLmv7DivfNa";
        for pool in [
            Pool::Ckpool,
            Pool::Braiins,
            Pool::Solopool,
            Pool::Public,
            Pool::Ocean,
        ] {
            assert!(pool.account_url(wallet).unwrap().ends_with(wallet));
            assert!(pool.account_url("").is_err());
            assert!(pool.account_url("../../bad?x=1").is_err());
        }
        assert!(
            Pool::Public
                .account_url(wallet)
                .unwrap()
                .contains("/#/app/")
        );
        assert!(
            Pool::Solopool
                .account_url(wallet)
                .unwrap()
                .contains("user.html?addr=")
        );
        assert_eq!(
            Pool::Onpool.endpoint_for(MiningMode::Solo),
            "stratum+tcp://btc.onpool.net:3333"
        );
        assert_eq!(
            Pool::Onpool.endpoint_for(MiningMode::Pool),
            "stratum+tcp://btc.onpool.net:3344"
        );
        assert_eq!(
            Pool::Public.endpoint_for(MiningMode::Pool),
            "stratum+tcp://public-pool.io:13333"
        );
        assert_eq!(
            Pool::Onpool.account_url(wallet).unwrap(),
            "https://onpool.net/dashboard"
        );
        assert!(
            Pool::BraiinsPool
                .account_url("")
                .unwrap()
                .contains("braiins.com")
        );
        assert!(Pool::F2pool.account_url("").unwrap().contains("f2pool.com"));
    }
}
