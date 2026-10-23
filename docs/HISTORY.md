# Versioni

## 0.5.1

- Distribuzione limitata a macOS Apple Silicon.
- Controllo Metal all’avvio; in caso di errore l’app mostra un messaggio ed esce.

## 0.5.0

- Finestra compatta e slider GPU con incrementi del 5%.
- Icona condivisa e barra superiore trascinabile.
- Registro attività persistente con conservazione configurabile.

## 0.4.1

- Ridotti i testi dell'interfaccia.

## 0.4.0

- Interfaccia egui con avvio, arresto e riavvio alla modifica delle impostazioni.
- Grafico hashrate e temi chiaro, scuro e sistema.

## 0.3.2

- Difficoltà intere serializzate come interi JSON: SoloPool ignora valori come `10.0` ma accetta `10`.

## 0.3.1

- Su SoloPool, richiesta di difficoltà dopo l'autorizzazione e aggiornamento del job corrente quando il pool non ne invia uno nuovo.
- Riepilogo notifiche aggiornato al salvataggio.

## 0.3.0

- Opzioni per wallet, pool, difficoltà e duty cycle.
- Notifiche per share, candidati blocco ed errori.

## 0.2.1

- Richiesta di difficoltà iniziale 1 per Public Pool. Il worker compare nella dashboard dopo la prima share valida.

## 0.2.0

- Mining SHA-256d su Metal, verifica CPU e client Stratum V1.

## 0.1.0

- Menu nella barra di sistema e configurazione dei pool.
