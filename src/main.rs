use anyhow::{anyhow, Result};
use reqwest::Client;
use serde::Deserialize;
use std::cmp::Ordering;

// Temporary public test key. Set DBOT_API_KEY in the environment for your own key.
const DEFAULT_API_KEY: &str = "ckydkvw5urnw3shhjmz9wvuqmoqt36l2";

#[derive(Debug, Deserialize)]
struct PoolInfo {
    #[serde(alias = "pair", alias = "pairId", alias = "id")]
    pair_id: String,
    #[serde(alias = "dex")]
    dex: Option<String>,
    #[serde(alias = "solReserve")]
    sol_reserve: Option<f64>,
    #[serde(alias = "tokenReserve")]
    token_reserve: Option<f64>,
    #[serde(alias = "tokenPrice")]
    token_price: Option<f64>,
}

#[derive(Debug, Deserialize)]
struct SearchResp {
    #[serde(alias = "res", alias = "data")]
    pools: Vec<PoolInfo>,
    err: Option<bool>,
}

#[derive(Debug, Deserialize)]
struct OrderResp {
    #[serde(alias = "orderId")]
    order_id: String,
    status: String,
}

#[tokio::main]
async fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().collect();
    if args.len() < 2 {
        eprintln!("usage: {} <mint>", args[0]);
        std::process::exit(1);
    }
    let mint = &args[1];

    let client = Client::new();
    let api_key = std::env::var("DBOT_API_KEY").unwrap_or_else(|_| {
        eprintln!("DBOT_API_KEY not set; using test key");
        DEFAULT_API_KEY.to_string()
    });

    // Search pools by mint/CA
    let search_url = format!(
        "https://api-data-v1.dbotx.com/kline/search?keyword={}",
        mint
    );
    let body: SearchResp = client
        .get(&search_url)
        .send()
        .await?
        .json()
        .await?;
    if body.err == Some(true) {
        anyhow::bail!("dbot error from search endpoint");
    }
    let pool = body
        .pools
        .into_iter()
        .filter(|p| p.sol_reserve.unwrap_or(0.0) > 0.0)
        .max_by(|a, b| {
            a.sol_reserve
                .partial_cmp(&b.sol_reserve)
                .unwrap_or(Ordering::Equal)
        })
        .ok_or_else(|| anyhow!("no pool found for {mint}"))?;
    println!("selected pool: {:?}", pool);

    // Place a small IOC buy order using API key header auth
    let order_url = "https://api-bot-v1.dbotx.com/trade/order";
    let token_price = pool
        .token_price
        .ok_or_else(|| anyhow!("pool {} missing price", pool.pair_id))?;
    let price = token_price * 0.98;
    let payload = serde_json::json!({
        "chain": "solana",
        "pair": pool.pair_id,
        "side": "buy",
        "type": "limit",
        "price": price,
        "size": "0.01",
        "timeInForce": "IOC",
        "slippageBps": 250,
        "clientOrderId": uuid::Uuid::new_v4().to_string(),
    });

    let resp: OrderResp = client
        .post(order_url)
        .header("X-API-KEY", api_key)
        .json(&payload)
        .send()
        .await?
        .json()
        .await?;
    println!("order response: {:?}", resp);

    Ok(())
}
