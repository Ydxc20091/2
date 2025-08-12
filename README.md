# DbotX Trade Demo

This repository contains a minimal Rust example showing how to interact with the DbotX Data and Trade APIs.

The program searches for an active pool by Solana token mint (CA) and submits a small IOC limit buy order using an API key.

## Usage

```bash
cargo run -- <MINT>
```

Example:

```bash
cargo run -- QMaZd9LkqQexX2jz2LEir6N18PSYHm4pRjVTq5abonk
```

The code uses a temporary test key embedded in the source (`ckydkvw5urnw3shhjmz9wvuqmoqt36l2`). In real deployments, read credentials from environment variables and implement proper authentication/signing.
