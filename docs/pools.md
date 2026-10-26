# Pools

Select **Solo** or **Pool**, then choose a provider. SoloPool is available in both modes. Its Pool mode has a 0–99% solo split; `0` is fully shared. The app sends the selected percentage as the Stratum password. Solo mode sends `100`.

| Provider | Solo | Pool | Stratum TCP endpoint | Identity |
| --- | --- | --- | --- | --- |
| CKPool Solo | Yes | — | `stratum.ckpool.org:3333` | Bitcoin address |
| Braiins Solo | Yes | — | `solo.stratum.braiins.com:3333` | Bitcoin address |
| Public Pool | Yes | — | `public-pool.io:3333` | Bitcoin address |
| SoloPool | Yes | Yes | `stratum.solopool.com:3333` | Bitcoin address |
| OCEAN | — | Yes | `mine.ocean.xyz:3334` | Bitcoin address |
| OnPool | Yes | Yes | `btc.onpool.net:3333` (Solo), `:3344` (PPLNS) | Bitcoin address |
| Braiins Pool | — | Yes | `stratum.braiins.com:3333` | Braiins account name |
| f2pool | — | Yes | `btc.f2pool.com:3333` | f2pool account name |

The worker suffix is `.mac`. Address-based providers use the saved mainnet reward address in the Stratum username. Braiins Pool and f2pool use `<account>.mac`; configure the payout address in the provider account. The saved reward address is not sent to those two pools. Neither account password nor API credentials are stored by the app; the Stratum password is `x`.

The app requests a share difficulty from Public Pool, OnPool and Braiins Pool before authorization, and from SoloPool afterward. Other providers assign it themselves. A requested value is not a guarantee: the server's `mining.set_difficulty` determines the target. SoloPool requires an integer request. Public Pool may show a worker only after its first accepted share.

SoloPool's split applies per address across workers. Its 2% fee is deducted from block rewards in every mode. An increase in solo percentage can lock decreases for 48 hours; another worker with a higher solo percentage can override the requested ratio. Check the provider's statistics for the effective ratio. Shared rewards depend on a shared-route block, contributed work, provider rules and payout thresholds. An accepted share is not a payout.

`pool-options.json` beside `config.toml` stores the mode, SoloPool split and pool account names. New providers serialize as CKPool in the legacy TOML file; a previous app version can still load that file after updater rollback. New versions read the selected provider from `pool-options.json`.

## Provider documentation

- [CKPool Solo](https://solo.ckpool.org/)
- [Braiins Solo](https://academy.braiins.com/braiins-pool/solo-mining)
- [Public Pool](https://web.public-pool.io/)
- [SoloPool modes and split](https://solopool.com/docs.html)
- [OCEAN setup](https://ocean.xyz/getstarted)
- [OnPool ports](https://onpool.net/connect)
- [Braiins Pool setup](https://academy.braiins.com/braiins-pool/btc-mining-setup)
- [f2pool setup](https://f2pool.zendesk.com/hc/en-us/articles/22892423030809-How-to-mine-Bitcoin)

## Verification

On October 26, 2026, the app's Stratum parser received authorization and valid jobs from CKPool Solo, Braiins Solo, Public Pool, SoloPool, OCEAN, OnPool Solo and OnPool PPLNS. Braiins Pool and f2pool accepted TCP subscription and returned jobs. Braiins Pool rejected an unregistered test account; f2pool returned authorization before account ownership could be confirmed. Registered accounts were unavailable, so payout attribution and accepted real shares were not verified on those providers. No live shares were submitted during these checks. Local fixtures cover authorization, difficulty messages and share submission for every configured provider.
