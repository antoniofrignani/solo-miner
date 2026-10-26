# Pools

Select **Solo** or **Pool**, then choose a provider. SoloPool is available in both modes. Its Pool mode has a 0–99% solo split; `0` is fully shared. The app sends the selected percentage as the Stratum password. Solo mode sends `100`.

| Provider | Solo | Pool | Stratum TCP endpoint | Identity |
| --- | --- | --- | --- | --- |
| CKPool Solo | Yes | — | `stratum.ckpool.org:3333` | Bitcoin address |
| Braiins Solo | Yes | — | `solo.stratum.braiins.com:3333` | Bitcoin address |
| Public Pool | Yes | Yes | `public-pool.io:3333` (Solo), `:13333` (PPLNS) | Bitcoin address |
| SoloPool | Yes | Yes | `stratum.solopool.com:3333` | Bitcoin address |
| OCEAN | — | Yes | `mine.ocean.xyz:3334` | Bitcoin address |
| OnPool | Yes | Yes | `btc.onpool.net:3333` (Solo), `:3344` (PPLNS) | Bitcoin address |

The worker suffix is `.mac`. Every available provider uses the saved mainnet reward address in the Stratum username. Braiins Pool and f2pool require registered accounts and are disabled. Previously saved account names remain in `pool-options.json`; a saved selection loads, but mining cannot start until another provider is selected.

The app requests a share difficulty from Public Pool and OnPool before authorization, and from SoloPool afterward. Other providers assign it themselves. A requested value is not a guarantee: the server's `mining.set_difficulty` determines the target. SoloPool requires an integer request. Public Pool may show a worker only after its first accepted share.

SoloPool's split applies per address across workers. Its 2% fee is deducted from block rewards in every mode. An increase in solo percentage can lock decreases for 48 hours; another worker with a higher solo percentage can override the requested ratio. Check the provider's statistics for the effective ratio. Shared rewards depend on a shared-route block, contributed work, provider rules and payout thresholds. An accepted share is not a payout.

`pool-options.json` beside `config.toml` stores the mode, SoloPool split and provider. It also preserves account names from previous builds. New providers serialize as CKPool in the legacy TOML file; a previous app version can still load that file after updater rollback. New versions read the selected provider from `pool-options.json`.

## Provider documentation

- [CKPool Solo](https://solo.ckpool.org/)
- [Braiins Solo](https://academy.braiins.com/braiins-pool/solo-mining)
- [Public Pool](https://web.public-pool.io/)
- [SoloPool modes and split](https://solopool.com/docs.html)
- [OCEAN setup](https://ocean.xyz/getstarted)
- [OnPool ports](https://onpool.net/connect)

## Verification

On October 26, 2026, the app's Stratum parser received authorization and valid jobs from every available provider and mode: CKPool Solo, Braiins Solo, Public Pool Solo and PPLNS, SoloPool Solo and Pool, OCEAN, and OnPool Solo and PPLNS. SoloPool Pool used an isolated test address so the saved wallet's ratio was unchanged. Public Pool accepted a real GPU share in each mode using the saved wallet. Local fixtures cover authorization, difficulty messages and share submission for every available provider; a Metal GPU submitted a share to the local Stratum fixture. No live shares were submitted to the other providers, and no block payout was verified.
