# DbotX Trade Demo

This repository contains a minimal Rust example showing how to interact with the DbotX Data and Trade APIs.

The program searches for an active pool by Solana token mint (CA) and submits a small IOC limit buy order using an API key.

## Usage

Set your API key via `DBOT_API_KEY` and run:

```bash
export DBOT_API_KEY=your_key
cargo run -- <MINT>
```

Example:

```bash
export DBOT_API_KEY=ckydkvw5urnw3shhjmz9wvuqmoqt36l2
cargo run -- QMaZd9LkqQexX2jz2LEir6N18PSYHm4pRjVTq5abonk
```

If `DBOT_API_KEY` is not set, a temporary public key is used for quick testing, but you should supply your own credentials in practice and implement proper authentication/signing.
