//! Normalizes Binance Web3 RWA Data API responses into this service's
//! `Asset`/`Provider` model (see architecture doc section 4.2, "Asset
//! Normalization Layer") and builds real executable quotes via the Trading
//! API's DEX aggregator (section 6, "Executable Spread Engine").

use crate::binance::BinanceClient;
use crate::{radar_score, reference_gap, z_score, Asset, Provider};
use serde_json::{json, Value};
use sqlx::PgPool;
use std::collections::HashMap;

const BSC_CHAIN_ID: &str = "56";
/// USDT on BNB Smart Chain (BEP-20), 18 decimals — the counter asset every
/// executable quote is priced against.
const BSC_USDT: &str = "0x55d398326f99059fF775485246999027B3197955";
const USDT_DECIMALS: i32 = 18;

fn as_str<'a>(v: &'a Value, key: &str) -> Option<&'a str> {
    v.get(key).and_then(Value::as_str)
}
/// RWA numeric fields come back as JSON strings (e.g. `"61.89"`); accept a
/// bare number too in case a future response shape changes that.
fn as_num(v: &Value, key: &str) -> Option<f64> {
    v.get(key).and_then(|f| {
        f.as_str()
            .and_then(|s| s.parse::<f64>().ok())
            .or_else(|| f.as_f64())
    })
}

fn platform_label(id: &str) -> String {
    match id.to_ascii_lowercase().as_str() {
        "ondo" => "Ondo".into(),
        "bstock" | "bstocks" => "bStocks".into(),
        "xstock" | "xstocks" => "xStocks".into(),
        other => other.into(),
    }
}

/// Fetch the live BSC tokenized-stock list and normalize it into `Asset`s,
/// one per underlying ticker with one `Provider` per issuance platform.
/// Returns `Err` on transport/API failure; callers keep serving the last
/// known-good in-memory snapshot rather than resetting to nothing.
pub async fn load_live_assets(db: &PgPool, client: &BinanceClient) -> Result<Vec<Asset>, String> {
    let items = client
        .rwa_tokens(Some(BSC_CHAIN_ID), None)
        .await
        .map_err(|e| e.to_string())?;
    if items.is_empty() {
        return Err("RWA token list returned no rows".into());
    }
    let mut groups: HashMap<String, Vec<Value>> = HashMap::new();
    for item in items {
        if let Some(ticker) = as_str(&item, "underlyingTicker") {
            groups.entry(ticker.to_string()).or_default().push(item);
        }
    }
    let mut assets = Vec::with_capacity(groups.len());
    for (ticker, group) in &groups {
        if let Some(asset) = build_asset(db, ticker, group).await {
            assets.push(asset);
        }
    }
    assets.sort_by(|a, b| a.symbol.cmp(&b.symbol));
    Ok(assets)
}

async fn build_asset(db: &PgPool, ticker: &str, group: &[Value]) -> Option<Asset> {
    let mut providers = Vec::new();
    let mut reference = 0.0f64;
    let mut name = ticker.to_string();
    let mut volume = 0.0f64;
    let mut market_status = None;
    let mut any_open = false;
    for item in group {
        let platform = as_str(item, "platformId").unwrap_or("unknown");
        let price = match as_num(item, "tokenPrice") {
            Some(p) if p > 0.0 => p,
            _ => continue,
        };
        if let Some(r) = as_num(item, "referencePrice") {
            reference = reference.max(r);
        }
        if let Some(n) = as_str(item, "underlyingName") {
            name = n.to_string();
        }
        volume += as_num(item, "volume24H").unwrap_or(0.0);
        if let Some(status) = item.get("statusInfo") {
            market_status = as_str(status, "marketStatus")
                .map(str::to_string)
                .or(market_status);
            if status.get("openState").and_then(Value::as_bool).unwrap_or(false) {
                any_open = true;
            }
        }
        providers.push(Provider {
            name: platform_label(platform),
            price,
            contract_address: as_str(item, "tokenContractAddress").map(str::to_string),
            chain_id: as_str(item, "binanceChainId").map(str::to_string),
            decimals: item.get("decimals").and_then(Value::as_i64).map(|d| d as i32),
        });
    }
    if providers.is_empty() || reference <= 0.0 {
        return None;
    }
    let primary = providers
        .iter()
        .max_by(|a, b| a.price.partial_cmp(&b.price).unwrap_or(std::cmp::Ordering::Equal))
        .cloned()?;
    // Best-effort recent price history from our own snapshot cache; a fresh
    // ticker with no history yet just gets a single flat point (z-score of 0
    // until enough ticks accumulate) rather than a fabricated trend.
    let history = price_history(db, ticker, &primary.name)
        .await
        .unwrap_or_else(|| vec![primary.price]);
    let gaps: Vec<f64> = history.iter().map(|h| (h - reference) / reference).collect();
    let gap = reference_gap(primary.price, reference).unwrap_or(0.0);
    let z = z_score(gap, &gaps);
    let low = providers.iter().map(|p| p.price).fold(f64::INFINITY, f64::min);
    let high = providers.iter().map(|p| p.price).fold(0.0, f64::max);
    let spread = if low > 0.0 { (high - low) / low } else { 0.0 };
    // Momentum over the retained snapshot window (up to ~30 minutes at the
    // current 60s tick rate) — not a calendar 24h change; see README.
    let change = match history.first() {
        Some(&first) if history.len() >= 2 && first > 0.0 => {
            (primary.price - first) / first * 100.0
        }
        _ => 0.0,
    };
    Some(Asset {
        symbol: ticker.to_string(),
        name,
        sector: "Tokenized Equity".into(),
        price: primary.price,
        reference,
        change,
        volume,
        // Volume-magnitude proxy, not a real DEX liquidity-depth curve; see
        // README for the plan to source this from the top-liquidity endpoint.
        liquidity: volume.max(1.0),
        score: radar_score(gap, z, spread, volume.max(1.0), !any_open),
        providers,
        history,
        market_status,
    })
}

async fn price_history(db: &PgPool, ticker: &str, provider: &str) -> Option<Vec<f64>> {
    // Bounded to a recent window so a restart, a demo/live mode switch, or a
    // long gap in the worker never blends in stale rows as if they were
    // continuous history (they'd otherwise silently distort z-score/change).
    let rows: Vec<f64> = sqlx::query_scalar(
        "SELECT ms.token_price FROM market_snapshots ms \
         JOIN tokenized_assets t ON ms.tokenized_asset_id = t.id \
         JOIN underlying_assets u ON t.underlying_id = u.id \
         WHERE u.ticker = $1 AND t.provider = $2 \
           AND ms.created_at > now() - interval '30 minutes' \
         ORDER BY ms.created_at DESC LIMIT 30",
    )
    .bind(ticker)
    .bind(provider)
    .fetch_all(db)
    .await
    .ok()?;
    if rows.is_empty() {
        None
    } else {
        Some(rows.into_iter().rev().collect())
    }
}

/// The concrete DEX route (chain + token addresses + raw smallest-unit
/// amount) for swapping `amount_usd` of a provider's token. Shared by
/// [`live_quote`] and `main.rs`'s real-signing `/api/trades/prepare` so both
/// paths always compute the exact same amount for the exact same trade —
/// any drift between them could size the actual on-chain swap differently
/// from what was quoted.
pub struct SwapRoute {
    pub chain: String,
    pub from_token: String,
    pub to_token: String,
    pub raw_amount: String,
}

pub fn resolve_swap_route(provider: &Provider, side: &str, amount_usd: f64) -> Option<SwapRoute> {
    let contract = provider.contract_address.as_ref()?;
    let chain = provider
        .chain_id
        .clone()
        .unwrap_or_else(|| BSC_CHAIN_ID.into());
    let decimals = provider.decimals.unwrap_or(18);
    if provider.price <= 0.0 || !amount_usd.is_finite() || amount_usd <= 0.0 {
        return None;
    }
    let (from_token, to_token, from_decimals, from_units) = if side == "sell" {
        (contract.clone(), BSC_USDT.to_string(), decimals, amount_usd / provider.price)
    } else {
        (BSC_USDT.to_string(), contract.clone(), USDT_DECIMALS, amount_usd)
    };
    let raw = (from_units * 10f64.powi(from_decimals)).round();
    if !raw.is_finite() || raw <= 0.0 {
        return None;
    }
    Some(SwapRoute {
        chain,
        from_token,
        to_token,
        raw_amount: format!("{raw:.0}"),
    })
}

/// Normalize a Binance "build swap transaction" response down to the minimal
/// `{to, data, value, gas?, gasPrice?}` shape safe to hand to a browser
/// wallet.
///
/// Forwards Binance's own `gasPrice` (legacy pricing, not the EIP-1559
/// `maxFeePerGas`/`maxPriorityFeePerGas` pair, since BSC wallets handle plain
/// `gasPrice` most reliably) as a live, just-fetched suggestion. This used to
/// be deliberately dropped on the theory that the wallet's own fee-market
/// estimate would be more trustworthy — in practice a wallet's cached
/// default can be stale or too low for the current block, leaving the
/// transaction stuck pending until the wallet auto-cancels it (confirmed via
/// a real signing attempt whose approve tx sat pending and then showed as
/// "cancel" in the wallet's own activity view). Binance's price is fetched
/// moments before signing, so it's a better default; wallets still let the
/// user override it before sending.
///
/// The real response nests the actual transaction under a `tx` key —
/// `{"executionMode":"SWAP","routerResult":{...},"tx":{"to":...,"data":...}}`
/// — not at the top level or in an array as the Binance docs alone
/// suggested; confirmed via a captured raw response after this previously
/// returned `None` for every real swap attempt (SWAP_PARSE_FAILED). Still
/// falls back to a bare object or one-element array for robustness in case
/// another route ever returns the flatter shape.
pub fn normalize_swap_tx(v: &Value) -> Option<Value> {
    let obj = if v.is_array() {
        v.as_array()?.first()?.clone()
    } else {
        v.clone()
    };
    let obj = obj.get("tx").cloned().unwrap_or(obj);
    let to = obj.get("to").and_then(Value::as_str)?.to_string();
    let data = obj
        .get("data")
        .and_then(Value::as_str)
        .unwrap_or("0x")
        .to_string();
    let value = obj
        .get("value")
        .and_then(Value::as_str)
        .unwrap_or("0")
        .to_string();
    let mut tx = json!({ "to": to, "data": data, "value": value });
    if let Some(gas) = obj.get("gas").and_then(Value::as_str) {
        tx["gas"] = json!(gas);
    }
    if let Some(gas_price) = obj.get("gasPrice").and_then(Value::as_str) {
        tx["gasPrice"] = json!(gas_price);
    }
    Some(tx)
}

/// Normalize a Binance "approve transaction" response. The documented
/// response shape carries `data`/`gasLimit` but not a reliable `to` — so
/// `to` is set here to `token_contract` directly: calling `approve()` on
/// anything other than the token's own contract is meaningless for ERC-20,
/// so this is correct by construction rather than by trusting a field name.
/// Also forwards `gasPrice` when present, for the same stuck-pending-tx
/// reason as `normalize_swap_tx` above.
pub fn normalize_approve_tx(items: &[Value], token_contract: &str) -> Option<Value> {
    let obj = items.first()?;
    let data = obj.get("data").and_then(Value::as_str)?.to_string();
    let mut tx = json!({ "to": token_contract, "data": data, "value": "0" });
    if let Some(gas) = obj.get("gasLimit").and_then(Value::as_str) {
        tx["gas"] = json!(gas);
    }
    if let Some(gas_price) = obj.get("gasPrice").and_then(Value::as_str) {
        tx["gasPrice"] = json!(gas_price);
    }
    Some(tx)
}

/// Build a real executable quote via the DEX aggregator for a sell/buy of
/// `amount_usd` against this provider's on-chain token. Returns `None` when
/// the provider has no known contract mapping or the live call fails, so the
/// caller can fall back to the demo formula.
pub async fn live_quote(
    client: &BinanceClient,
    provider: &Provider,
    side: &str,
    amount_usd: f64,
    wallet_address: Option<&str>,
) -> Option<Value> {
    let route = resolve_swap_route(provider, side, amount_usd)?;
    let (chain, from_token, to_token, raw_amount) =
        (route.chain.as_str(), route.from_token.as_str(), route.to_token.as_str(), route.raw_amount.as_str());
    let decimals = provider.decimals.unwrap_or(18);
    let routes = match client
        .aggregator_quote(chain, raw_amount, from_token, to_token, wallet_address)
        .await
    {
        Ok(r) => r,
        Err(e) => {
            tracing::warn!(error=%e, from_token, to_token, "aggregator_quote failed; falling back to demo formula");
            return None;
        }
    };
    let Some(best) = routes
        .iter()
        .find(|r| r.get("isBest").and_then(Value::as_bool).unwrap_or(false))
        .or_else(|| routes.first())
    else {
        tracing::warn!(from_token, to_token, "aggregator_quote returned no routes; falling back to demo formula");
        return None;
    };
    let to_amount: f64 = as_str(best, "toTokenAmount")?.parse().ok()?;
    let to_decimals = if side == "sell" { USDT_DECIMALS } else { decimals };
    let raw_output = to_amount / 10f64.powi(to_decimals);
    let expected_output_usd = if side == "sell" {
        raw_output
    } else {
        raw_output * provider.price
    };
    let price_impact = as_str(best, "priceImpactPercent")
        .and_then(|s| s.parse::<f64>().ok())
        .map(|p| p.abs() / 100.0)
        .unwrap_or(0.0);
    let execution_gap = if amount_usd > 0.0 {
        expected_output_usd / amount_usd - 1.0
    } else {
        0.0
    };
    let gas_estimate = estimate_gas_bnb(client, chain, best).await;
    Some(json!({
        "expectedOutput": expected_output_usd,
        "priceImpact": price_impact,
        "executionGap": execution_gap,
        "gasEstimate": gas_estimate,
        "mode": "live",
        "vendor": best.get("vendorName"),
        "quoteId": best.get("quoteId"),
    }))
}

async fn estimate_gas_bnb(client: &BinanceClient, chain: &str, route: &Value) -> f64 {
    const FALLBACK: f64 = 0.01;
    let gas_units: f64 = match as_str(route, "estimateGasFee").and_then(|s| s.parse::<f64>().ok()) {
        Some(g) if g > 0.0 => g,
        _ => return FALLBACK,
    };
    match client.gas_price(chain).await {
        Ok(v) => {
            let wei = v
                .get("evmLegacyGasPrice")
                .and_then(|g| as_str(g, "mediumGasPrice"))
                .and_then(|s| s.parse::<f64>().ok())
                .unwrap_or(0.0);
            if wei > 0.0 {
                gas_units * wei / 1e18
            } else {
                FALLBACK
            }
        }
        Err(_) => FALLBACK,
    }
}
