use anyhow::Result;
use reqwest::Client;
use serde::Deserialize;

const API_KEY: &str = "ckydkvw5urnw3shhjmz9wvuqmoqt36l2";

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

    // Search pools by mint/CA
    let search_url = format!(
        "https://api-data-v1.dbotx.com/kline/search?keyword={}",
        mint
    );

        .get(&search_url)
        .send()
        .await?
        .json()
        .await?;

    if pools.is_empty() {
        eprintln!("no pool found for {}", mint);
        return Ok(());
    }
    let pool = &pools[0];
    println!("selected pool: {:?}", pool);

    // Place a small IOC buy order using API key header auth
    let order_url = "https://api-bot-v1.dbotx.com/trade/order";
    let price = pool.token_price.unwrap_or(0.0) * 0.98;
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
        .header("X-API-KEY", API_KEY)
        .json(&payload)
        .send()
        .await?
        .json()
        .await?;
    println!("order response: {:?}", resp);

    Ok(())
}
