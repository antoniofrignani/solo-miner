# Solo Miner

Bitcoin solo miner per macOS, scritto in Rust. Hashing SHA-256d su GPU Metal, interfaccia egui e menu nella barra di sistema. Licenza MIT.

## Piattaforme

È distribuita solo la build per **macOS Apple Silicon (arm64)**, firmata ad hoc e senza notarizzazione Apple.

All'avvio l'app verifica GPU, shader e calcolo Metal. Se il controllo fallisce, mostra un messaggio con l'errore ed esce prima di aprire l'interfaccia o collegarsi al pool.

## Uso

Apri **Solo Miner.app**, configura indirizzo Bitcoin mainnet, pool, difficoltà share e duty cycle GPU, poi salva e avvia il mining.

- **Panoramica**: hashrate, share, durata, grafico e dati del pool.
- **Mining e wallet**: configurazione, notifiche e benchmark di 3 secondi.
- **Aspetto**: modalità chiara/scura/sistema, tema e accento.
- **Attività**: registro su disco, conservazione di 7 o 30 giorni oppure senza scadenza.

I pool disponibili sono CKPool Solo, Braiins Solo, SoloPool e Public Pool. **Account pool** apre le statistiche del wallet configurato. Endpoint e note Stratum: [docs/pools.md](docs/pools.md).

Le modifiche a wallet, pool, difficoltà o duty cycle riavviano il mining dopo il salvataggio. Aspetto e notifiche si applicano senza riavvio. Chiudere la finestra lascia attivo il mining; per terminare l'app usa il menu nella barra di sistema o `⌘Q`.

Il duty cycle regola batch GPU e pause, con incrementi del 5% nell'interfaccia. Non è un limite di potenza o temperatura. La difficoltà share effettiva è assegnata dal pool. Un candidato blocco viene verificato sulla CPU prima dell'invio; accettazione e ricompensa dipendono dal pool e dalla rete.

## Build

Servono un Mac Apple Silicon, Rust stable e Xcode Command Line Tools.

```sh
cargo build --locked --release
sh scripts/bundle.sh
open 'dist/Solo Miner.app'
```

```sh
cargo run -- --check          # Percorso e valori della configurazione
cargo run -- --options        # Impostazioni (macOS)
cargo run -- --resume-mining  # Avvio del mining (macOS)
cargo run --release -- --gpu-check  # Test GPU e benchmark (macOS)
```

Le impostazioni sono in `config.toml` nella directory utente dell'app; `--check` ne stampa il percorso. Il registro `activity.jsonl` è nella stessa directory. Hashrate e contatori della sessione restano in memoria.

## Test

```sh
cargo fmt --check
cargo clippy --locked --all-targets -- -D warnings
cargo test --locked
```

I test ordinari non contattano pool esterni. Su macOS sono disponibili due test aggiuntivi:

```sh
cargo test --locked gpu_to_stratum_share -- --ignored --nocapture
cargo test --locked solopool_live_difficulty_handshake -- --ignored --nocapture
```

Il primo richiede una GPU Metal e usa un server Stratum locale. Il secondo verifica l'handshake con SoloPool usando la configurazione salvata, senza inviare share.

## GitHub Actions

Ogni push su `main` e ogni pull request eseguono formattazione, Clippy, test e build su macOS Apple Silicon. Gli archivi e i checksum SHA-256 sono scaricabili dagli artifact della [build](https://github.com/antoniofrignani/solo-miner/actions/workflows/ci.yml).

Un tag `vX.Y.Z` uguale alla versione in `Cargo.toml` pubblica una release con gli stessi archivi, dopo il completamento della build.

[Note delle versioni](docs/HISTORY.md).
