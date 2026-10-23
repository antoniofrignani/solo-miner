use serde::{Deserialize, Serialize};

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
}

impl Pool {
    pub const ALL: [Self; 4] = [Self::Ckpool, Self::Braiins, Self::Solopool, Self::Public];

    pub fn name(self) -> &'static str {
        match self {
            Self::Ckpool => "CKPool Solo",
            Self::Braiins => "Braiins Solo",
            Self::Solopool => "SoloPool",
            Self::Public => "Public Pool",
        }
    }

    pub fn endpoint(self) -> &'static str {
        match self {
            Self::Ckpool => "stratum+tcp://stratum.ckpool.org:3333",
            Self::Braiins => "stratum+tcp://solo.stratum.braiins.com:3333",
            Self::Solopool => "stratum+tcp://stratum.solopool.com:3333",
            Self::Public => "stratum+tcp://public-pool.io:3333",
        }
    }

    pub fn account_url(self, address: &str) -> anyhow::Result<String> {
        crate::config::validate_address(address)?;
        Ok(match self {
            Self::Ckpool => format!("https://stats.ckpool.org/users/{address}"),
            Self::Braiins => format!("https://hashpower.braiins.com/solo/{address}"),
            Self::Solopool => format!("https://solopool.com/user.html?addr={address}"),
            Self::Public => format!("https://web.public-pool.io/#/app/{address}"),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn account_urls_include_validated_wallet() {
        let wallet = "1A1zP1eP5QGefi2DMPTfTL5SLmv7DivfNa";
        for pool in Pool::ALL {
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
    }
}
