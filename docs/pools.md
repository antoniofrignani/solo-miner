# Pool

| Pool | Endpoint Stratum TCP | Documentazione |
| --- | --- | --- |
| CKPool Solo | `stratum.ckpool.org:3333` | https://solo.ckpool.org/ |
| Braiins Solo | `solo.stratum.braiins.com:3333` | https://academy.braiins.com/braiins-pool/solo-mining |
| Public Pool | `public-pool.io:3333` | https://web.public-pool.io/ |
| SoloPool | `stratum.solopool.com:3333` | https://solopool.com/docs.html |

Il client usa `ADDRESS.mac` come username e `x` come password. Su SoloPool `x` seleziona la modalità 100% solo.

`mining.suggest_difficulty` viene inviato prima dell'autorizzazione, tranne su SoloPool che richiede l'invio dopo. Il client applica la difficoltà assegnata dal server. Su SoloPool i valori interi devono essere numeri JSON interi; un aggiornamento può riguardare il job corrente senza un nuovo `mining.notify`.

Public Pool registra il worker dopo la prima share valida. Il tempo medio per share è `difficoltà × 2^32 / hashrate`.

## Statistiche del wallet

- CKPool: `https://stats.ckpool.org/users/ADDRESS`
- Braiins: `https://hashpower.braiins.com/solo/ADDRESS`
- SoloPool: `https://solopool.com/user.html?addr=ADDRESS`
- Public Pool: `https://web.public-pool.io/#/app/ADDRESS`

Le pagine statistiche usano l'indirizzo mainnet validato, senza suffisso worker. Endpoint e URL verificati il 23 ottobre 2026.
