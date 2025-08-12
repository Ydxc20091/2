use anyhow::{anyhow, Context, Result};
use rand::{distributions::Alphanumeric, Rng};
use reqwest::Client;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{cmp::Ordering, time::Duration};
use tokio::time::sleep;

/// ----------------------------
/// 配置与默认参数
/// ----------------------------
const DATA_BASE: &str = "https://api-data-v1.dbotx.com";
const TRADE_BASE: &str = "https://api-bot-v1.dbotx.com";
const DEFAULT_API_KEY: &str = "ckydkvw5urnw3shhjmz9wvuqmoqt36l2";

const DEFAULT_UNIT_SOL: f64 = 0.02;     // 每次下单的 SOL
const DEFAULT_IOC_SKEW_BPS: f64 = 100.; // IOC 买：+1%；IOC 卖：-1%
const DEFAULT_GRID_PCT: f64 = 0.015;    // 1.5%
const DEFAULT_MIN_TP: f64 = 0.02;       // 2%
const DEFAULT_MAX_TP: f64 = 0.05;       // 5%
const DEFAULT_K1_ATR: f64 = 1.4;        // TP = max(k1*ATR1m, m1*grid)
const DEFAULT_M1_GRID: f64 = 1.6;
const DEFAULT_K2_TRAIL: f64 = 0.6;      // TS = max(k2*TP, grid)
const DEFAULT_MIN_TS: f64 = 0.008;      // 0.8%
const DEFAULT_MAX_TS: f64 = 0.03;       // 3%
const DEFAULT_HARD_SL: f64 = 0.12;      // 12%
const DEFAULT_MAX_HOLD_SECS: u64 = 900; // 最长持仓 15 分钟
const POLL_INTERVAL_MS: u64 = 3000;

/// ----------------------------
/// 数据结构
/// ----------------------------
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
    #[serde(alias = "res", alias = "data", default)]
    pools: Vec<PoolInfo>,
    #[serde(default)]
    err: Option<bool>,
}

#[derive(Debug, Deserialize)]
struct OrderResp {
    #[serde(alias = "orderId")]
    order_id: String,
    status: String,
    // 真实接口里一般会返回成交均价/成交量等，这里未列出是为了兼容示例 key
}

#[derive(Debug, Clone, Copy, Deserialize)]
struct Candle {
    #[serde(default)]
    open: f64,
    #[serde(default)]
    high: f64,
    #[serde(default)]
    low: f64,
    #[serde(default)]
    close: f64,
    #[serde(default)]
    t: i64,
}

/// ----------------------------
/// 工具：安全读浮点
/// ----------------------------
fn f(v: &Value, k: &str) -> Option<f64> {
    v.get(k)?.as_f64()
}

/// ----------------------------
/// 1) 搜池 -> 选“总成本最低”的近似池
///    - 这里用 “高流动性 + dex 偏好” 近似最低滑点/总成本
/// ----------------------------
async fn search_pools(client: &Client, mint: &str) -> Result<Vec<PoolInfo>> {
    let url = format!("{DATA_BASE}/kline/search?keyword={mint}");
    let resp = client.get(&url).send().await?.error_for_status()?;
    let body: SearchResp = resp.json().await?;
    if body.err == Some(true) {
        return Err(anyhow!("dbot search endpoint returned err=true"));
    }
    Ok(body.pools)
}

fn score_pool(p: &PoolInfo) -> (i32, f64) {
    // dex 偏好：raydium_cpmm > meteora_dlmm > 其他
    let dex_rank = match p.dex.as_deref() {
        Some("raydium_cpmm") => 0,
        Some("meteora_dlmm") | Some("meteora_dyn2") => 1,
        _ => 2,
    };
    // 负的流动性排名（越大越好，所以乘 -1 让它排序到前面）
    let liq = -(p.sol_reserve.unwrap_or(0.0));
    (dex_rank, liq)
}

fn choose_best_pool(mut pools: Vec<PoolInfo>) -> Option<PoolInfo> {
    pools
        .into_iter()
        .filter(|p| p.sol_reserve.unwrap_or(0.0) > 0.0)
        .min_by(|a, b| {
            let sa = score_pool(a);
            let sb = score_pool(b);
            sa.cmp(&sb)
        })
}

async fn refresh_pool_price(client: &Client, mint: &str, pair_id: &str) -> Result<f64> {
    // 用 search 再捞一次价格，找到同一个 pair
    let pools = search_pools(client, mint).await?;
    let price = pools
        .into_iter()
        .find(|p| p.pair_id == pair_id)
        .and_then(|p| p.token_price)
        .ok_or_else(|| anyhow!("cannot refresh price for pair {}", pair_id))?;
    Ok(price)
}

/// ----------------------------
/// 2) 拉 1m K线，算 ATR1m%（拿不到就兜底）
/// ----------------------------
async fn fetch_candles_try(client: &Client, pair_id: &str) -> Result<Vec<Candle>> {
    // 兼容多种参数名与路径
    let try_params = [
        ("ohlcv", "pair"),
        ("ohlcv", "id"),
        ("ohlcv", "pairId"),
        ("kline", "pair"),
        ("klines", "pair"),
        ("candles", "pair"),
    ];

    for (path, key) in try_params {
        let url = format!("{DATA_BASE}/kline/{path}?chain=solana&{key}={pair_id}&interval=1m&limit=20");
        let resp = client.get(&url).send().await?;
        if !resp.status().is_success() { continue; }

        // 既可能是数组，也可能是对象里包数组；尽量宽松解析
        let v: Value = resp.json().await.unwrap_or(Value::Null);
        let arr_opt = if v.is_array() {
            v.as_array().cloned()
        } else {
            v.get("res").and_then(|x| x.as_array().cloned())
                .or_else(|| v.get("data").and_then(|x| x.as_array().cloned()))
                .or_else(|| v.pointer("/res/list").and_then(|x| x.as_array().cloned()))
        };

        if let Some(arr) = arr_opt {
            let mut out = Vec::new();
            for it in arr {
                // 允许字段缺失
                let c = Candle {
                    open: f(&it, "open").unwrap_or_default(),
                    high: f(&it, "high").unwrap_or_default(),
                    low:  f(&it, "low").unwrap_or_default(),
                    close:f(&it, "close").unwrap_or_default(),
                    t:    it.get("t").and_then(|x| x.as_i64()).unwrap_or_default(),
                };
                if c.close > 0.0 && c.high >= c.low {
                    out.push(c);
                }
            }
            if !out.is_empty() {
                return Ok(out);
            }
        }
    }
    Err(anyhow!("no candles from known endpoints"))
}

fn atr1m_pct(candles: &[Candle]) -> f64 {
    if candles.is_empty() { return DEFAULT_GRID_PCT; }
    let n = candles.len() as f64;
    let mut sum = 0.0;
    let mut last_close = candles[0].close;
    for c in candles {
        let tr = (c.high - c.low)
            .max((c.high - last_close).abs())
            .max((c.low - last_close).abs());
        if c.close > 0.0 {
            sum += tr / c.close;
        }
        last_close = c.close;
    }
    // 平均 TR 占价的百分比
    (sum / n).max(0.003).min(0.05) // 0.3%~5% 的夹逼，避免异常
}

/// ----------------------------
/// 3) 动态 TP/TS 计算
/// ----------------------------
#[derive(Debug, Clone, Copy)]
struct TpTsCfg {
    min_tp: f64,
    max_tp: f64,
    k1_atr_mult: f64,
    m1_grid_mult: f64,
    k2_trail_mult: f64,
    min_trail: f64,
    max_trail: f64,
}
impl Default for TpTsCfg {
    fn default() -> Self {
        Self {
            min_tp: DEFAULT_MIN_TP,
            max_tp: DEFAULT_MAX_TP,
            k1_atr_mult: DEFAULT_K1_ATR,
            m1_grid_mult: DEFAULT_M1_GRID,
            k2_trail_mult: DEFAULT_K2_TRAIL,
            min_trail: DEFAULT_MIN_TS,
            max_trail: DEFAULT_MAX_TS,
        }
    }
}

fn decide_tp_ts(p0: f64, atr1m_pct: f64, grid_pct: f64, cfg: TpTsCfg) -> (f64, f64) {
    let mut tp = (cfg.k1_atr_mult * atr1m_pct).max(cfg.m1_grid_mult * grid_pct);
    tp = tp.clamp(cfg.min_tp, cfg.max_tp);
    let mut ts = (cfg.k2_trail_mult * tp).max(grid_pct);
    ts = ts.clamp(cfg.min_trail, cfg.max_trail);
    tracing::info!("entry={:.8}, atr1m%={:.3}%, grid%={:.2}%, tp%={:.2}%, ts%={:.2}%",
        p0, atr1m_pct*100.0, grid_pct*100.0, tp*100.0, ts*100.0);
    (tp, ts)
}

/// ----------------------------
/// 4) 下单与轮询
/// ----------------------------
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct OrderPayload<'a> {
    chain: &'a str,
    pair: &'a str,
    side: &'a str,     // "buy"/"sell"
    #[serde(rename = "type")]
    typ: &'a str,      // "limit"
    price: f64,
    size: String,      // SOL 数量字符串
    time_in_force: &'a str, // "IOC"
    slippage_bps: u32,
    client_order_id: String,
}

async fn ioc_limit_buy(
    client: &Client,
    api_key: &str,
    pair_id: &str,
    price: f64,
    size_sol: f64,
) -> Result<OrderResp> {
    let url = format!("{TRADE_BASE}/trade/order");
    let payload = OrderPayload {
        chain: "solana",
        pair: pair_id,
        side: "buy",
        typ: "limit",
        price,
        size: format!("{:.6}", size_sol),
        time_in_force: "IOC",
        slippage_bps: 250,
        client_order_id: rand_id(),
    };
    let resp = client
        .post(url)
        .header("X-API-KEY", api_key)
        .json(&payload)
        .send()
        .await?
        .error_for_status()?
        .json::<OrderResp>()
        .await?;
    Ok(resp)
}

async fn ioc_limit_sell_all(
    client: &Client,
    api_key: &str,
    pair_id: &str,
    price: f64,
    est_pos_sol: f64,
) -> Result<OrderResp> {
    // 这里为了示例，直接把“估算仓位”一次性卖出；
    // 真实情况应使用钱包查询可用余额/持仓合约返回的数量
    let url = format!("{TRADE_BASE}/trade/order");
    let payload = OrderPayload {
        chain: "solana",
        pair: pair_id,
        side: "sell",
        typ: "limit",
        price,
        size: format!("{:.6}", est_pos_sol),
        time_in_force: "IOC",
        slippage_bps: 250,
        client_order_id: rand_id(),
    };
    let resp = client
        .post(url)
        .header("X-API-KEY", api_key)
        .json(&payload)
        .send()
        .await?
        .error_for_status()?
        .json::<OrderResp>()
        .await?;
    Ok(resp)
}

fn rand_id() -> String {
    rand::thread_rng()
        .sample_iter(&Alphanumeric)
        .take(16)
        .map(char::from)
        .collect()
}

/// ----------------------------
/// 主流程
/// ----------------------------
#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter("info")
        .with_target(false)
        .compact()
        .init();

    let args: Vec<String> = std::env::args().collect();
    if args.len() < 2 {
        eprintln!("usage: {} <mint/ca> [unit_sol]", args[0]);
        std::process::exit(1);
    }
    let mint = &args[1];
    let unit_sol: f64 = args
        .get(2)
        .and_then(|x| x.parse().ok())
        .unwrap_or(DEFAULT_UNIT_SOL);

    let api_key = std::env::var("DBOT_API_KEY").unwrap_or_else(|_| {
        eprintln!("DBOT_API_KEY not set; using test key");
        DEFAULT_API_KEY.to_string()
    });

    let client = Client::builder()
        .user_agent("dbot-grid-tp-bot/0.1")
        .timeout(Duration::from_secs(15))
        .build()?;

    // 1) 搜索并选择最优池
    let pools = search_pools(&client, mint).await.context("search pools")?;
    if pools.is_empty() {
        return Err(anyhow!("no pool found for {}", mint));
    }
    let pool = choose_best_pool(pools).ok_or_else(|| anyhow!("no eligible pool"))?;
    let pair_id = pool.pair_id.clone();
    let dex = pool.dex.clone().unwrap_or_default();
    tracing::info!("selected pool: pair={} dex={} liq(SOL)={:.4}",
        pair_id, dex, pool.sol_reserve.unwrap_or_default());

    // 2) 刷一个最新价格
    let mut last_price = refresh_pool_price(&client, mint, &pair_id)
        .await
        .or_else(|_| pool.token_price.ok_or_else(|| anyhow!("pool missing price"))))?;
    tracing::info!("last price ~ {:.10}", last_price);

    // 3) 拉 K 线，计算 ATR1m%
    let atr_pct = fetch_candles_try(&client, &pair_id)
        .await
        .map(|cs| atr1m_pct(&cs))
        .unwrap_or(0.015);
    let grid_pct = DEFAULT_GRID_PCT;
    let cfg = TpTsCfg::default();

    // 4) 计算动态 TP/TS（2%–5% + 跟踪）
    let (tp_pct, ts_pct) = decide_tp_ts(last_price, atr_pct, grid_pct, cfg);
    let sell_splits = [0.4, 0.2, 0.2, 0.2];

    // 5) 进场：IOC 买（为保证成交，买单价格抬高 +ioc_skew）
    let buy_price = last_price * (1.0 + DEFAULT_IOC_SKEW_BPS / 10_000.0);
    let _order = ioc_limit_buy(&client, &api_key, &pair_id, buy_price, unit_sol)
        .await
        .context("place IOC buy")?;
    tracing::info!("buy placed IOC@{:.8} size={}", buy_price, unit_sol);

    // 入场均价（真实环境应以成交均价替代）
    let entry = refresh_pool_price(&client, mint, &pair_id).await.unwrap_or(last_price);
    let mut high = entry;
    let mut remaining = unit_sol;

    // 计算分批止盈价阶梯（随网格推进）
    let mut tp_levels = Vec::new();
    let mut acc = tp_pct;
    for _ in 0..sell_splits.len() {
        tp_levels.push(entry * (1.0 + acc));
        acc += grid_pct;
    }

    tracing::info!(
        "entry={:.10}, tp_levels={:?}, trail%={:.2}%, hardSL={:.2}%",
        entry,
        tp_levels.iter().map(|x| format!("{:.10}", x)).collect::<Vec<_>>(),
        ts_pct * 100.0,
        DEFAULT_HARD_SL * 100.0
    );

    let start = std::time::Instant::now();
    let mut next_idx = 0usize;

    // 6) 轮询价格 → 分批止盈 + 跟踪止盈 + 硬止损
    loop {
        if start.elapsed().as_secs() > DEFAULT_MAX_HOLD_SECS {
            tracing::warn!("max holding time reached, exit by IOC");
            // 以“可成交限价”卖出全部（降 1%）
            let px = refresh_pool_price(&client, mint, &pair_id).await.unwrap_or(last_price)
                * (1.0 - DEFAULT_IOC_SKEW_BPS / 10_000.0);
            let _ = ioc_limit_sell_all(&client, &api_key, &pair_id, px, remaining).await;
            break;
        }

        sleep(Duration::from_millis(POLL_INTERVAL_MS)).await;
        last_price = refresh_pool_price(&client, mint, &pair_id).await.unwrap_or(last_price);

        if last_price > high {
            high = last_price;
        }

        // 硬止损
        if last_price <= entry * (1.0 - DEFAULT_HARD_SL) {
            tracing::warn!("hard stop loss hit. price={:.10}", last_price);
            let px = last_price * (1.0 - DEFAULT_IOC_SKEW_BPS / 10_000.0);
            let _ = ioc_limit_sell_all(&client, &api_key, &pair_id, px, remaining).await;
            break;
        }

        // 分批止盈
        while next_idx < tp_levels.len() && last_price >= tp_levels[next_idx] && remaining > 0.0 {
            let part = (sell_splits[next_idx] * unit_sol).min(remaining);
            let px = last_price * (1.0 - DEFAULT_IOC_SKEW_BPS / 10_000.0); // 卖单降 1% 提高成交
            let _ = ioc_limit_sell_all(&client, &api_key, &pair_id, px, part).await;
            remaining -= part;
            tracing::info!(
                "TP#{}, px={:.10}, part={:.6}, left={:.6}",
                next_idx + 1,
                px,
                part,
                remaining
            );
            next_idx += 1;
        }
        if remaining <= 1e-9 {
            tracing::info!("all taken profit. exit");
            break;
        }

        // 跟踪止盈：从新高回撤 ts%
        let protect = high * (1.0 - ts_pct);
        if last_price <= protect {
            let px = last_price * (1.0 - DEFAULT_IOC_SKEW_BPS / 10_000.0);
            let _ = ioc_limit_sell_all(&client, &api_key, &pair_id, px, remaining).await;
            tracing::info!(
                "trailing stop hit. H={:.10} P={:.10} sell_all@{:.10}",
                high,
                protect,
                px
            );
            break;
        }
    }

    Ok(())
}
