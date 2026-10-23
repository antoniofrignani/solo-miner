# Pools

| Pool | Stratum TCP endpoint | Documentation |
| --- | --- | --- |
| CKPool Solo | `stratum.ckpool.org:3333` | https://solo.ckpool.org/ |
| Braiins Solo | `solo.stratum.braiins.com:3333` | https://academy.braiins.com/braiins-pool/solo-mining |
| Public Pool | `public-pool.io:3333` | https://web.public-pool.io/ |
| SoloPool | `stratum.solopool.com:3333` | https://solopool.com/docs.html |

The client uses `ADDRESS.mac` as the username and `x` as the password. On SoloPool, `x` selects 100% solo mode.

`mining.suggest_difficulty` is sent before authorization, except on SoloPool, which requires it afterward. The client applies the difficulty assigned by the server. SoloPool requires integer values to be JSON integers; an update can affect the current job without a new `mining.notify`.

Public Pool registers a worker after its first valid share. Average time per share is `difficulty × 2^32 / hashrate`.

## Wallet statistics

- CKPool: `https://stats.ckpool.org/users/ADDRESS`
- Braiins: `https://hashpower.braiins.com/solo/ADDRESS`
- SoloPool: `https://solopool.com/user.html?addr=ADDRESS`
- Public Pool: `https://web.public-pool.io/#/app/ADDRESS`

Statistics pages use the validated mainnet address without a worker suffix. Endpoints and URLs verified on October 23, 2026.
