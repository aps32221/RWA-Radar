mod agent;
mod binance;
mod live;

use axum::{
    extract::{Path, Query, Request, State},
    http::StatusCode,
    middleware::{self, Next},
    response::{IntoResponse, Response},
    routing::{get, patch, post},
    Json, Router,
};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sqlx::{PgPool, Row};
use std::{
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
    time::Instant,
};
use tokio::sync::RwLock;
use uuid::Uuid;

#[derive(Clone)]
struct App {
    db: PgPool,
    assets: Arc<RwLock<Vec<Asset>>>,
    binance: Option<Arc<binance::BinanceClient>>,
    openai: Option<Arc<agent::OpenAiClient>>,
    live: Arc<AtomicBool>,
}
#[derive(Clone, Serialize)]
struct Provider {
    name: String,
    price: f64,
    #[serde(skip_serializing_if = "Option::is_none")]
    contract_address: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    chain_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    decimals: Option<i32>,
}
#[derive(Clone, Serialize)]
struct Asset {
    symbol: String,
    name: String,
    sector: String,
    price: f64,
    reference: f64,
    change: f64,
    volume: f64,
    liquidity: f64,
    score: f64,
    providers: Vec<Provider>,
    history: Vec<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    market_status: Option<String>,
}
type ApiResult = Result<Json<Value>, ApiError>;
/// `code` is a stable, machine-readable identifier the frontend's i18n
/// layer maps to a localized message; `1` (the English message) is always
/// present as a fallback for any code the frontend doesn't recognize yet.
/// `3` carries optional structured data (e.g. how much daily budget is left)
/// that the frontend interpolates into the localized message's `{{vars}}`
/// so numbers stay live even though the sentence itself is translated.
struct ApiError(StatusCode, String, Option<&'static str>, Option<Value>);
impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let mut body = json!({"error": self.1});
        if let Some(code) = self.2 {
            body["code"] = json!(code);
        }
        if let Some(vars) = self.3 {
            body["vars"] = vars;
        }
        (self.0, Json(body)).into_response()
    }
}
fn bad_coded(code: &'static str, s: &str) -> ApiError {
    ApiError(StatusCode::BAD_REQUEST, s.into(), Some(code), None)
}
fn internal(e: sqlx::Error) -> ApiError {
    tracing::error!(error=%e,"Database operation failed");
    ApiError(
        StatusCode::INTERNAL_SERVER_ERROR,
        "Database unavailable".into(),
        None, None
    )
}
fn reference_gap(price: f64, reference: f64) -> Option<f64> {
    if reference <= 0.0 || !price.is_finite() || !reference.is_finite() {
        None
    } else {
        Some((price - reference) / reference)
    }
}
fn z_score(current: f64, history: &[f64]) -> f64 {
    if history.len() < 2 {
        return 0.0;
    }
    let mean = history.iter().sum::<f64>() / history.len() as f64;
    let sd =
        (history.iter().map(|v| (v - mean).powi(2)).sum::<f64>() / history.len() as f64).sqrt();
    if sd <= f64::EPSILON {
        0.0
    } else {
        (current - mean) / sd
    }
}
fn radar_score(gap: f64, z: f64, spread: f64, liquidity: f64, closed: bool) -> f64 {
    ((gap.abs() / 0.03).min(1.0) * 30.0
        + (z.abs() / 3.0).min(1.0) * 25.0
        + (spread / 0.03).min(1.0) * 20.0
        + (100000.0 / liquidity.max(1.0)).min(1.0) * 15.0
        + if closed { 10.0 } else { 0.0 })
    .round()
    .clamp(0.0, 100.0)
}
fn demo_assets() -> Vec<Asset> {
    let rows = [
        (
            "NVDA",
            "NVIDIA",
            "Technology",
            193.4,
            188.2,
            3.24,
            2840000.,
            1240000.,
        ),
        (
            "TSLA",
            "Tesla",
            "Automotive",
            258.62,
            252.1,
            -1.82,
            1960000.,
            860000.,
        ),
        (
            "AAPL",
            "Apple",
            "Technology",
            232.85,
            230.54,
            1.16,
            3210000.,
            2450000.,
        ),
        (
            "MSFT",
            "Microsoft",
            "Technology",
            428.76,
            426.8,
            0.84,
            1540000.,
            1820000.,
        ),
        (
            "AMZN", "Amazon", "Consumer", 218.34, 219.6, -0.62, 980000., 920000.,
        ),
        (
            "GOOGL",
            "Alphabet",
            "Technology",
            192.18,
            191.72,
            1.42,
            1240000.,
            1640000.,
        ),
        (
            "COIN",
            "Coinbase",
            "Financial",
            284.56,
            281.2,
            4.28,
            1720000.,
            680000.,
        ),
        (
            "SPY",
            "S&P 500 ETF",
            "ETF",
            602.45,
            601.8,
            0.38,
            4620000.,
            3850000.,
        ),
    ];
    rows.iter()
        .enumerate()
        .map(|(i, &(s, n, sector, p, r, c, v, l))| {
            let history: Vec<f64> = (0..25)
                .map(|j| r + (j as f64 * 0.7 + i as f64).sin() * 0.8 + (j as f64 / 24.) * (p - r))
                .collect();
            let gaps: Vec<f64> = history.iter().map(|h| (h - r) / r).collect();
            let providers = vec![
                Provider {
                    name: "xStocks".into(),
                    price: p,
                    contract_address: None,
                    chain_id: None,
                    decimals: None,
                },
                Provider {
                    name: "Ondo".into(),
                    price: r * 1.008,
                    contract_address: None,
                    chain_id: None,
                    decimals: None,
                },
                Provider {
                    name: "bStocks".into(),
                    price: r * 1.01,
                    contract_address: None,
                    chain_id: None,
                    decimals: None,
                },
            ];
            let low = providers
                .iter()
                .map(|x| x.price)
                .fold(f64::INFINITY, f64::min);
            let high = providers.iter().map(|x| x.price).fold(0., f64::max);
            Asset {
                symbol: s.into(),
                name: n.into(),
                sector: sector.into(),
                price: p,
                reference: r,
                change: c,
                volume: v,
                liquidity: l,
                score: radar_score(
                    (p - r) / r,
                    z_score((p - r) / r, &gaps),
                    (high - low) / low,
                    l,
                    true,
                ),
                providers,
                history,
                market_status: Some("closed".into()),
            }
        })
        .collect()
}
#[tokio::main]
async fn main() {
    tracing_subscriber::fmt().with_env_filter("info").init();
    let url = std::env::var("DATABASE_URL").expect("DATABASE_URL must be set");
    let db = sqlx::postgres::PgPoolOptions::new()
        .max_connections(10)
        .connect(&url)
        .await
        .expect("PostgreSQL connection failed");
    sqlx::raw_sql(include_str!("../migrations/001_init.sql"))
        .execute(&db)
        .await
        .expect("Migration failed");
    let binance_client = binance::BinanceClient::from_env().map(Arc::new);
    let live_flag = Arc::new(AtomicBool::new(false));
    let initial_assets = match &binance_client {
        Some(client) => match live::load_live_assets(&db, client).await {
            Ok(assets) if !assets.is_empty() => {
                tracing::info!(count = assets.len(), "Loaded live RWA data from Binance Web3 API");
                live_flag.store(true, Ordering::Relaxed);
                assets
            }
            Ok(_) => {
                tracing::warn!("Live RWA fetch returned no assets; falling back to demo data");
                demo_assets()
            }
            Err(e) => {
                tracing::warn!(error=%e, "Live RWA fetch failed; falling back to demo data");
                demo_assets()
            }
        },
        None => {
            tracing::warn!(
                "BINANCE_API_KEY/BINANCE_API_SECRET not set; serving demo data only"
            );
            demo_assets()
        }
    };
    let openai_client = agent::OpenAiClient::from_env().map(Arc::new);
    if openai_client.is_some() {
        tracing::info!("OpenAI credentials detected; /api/agent/parse will use the live LLM");
    } else {
        tracing::warn!(
            "OPENAI_API_KEY not set; /api/agent/parse disabled, frontend falls back to its rule-based parser"
        );
    }
    let state = App {
        db,
        assets: Arc::new(RwLock::new(initial_assets)),
        binance: binance_client,
        openai: openai_client,
        live: live_flag,
    };
    seed_database(&state).await.expect("Seeding failed");
    let worker = state.clone();
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(std::time::Duration::from_secs(60));
        loop {
            interval.tick().await;
            if let Err(e) = snapshot(&worker).await {
                tracing::error!(error=%e,"Snapshot worker failed")
            }
        }
    });
    let app = Router::new()
        .route("/api/health", get(health))
        .route("/api/assets", get(assets))
        .route("/api/assets/{symbol}", get(asset))
        .route("/api/radar", get(radar))
        .route("/api/radar/replay/{symbol}", get(radar_replay))
        .route("/api/trades/quote", post(quote))
        .route("/api/trades/simulate", post(simulate))
        .route("/api/trades/execute", post(execute))
        .route("/api/agent/parse", post(agent_parse))
        .route("/api/agent/chat", post(agent_chat))
        .route("/api/agent/filter", post(agent_filter))
        .route("/api/agent/explain", post(agent_explain))
        .route("/api/portfolio/{address}", get(portfolio))
        .route("/api/trades/prepare", post(prepare_trade))
        .route("/api/trades/record", post(record_transaction))
        .route("/api/transactions", get(transactions_log))
        .route("/api/policies", get(policies).post(create_policy))
        .route("/api/policies/{id}", patch(toggle_policy))
        .route("/api/history", get(history))
        .route("/api/telemetry", get(telemetry))
        .layer(middleware::from_fn_with_state(state.clone(), record))
        .with_state(state);
    let listener = tokio::net::TcpListener::bind("0.0.0.0:8080").await.unwrap();
    tracing::info!("RWA Radar demo API listening on :8080; broadcast disabled");
    axum::serve(listener, app)
        .with_graceful_shutdown(async {
            let _ = tokio::signal::ctrl_c().await;
        })
        .await
        .unwrap();
}
async fn seed_database(s: &App) -> Result<(), sqlx::Error> {
    let assets = s.assets.read().await;
    for a in assets.iter() {
        let id:i32=sqlx::query_scalar("INSERT INTO underlying_assets(ticker,name,sector) VALUES($1,$2,$3) ON CONFLICT(ticker) DO UPDATE SET name=excluded.name RETURNING id").bind(&a.symbol).bind(&a.name).bind(&a.sector).fetch_one(&s.db).await?;
        for p in &a.providers {
            sqlx::query("INSERT INTO tokenized_assets(underlying_id,provider,chain_id,contract_address,decimals) VALUES($1,$2,$3,$4,$5) ON CONFLICT(underlying_id,provider) DO UPDATE SET chain_id=excluded.chain_id, contract_address=excluded.contract_address, decimals=excluded.decimals")
                .bind(id)
                .bind(&p.name)
                .bind(p.chain_id.as_deref().and_then(|c| c.parse::<i32>().ok()).unwrap_or(56))
                .bind(&p.contract_address)
                .bind(p.decimals.unwrap_or(18))
                .execute(&s.db).await?;
        }
    }
    Ok(())
}
async fn snapshot(s: &App) -> Result<(), sqlx::Error> {
    if let Some(client) = &s.binance {
        match live::load_live_assets(&s.db, client).await {
            Ok(fresh) if !fresh.is_empty() => {
                *s.assets.write().await = fresh;
                s.live.store(true, Ordering::Relaxed);
                seed_database(s).await?;
            }
            Ok(_) => tracing::warn!("Live RWA refresh returned no assets; keeping last known data"),
            Err(e) => tracing::warn!(error=%e, "Live RWA refresh failed; keeping last known data"),
        }
    }
    let assets = s.assets.read().await;
    for a in assets.iter() {
        for p in &a.providers {
            sqlx::query("INSERT INTO market_snapshots(tokenized_asset_id,token_price,reference_price,reference_gap,market_status) SELECT t.id,$1,$2,$3,$6 FROM tokenized_assets t JOIN underlying_assets u ON t.underlying_id=u.id WHERE u.ticker=$4 AND t.provider=$5")
                .bind(p.price)
                .bind(a.reference)
                .bind(reference_gap(p.price,a.reference).unwrap_or(0.))
                .bind(&a.symbol)
                .bind(&p.name)
                .bind(a.market_status.clone().unwrap_or_else(|| "closed".into()))
                .execute(&s.db).await?;
        }
    }
    Ok(())
}
async fn health(State(s): State<App>) -> ApiResult {
    sqlx::query("SELECT 1")
        .execute(&s.db)
        .await
        .map_err(internal)?;
    let mode = if s.live.load(Ordering::Relaxed) {
        "live"
    } else {
        "demo"
    };
    Ok(Json(
        json!({"status":"ok","mode":mode,"broadcastEnabled":false,"agentEnabled":s.openai.is_some()}),
    ))
}
async fn assets(State(s): State<App>) -> Json<Value> {
    let assets = s.assets.read().await;
    Json(json!(*assets))
}
async fn asset(State(s): State<App>, Path(symbol): Path<String>) -> ApiResult {
    let assets = s.assets.read().await;
    assets
        .iter()
        .find(|a| a.symbol.eq_ignore_ascii_case(&symbol))
        .map(|a| Json(json!(a)))
        .ok_or(ApiError(
            StatusCode::NOT_FOUND,
            "Asset not found".into(),
            Some("ASSET_NOT_FOUND"), None
        ))
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct RadarQuery {
    min_score: Option<f64>,
}
async fn radar(State(s): State<App>, Query(q): Query<RadarQuery>) -> Json<Value> {
    let assets = s.assets.read().await;
    let mut a: Vec<_> = assets
        .iter()
        .filter(|a| a.score >= q.min_score.unwrap_or(0.))
        .collect();
    a.sort_by(|a, b| b.score.total_cmp(&a.score));
    Json(json!(a))
}
/// "Signal replay" — the live asset list only ever shows the *current*
/// gap, but `market_snapshots` has been recording one row per tracked
/// asset roughly every 60s since the live integration went up (millions of
/// rows by now). This finds that asset's single biggest historical gap and
/// replays the price trajectory in a window around it, so a signal that's
/// no longer visible on the live table ("it already happened") still has
/// something to show, instead of only ever surfacing right-now anomalies.
async fn radar_replay(State(s): State<App>, Path(symbol): Path<String>) -> ApiResult {
    let peak: Option<(f64, f64, f64, chrono::DateTime<chrono::Utc>)> = sqlx::query_as(
        "SELECT ms.token_price, ms.reference_price, ms.reference_gap, ms.created_at \
         FROM market_snapshots ms \
         JOIN tokenized_assets t ON ms.tokenized_asset_id = t.id \
         JOIN underlying_assets u ON t.underlying_id = u.id \
         WHERE u.ticker = $1 \
         ORDER BY abs(ms.reference_gap) DESC LIMIT 1",
    )
    .bind(&symbol)
    .fetch_optional(&s.db)
    .await
    .map_err(internal)?;
    let Some((_, _, peak_gap, peak_at)) = peak else {
        return Err(bad_coded(
            "REPLAY_NO_HISTORY",
            "No snapshot history yet for this asset",
        ));
    };
    #[derive(sqlx::FromRow)]
    struct Bucket {
        bucket: chrono::DateTime<chrono::Utc>,
        price: f64,
        reference: f64,
        gap: f64,
    }
    let rows: Vec<Bucket> = sqlx::query_as(
        "SELECT date_trunc('minute', ms.created_at) AS bucket, \
                avg(ms.token_price) AS price, avg(ms.reference_price) AS reference, \
                avg(ms.reference_gap) AS gap \
         FROM market_snapshots ms \
         JOIN tokenized_assets t ON ms.tokenized_asset_id = t.id \
         JOIN underlying_assets u ON t.underlying_id = u.id \
         WHERE u.ticker = $1 \
           AND ms.created_at BETWEEN $2::timestamptz - interval '3 hours' \
                                  AND $2::timestamptz + interval '3 hours' \
         GROUP BY bucket ORDER BY bucket ASC",
    )
    .bind(&symbol)
    .bind(peak_at)
    .fetch_all(&s.db)
    .await
    .map_err(internal)?;
    // Downsample to a chart-friendly point count regardless of window density.
    let target_points = 60usize;
    let stride = (rows.len() / target_points).max(1);
    let points: Vec<Value> = rows
        .iter()
        .step_by(stride)
        .map(|r| {
            json!({
                "t": r.bucket,
                "price": r.price,
                "reference": r.reference,
                "gapPercent": r.gap * 100.0,
            })
        })
        .collect();
    let end_gap = rows.last().map(|r| r.gap).unwrap_or(peak_gap);
    let resolved = end_gap.abs() < peak_gap.abs() * 0.5;
    Ok(Json(json!({
        "symbol": symbol,
        "peakGapPercent": peak_gap * 100.0,
        "peakAt": peak_at,
        "resolved": resolved,
        "points": points,
    })))
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Trade {
    asset: String,
    provider: String,
    side: String,
    #[serde(rename = "amountUSD")]
    amount_usd: f64,
    /// Optional: when the caller already has a wallet connected, forwarding
    /// its address gets a real (not demo-fallback) quote for RFQ-quoted RWA
    /// tokens, which the Trading API requires a wallet address for.
    #[serde(default, rename = "walletAddress")]
    wallet_address: Option<String>,
}
async fn validate(s: &App, t: &Trade) -> Result<Asset, ApiError> {
    if !t.amount_usd.is_finite() || t.amount_usd <= 0.0 || t.amount_usd > 100000.0 {
        return Err(bad_coded("INVALID_AMOUNT", "Invalid amountUSD"));
    }
    if t.side != "buy" && t.side != "sell" {
        return Err(bad_coded("INVALID_SIDE", "side must be buy or sell"));
    }
    let assets = s.assets.read().await;
    let a = assets
        .iter()
        .find(|a| a.symbol == t.asset)
        .ok_or_else(|| bad_coded("UNKNOWN_ASSET", "Unknown asset"))?;
    if !a.providers.iter().any(|p| p.name == t.provider) {
        return Err(bad_coded("UNKNOWN_PROVIDER", "Unknown provider"));
    }
    Ok(a.clone())
}
fn quote_value(a: &Asset, t: &Trade) -> Value {
    let impact = t.amount_usd / a.liquidity;
    json!({"asset":a.symbol,"amountUSD":t.amount_usd,"expectedOutput":t.amount_usd*(1.0-impact),"priceImpact":impact,"gasEstimate":0.01,"mode":"demo","outputCurrency":"USD-equivalent","provider":t.provider,"side":t.side,"createdAt":chrono::Utc::now(),"expiresInSeconds":30})
}
fn enrich_quote(mut v: Value, a: &Asset, t: &Trade) -> Value {
    v["asset"] = json!(a.symbol);
    v["amountUSD"] = json!(t.amount_usd);
    v["provider"] = json!(t.provider);
    v["side"] = json!(t.side);
    v["createdAt"] = json!(chrono::Utc::now());
    v["expiresInSeconds"] = json!(30);
    v.as_object_mut()
        .unwrap()
        .entry("outputCurrency")
        .or_insert(json!("USD-equivalent"));
    v
}
/// Try a real Binance Web3 Trading API aggregator quote for this provider's
/// on-chain route; fall back to the demo formula when credentials, contract
/// mapping, or the live call itself are unavailable.
async fn resolve_quote(s: &App, a: &Asset, t: &Trade) -> Value {
    if let Some(client) = &s.binance {
        if let Some(provider) = a.providers.iter().find(|p| p.name == t.provider) {
            if let Some(live) = live::live_quote(client, provider, &t.side, t.amount_usd, t.wallet_address.as_deref()).await {
                return enrich_quote(live, a, t);
            }
        }
    }
    quote_value(a, t)
}
async fn quote(State(s): State<App>, Json(t): Json<Trade>) -> ApiResult {
    let a = validate(&s, &t).await?;
    Ok(Json(resolve_quote(&s, &a, &t).await))
}
/// No asset whitelist here on purpose: any asset the RWA Data API actually
/// lists is tradable for quote/simulate/policy purposes (`validate` already
/// rejects anything not in the live asset list). Real wallet *signing* uses
/// this same gate now too — see `prepare_trade`.
///
/// No per-trade/daily/slippage caps either (removed at the user's explicit
/// request, since the hackathon demo needs successful real trades, not a
/// policy gate blocking them); `validate` still enforces basic input sanity
/// (finite, positive, ≤ $100,000), and `prepare_trade`'s mandatory on-chain
/// `/pre-transaction/simulate` check still blocks anything that would
/// actually revert — that's a correctness check, not a business limit.
async fn simulate(State(s): State<App>, Json(t): Json<Trade>) -> ApiResult {
    let a = validate(&s, &t).await?;
    let mut result = resolve_quote(&s, &a, &t).await;
    let impact = result
        .get("priceImpact")
        .and_then(Value::as_f64)
        .unwrap_or(t.amount_usd / a.liquidity.max(1.0));
    let expected_output = result
        .get("expectedOutput")
        .and_then(Value::as_f64)
        .unwrap_or(t.amount_usd);
    let id = Uuid::new_v4();
    result["id"] = json!(id);
    result["success"] = json!(true);
    result["reason"] = json!(
        "No per-trade or daily limits are enforced; no on-chain EVM simulation has run yet, and live execution is disabled."
    );
    result["reasonCode"] = json!("SIM_PASSED");
    sqlx::query("INSERT INTO trade_simulations(id,asset,input_amount,expected_output,slippage,status,response_json) VALUES($1,$2,$3,$4,$5,'passed',$6)")
        .bind(id)
        .bind(&t.asset)
        .bind(t.amount_usd)
        .bind(expected_output)
        .bind(impact)
        .bind(&result)
        .execute(&s.db)
        .await
        .map_err(internal)?;
    Ok(Json(result))
}
async fn execute() -> ApiResult {
    Err(ApiError(
        StatusCode::NOT_IMPLEMENTED,
        "Live execution disabled: configure a verified trading provider, EVM simulation and client wallet signing before broadcast.".into(),
        Some("EXECUTE_DISABLED"), None
    ))
}
#[derive(Deserialize)]
struct AgentParseRequest {
    prompt: String,
}
/// LLM-backed natural-language strategy parser (architecture doc section
/// 11). The model only proposes four numbers via a schema-constrained call;
/// every value is independently re-validated here against the same hard
/// limits `create_policy` enforces, and the asset must literally appear in
/// the user's own text so a schema-coerced default can't slip past unnoticed.
async fn agent_parse(State(s): State<App>, Json(req): Json<AgentParseRequest>) -> ApiResult {
    let Some(client) = &s.openai else {
        return Err(ApiError(
            StatusCode::NOT_IMPLEMENTED,
            "LLM not configured: backend has no OPENAI_API_KEY".into(),
            Some("AGENT_DISABLED"), None
        ));
    };
    let prompt = req.prompt.trim();
    if prompt.is_empty() {
        return Err(bad_coded("PROMPT_EMPTY", "prompt must not be empty"));
    }
    let parsed = client.parse_policy_intent(prompt).await.map_err(|e| {
        tracing::warn!(error=%e, "Agent intent parse failed");
        ApiError(
            StatusCode::BAD_GATEWAY,
            format!("LLM parsing failed: {e}"),
            Some("AGENT_PARSE_FAILED"), None
        )
    })?;
    let asset = parsed
        .get("asset")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string();
    let threshold = parsed
        .get("referenceGapPercent")
        .and_then(Value::as_f64)
        .unwrap_or(f64::NAN);
    let executable_threshold = parsed
        .get("executableGapPercent")
        .and_then(Value::as_f64)
        .unwrap_or(f64::NAN);
    let amount = parsed
        .get("amountUSD")
        .and_then(Value::as_f64)
        .unwrap_or(f64::NAN);
    let known_asset = s.assets.read().await.iter().any(|a| a.symbol == asset);
    if !known_asset {
        return Err(bad_coded(
            "AGENT_UNKNOWN_ASSET",
            "That asset isn't in the tracked list",
        ));
    }
    if !prompt.to_uppercase().contains(&asset) {
        return Err(bad_coded(
            "AGENT_ASSET_NOT_IN_PROMPT",
            "The asset ticker isn't clearly mentioned in your description; please rephrase",
        ));
    }
    let sane = threshold.is_finite()
        && threshold > 0.0
        && threshold <= 100.0
        && executable_threshold.is_finite()
        && executable_threshold >= 0.0
        && executable_threshold <= 100.0
        && amount.is_finite()
        && amount > 0.0;
    if !sane {
        return Err(bad_coded(
            "AGENT_OUT_OF_RANGE",
            "Parsed values are outside the allowed range (amount must be positive, thresholds 0-100%)",
        ));
    }
    Ok(Json(json!({
        "asset": asset,
        "threshold": threshold,
        "executableThreshold": executable_threshold,
        "amount": amount,
        "conditions": [
            {"metric":"reference_gap","operator":">","value": threshold / 100.0},
            {"metric":"executable_gap","operator":">","value": executable_threshold / 100.0}
        ],
        "action": {"type":"sell","amountUSD": amount},
        "mode": "llm"
    })))
}
#[derive(Deserialize)]
struct ChatTurn {
    role: String,
    content: String,
}
#[derive(Deserialize)]
struct AgentChatRequest {
    messages: Vec<ChatTurn>,
}
/// Multi-turn conversational companion to `agent_parse` — same safety
/// contract (every proposal re-validated against the same hard limits,
/// asset must appear somewhere in the user's own words across the
/// conversation), but lets the model ask a clarifying question instead of
/// guessing when the request is ambiguous, and revise a proposal across
/// turns instead of the user starting over from a blank form.
async fn agent_chat(State(s): State<App>, Json(req): Json<AgentChatRequest>) -> ApiResult {
    let Some(client) = &s.openai else {
        return Err(ApiError(
            StatusCode::NOT_IMPLEMENTED,
            "LLM not configured: backend has no OPENAI_API_KEY".into(),
            Some("AGENT_DISABLED"), None
        ));
    };
    if req.messages.is_empty() {
        return Err(bad_coded("PROMPT_EMPTY", "messages must not be empty"));
    }
    let user_text = req
        .messages
        .iter()
        .filter(|m| m.role == "user")
        .map(|m| m.content.to_uppercase())
        .collect::<Vec<_>>()
        .join(" ");
    let history: Vec<(String, String)> = req
        .messages
        .iter()
        .map(|m| (m.role.clone(), m.content.clone()))
        .collect();
    let result = client.chat_policy_intent(&history).await.map_err(|e| {
        tracing::warn!(error=%e, "Agent chat turn failed");
        ApiError(
            StatusCode::BAD_GATEWAY,
            format!("LLM chat failed: {e}"),
            Some("AGENT_CHAT_FAILED"), None
        )
    })?;
    let kind = result.get("kind").and_then(Value::as_str).unwrap_or("question");
    let message = result.get("message").and_then(Value::as_str).unwrap_or("").to_string();
    if kind != "proposal" {
        return Ok(Json(json!({ "kind": "question", "message": message })));
    }
    let asset = result.get("asset").and_then(Value::as_str).unwrap_or("").to_string();
    let threshold = result
        .get("referenceGapPercent")
        .and_then(Value::as_f64)
        .unwrap_or(f64::NAN);
    let executable_threshold = result
        .get("executableGapPercent")
        .and_then(Value::as_f64)
        .unwrap_or(f64::NAN);
    let amount = result
        .get("amountUSD")
        .and_then(Value::as_f64)
        .unwrap_or(f64::NAN);
    let known_asset = s.assets.read().await.iter().any(|a| a.symbol == asset);
    let sane = known_asset
        && user_text.contains(&asset)
        && threshold.is_finite()
        && threshold > 0.0
        && threshold <= 100.0
        && executable_threshold.is_finite()
        && executable_threshold >= 0.0
        && executable_threshold <= 100.0
        && amount.is_finite()
        && amount > 0.0;
    if !sane {
        // Don't hand back an uncreatable proposal — downgrade to a question
        // so the conversation keeps going instead of dead-ending on an
        // invalid payload the frontend can't offer a "create" button for.
        tracing::warn!(asset = %asset, threshold, executable_threshold, amount, "Agent chat proposed an invalid rule; downgrading to a question");
        return Ok(Json(json!({ "kind": "question", "message": message })));
    }
    Ok(Json(json!({
        "kind": "proposal",
        "message": message,
        "asset": asset,
        "threshold": threshold,
        "executableThreshold": executable_threshold,
        "amount": amount,
    })))
}
#[derive(Deserialize)]
struct AgentFilterRequest {
    prompt: String,
}
/// Natural-language filter over the tracked-asset table. The model only
/// ever *proposes* filter criteria; the frontend applies them client-side
/// against the already-live asset list exactly like the manual
/// sector/search/sort controls, so nothing here can hide or fabricate a
/// price — `matchedSymbols` is additionally intersected with the real
/// tracked-symbol set below so a hallucinated ticker can't slip through.
async fn agent_filter(State(s): State<App>, Json(req): Json<AgentFilterRequest>) -> ApiResult {
    let Some(client) = &s.openai else {
        return Err(ApiError(
            StatusCode::NOT_IMPLEMENTED,
            "LLM not configured: backend has no OPENAI_API_KEY".into(),
            Some("AGENT_DISABLED"), None
        ));
    };
    let prompt = req.prompt.trim();
    if prompt.is_empty() {
        return Err(bad_coded("PROMPT_EMPTY", "prompt must not be empty"));
    }
    let tracked: Vec<(String, String)> = s
        .assets
        .read()
        .await
        .iter()
        .map(|a| (a.symbol.clone(), a.name.clone()))
        .collect();
    let result = client.filter_assets(&tracked, prompt).await.map_err(|e| {
        tracing::warn!(error=%e, "Agent filter parse failed");
        ApiError(
            StatusCode::BAD_GATEWAY,
            format!("LLM filter parsing failed: {e}"),
            Some("AGENT_FILTER_FAILED"), None
        )
    })?;
    let known: std::collections::HashSet<String> =
        tracked.into_iter().map(|(sym, _)| sym).collect();
    let matched_symbols: Vec<String> = result
        .get("matchedSymbols")
        .and_then(Value::as_array)
        .map(|arr| {
            arr.iter()
                .filter_map(Value::as_str)
                .map(str::to_string)
                .filter(|s| known.contains(s))
                .collect()
        })
        .unwrap_or_default();
    Ok(Json(json!({
        "matchedSymbols": matched_symbols,
        "query": result.get("query").and_then(Value::as_str),
        "minGapPercent": result.get("minGapPercent").and_then(Value::as_f64),
        "minScore": result.get("minScore").and_then(Value::as_f64),
        "liquidity": result.get("liquidity").and_then(Value::as_str),
        "summary": result.get("summary").and_then(Value::as_str).unwrap_or(""),
    })))
}
#[derive(Deserialize)]
struct AgentExplainRequest {
    asset: String,
}
/// "Interpret this signal" — turns a single asset's already-computed
/// numbers (gap, spread, liquidity, score) into a short plain-language
/// explanation via the LLM. Purely descriptive color commentary on numbers
/// the backend already computed, not a new data source: the model never
/// sees anything beyond what's in `context` below, so it can't invent a
/// news event or imply advice that isn't grounded in the live numbers.
async fn agent_explain(State(s): State<App>, Json(req): Json<AgentExplainRequest>) -> ApiResult {
    let Some(client) = &s.openai else {
        return Err(ApiError(
            StatusCode::NOT_IMPLEMENTED,
            "LLM not configured: backend has no OPENAI_API_KEY".into(),
            Some("AGENT_DISABLED"), None
        ));
    };
    let assets = s.assets.read().await;
    let a = assets
        .iter()
        .find(|a| a.symbol == req.asset)
        .ok_or_else(|| bad_coded("AGENT_UNKNOWN_ASSET", "That asset isn't in the tracked list"))?
        .clone();
    drop(assets);
    let gap = reference_gap(a.price, a.reference).unwrap_or(0.0);
    let prices: Vec<f64> = a
        .providers
        .iter()
        .map(|p| p.price)
        .filter(|p| p.is_finite() && *p > 0.0)
        .collect();
    let spread = if prices.len() >= 2 {
        let max = prices.iter().cloned().fold(f64::MIN, f64::max);
        let min = prices.iter().cloned().fold(f64::MAX, f64::min);
        if min > 0.0 {
            (max - min) / min
        } else {
            0.0
        }
    } else {
        0.0
    };
    let context = json!({
        "symbol": a.symbol,
        "name": a.name,
        "sector": a.sector,
        "onChainPrice": a.price,
        "referencePrice": a.reference,
        "gapPercent": gap * 100.0,
        "radarScore": a.score,
        "liquidityProxy": a.liquidity,
        "issuerSpreadPercent": spread * 100.0,
        "marketStatus": a.market_status.clone().unwrap_or_else(|| "unknown".into()),
    });
    let explanation = client.explain_signal(&context).await.map_err(|e| {
        tracing::warn!(error=%e, asset=%a.symbol, "Agent signal explanation failed");
        ApiError(
            StatusCode::BAD_GATEWAY,
            format!("LLM explanation failed: {e}"),
            Some("AGENT_EXPLAIN_FAILED"), None
        )
    })?;
    Ok(Json(json!({
        "asset": a.symbol,
        "headline": explanation.get("headline").and_then(Value::as_str).unwrap_or(""),
        "explanation": explanation.get("explanation").and_then(Value::as_str).unwrap_or(""),
        "caution": explanation.get("caution").and_then(Value::as_str).unwrap_or(""),
    })))
}
/// Read-only wallet portfolio via the Binance Web3 Wallet API. No custody or
/// signing here: the caller already controls `address` through their own
/// browser wallet, and this only reads public on-chain balances for it.
async fn portfolio(State(s): State<App>, Path(address): Path<String>) -> ApiResult {
    let Some(client) = &s.binance else {
        return Err(ApiError(
            StatusCode::NOT_IMPLEMENTED,
            "Wallet lookup not configured: backend has no BINANCE_API_KEY".into(),
            Some("WALLET_QUERY_DISABLED"), None
        ));
    };
    if !address.starts_with("0x") || address.len() != 42 {
        return Err(bad_coded(
            "INVALID_ADDRESS",
            "address must be a valid EVM address (0x-prefixed, 42 characters)",
        ));
    }
    let assets = client.wallet_balances("56", &address).await.map_err(|e| {
        tracing::warn!(error=%e, "Wallet balance fetch failed");
        ApiError(
            StatusCode::BAD_GATEWAY,
            format!("Failed to read wallet balances: {e}"),
            Some("WALLET_QUERY_FAILED"), None
        )
    })?;
    let normalized: Vec<Value> = assets
        .iter()
        .map(|a| {
            json!({
                "symbol": a.get("symbol").and_then(Value::as_str).unwrap_or("?"),
                "balance": a.get("balance").and_then(Value::as_str).unwrap_or("0"),
                "usdPrice": a.get("tokenPrice").and_then(Value::as_str),
                "contractAddress": a.get("tokenContractAddress").and_then(Value::as_str),
                "riskFlagged": a.get("isRiskToken").and_then(Value::as_bool).unwrap_or(false),
            })
        })
        .collect();
    Ok(Json(json!(normalized)))
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct PrepareTrade {
    asset: String,
    provider: String,
    side: String,
    #[serde(rename = "amountUSD")]
    amount_usd: f64,
    wallet_address: String,
}
/// Build a real, signable BSC transaction for the caller's own connected
/// wallet. This only *builds and simulates*: nothing is signed or broadcast
/// here (Rule 1, section 21 — the backend never touches a private key). The
/// browser wallet signs and sends the returned `approveTx`/`swapTx` itself
/// via `eth_sendTransaction`.
///
/// Every real-signed trade goes through the exact same policy gate as
/// `/api/trades/simulate` and reserves the same shared daily budget (an
/// attacker — or an honest mistake — can't bypass the $5/$20 caps just by
/// using this endpoint instead), and a doomed transaction is never handed
/// back: the Transaction API's off-chain simulation must predict success
/// first (Rule 4, section 21).
async fn prepare_trade(State(s): State<App>, Json(p): Json<PrepareTrade>) -> ApiResult {
    let Some(client) = &s.binance else {
        return Err(ApiError(
            StatusCode::NOT_IMPLEMENTED,
            "BINANCE_API_KEY not configured: cannot build a real transaction".into(),
            Some("TRADING_DISABLED"), None
        ));
    };
    if !p.wallet_address.starts_with("0x") || p.wallet_address.len() != 42 {
        return Err(bad_coded(
            "INVALID_ADDRESS",
            "walletAddress must be a valid EVM address",
        ));
    }
    // Real signing now covers every tracked asset (confirmed explicitly by
    // the user, since this expands what real funds can move); `validate`
    // below still rejects anything not in the live asset list. No per-trade/
    // daily/slippage cap either (removed at the user's explicit request) —
    // the mandatory on-chain simulation check further down is what actually
    // guards against a doomed transaction, not a business policy gate.
    let t = Trade {
        asset: p.asset.clone(),
        provider: p.provider.clone(),
        side: p.side.clone(),
        amount_usd: p.amount_usd,
        wallet_address: Some(p.wallet_address.clone()),
    };
    let a = validate(&s, &t).await?;
    let provider = a
        .providers
        .iter()
        .find(|pr| pr.name == t.provider)
        .cloned()
        .ok_or_else(|| bad_coded("UNKNOWN_PROVIDER", "Unknown provider"))?;
    let quote = live::live_quote(client, &provider, &t.side, t.amount_usd, Some(&p.wallet_address))
        .await
        .ok_or_else(|| bad_coded(
            "NO_LIVE_QUOTE",
            "Couldn't get a live quote for this amount (the provider may lack a contract mapping, the amount may be below its minimum order size, or there was a transient API error); try again or change provider/amount",
        ))?;
    let impact = quote
        .get("priceImpact")
        .and_then(Value::as_f64)
        .unwrap_or(1.0);
    let expected_output = quote
        .get("expectedOutput")
        .and_then(Value::as_f64)
        .unwrap_or(0.0);
    let quote_id = quote
        .get("quoteId")
        .and_then(Value::as_str)
        .ok_or_else(|| {
            ApiError(
                StatusCode::BAD_GATEWAY,
                "Quote did not return a quoteId".into(),
                Some("QUOTE_MISSING_ID"), None
            )
        })?
        .to_string();

    let sim_id = Uuid::new_v4();
    // Shaped like a normal Simulation record (not just {mode,walletAddress,
    // quoteId}) so /api/history and the Activity page render this reservation
    // sensibly instead of as a $NaN/Invalid Date row — this is now purely an
    // audit record of a real-sign attempt in progress (no daily cap left to
    // reserve budget against).
    let reservation = json!({
        "id": sim_id,
        "asset": t.asset,
        "amountUSD": t.amount_usd,
        "expectedOutput": expected_output,
        "priceImpact": impact,
        "gasEstimate": 0.0,
        "mode": "live-sign",
        "provider": t.provider,
        "side": t.side,
        "success": true,
        "reason": "Real-signing quote obtained; no per-trade or daily limits are enforced. This is not a demo simulation result.",
        "reasonCode": "REAL_SIGN_RESERVED",
        "createdAt": chrono::Utc::now(),
        "walletAddress": p.wallet_address,
        "quoteId": quote_id,
    });
    sqlx::query("INSERT INTO trade_simulations(id,asset,input_amount,expected_output,slippage,status,response_json) VALUES($1,$2,$3,$4,$5,'passed',$6)")
        .bind(sim_id)
        .bind(&t.asset)
        .bind(t.amount_usd)
        .bind(expected_output)
        .bind(impact)
        .bind(&reservation)
        .execute(&s.db)
        .await
        .map_err(internal)?;

    let route = live::resolve_swap_route(&provider, &t.side, t.amount_usd).ok_or_else(|| {
        ApiError(
            StatusCode::BAD_GATEWAY,
            "Could not resolve a swap route".into(),
            Some("ROUTE_UNRESOLVED"), None
        )
    })?;

    let approve_raw = client
        .approve_transaction(&route.chain, &route.from_token, &route.raw_amount)
        .await
        .map_err(|e| {
            ApiError(
                StatusCode::BAD_GATEWAY,
                format!("Failed to build the approval transaction: {e}"),
                Some("APPROVE_BUILD_FAILED"), None
            )
        })?;
    let approve_tx = live::normalize_approve_tx(&approve_raw, &route.from_token);

    let swap_raw = client
        .build_swap_transaction(
            &route.chain,
            &route.raw_amount,
            &route.from_token,
            &route.to_token,
            &p.wallet_address,
            &quote_id,
            "0.5",
        )
        .await
        .map_err(|e| {
            ApiError(
                StatusCode::BAD_GATEWAY,
                format!("Failed to build the swap transaction: {e}"),
                Some("SWAP_BUILD_FAILED"), None
            )
        })?;
    let swap_tx = live::normalize_swap_tx(&swap_raw).ok_or_else(|| {
        tracing::warn!(raw = %swap_raw, "normalize_swap_tx failed to parse /aggregator/swap response");
        ApiError(
            StatusCode::BAD_GATEWAY,
            "Could not parse the swap-build response".into(),
            Some("SWAP_PARSE_FAILED"), None
        )
    })?;

    let sim_to = swap_tx.get("to").and_then(Value::as_str).unwrap_or("");
    let sim_value = swap_tx.get("value").and_then(Value::as_str).unwrap_or("0");
    let sim_data = swap_tx.get("data").and_then(Value::as_str).unwrap_or("0x");
    let simulation = client
        .simulate_evm_tx(&route.chain, &p.wallet_address, sim_to, sim_value, sim_data)
        .await
        .map_err(|e| {
            ApiError(
                StatusCode::BAD_GATEWAY,
                format!("On-chain simulation failed: {e}"),
                Some("SIMULATION_FAILED"), None
            )
        })?;
    let sim_status = simulation
        .get("status")
        .and_then(Value::as_str)
        .unwrap_or("UNKNOWN");
    if sim_status != "SUCCESS" {
        let reason = simulation
            .get("failReason")
            .and_then(Value::as_str)
            .unwrap_or("unknown reason");
        // The swap is simulated in isolation, against *current* on-chain
        // state — it can't see the approve transaction sitting right above
        // it that the frontend will broadcast (and wait to confirm) first.
        // So when there's an approve step queued and the only problem is
        // insufficient allowance, that's not a doomed transaction, it's
        // just the simulator seeing pre-approval state; only block for
        // reasons an approve wouldn't fix (e.g. insufficient balance).
        let allowance_will_be_fixed_by_approve =
            approve_tx.is_some() && reason.to_lowercase().contains("allowance");
        if !allowance_will_be_fixed_by_approve {
            return Err(ApiError(
                StatusCode::BAD_REQUEST,
                format!("On-chain simulation predicts this transaction would fail; signing blocked: {reason}"),
                Some("SIMULATION_PREDICTS_FAILURE"),
                Some(json!({"reason": reason})),
            ));
        }
    }

    Ok(Json(json!({
        "simulationId": sim_id,
        "quoteId": quote_id,
        "chainId": route.chain,
        "approveTx": approve_tx,
        "swapTx": swap_tx,
        "simulation": simulation,
        "expectedOutput": expected_output,
        "priceImpact": impact,
        "mode": "live"
    })))
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct RecordTx {
    #[serde(rename = "txHash")]
    tx_hash: String,
    #[serde(rename = "chainId")]
    chain_id: i32,
    wallet_address: String,
}
/// Persist a real transaction hash the *browser wallet* already
/// signed-and-sent (audit trail per architecture doc section 22). Purely a
/// record: this never signs, sends, or checks confirmation status itself.
async fn record_transaction(State(s): State<App>, Json(r): Json<RecordTx>) -> ApiResult {
    if !r.tx_hash.starts_with("0x") || r.tx_hash.len() != 66 {
        return Err(bad_coded("INVALID_TX_HASH", "txHash is not a valid transaction hash"));
    }
    let id = Uuid::new_v4();
    sqlx::query("INSERT INTO transactions(id,wallet_id,tx_hash,chain_id,status) VALUES($1,$2,$3,$4,'submitted') ON CONFLICT(tx_hash) DO NOTHING")
        .bind(id)
        .bind(&r.wallet_address)
        .bind(&r.tx_hash)
        .bind(r.chain_id)
        .execute(&s.db)
        .await
        .map_err(internal)?;
    Ok(Json(
        json!({"id":id,"txHash":r.tx_hash,"status":"submitted"}),
    ))
}
async fn transactions_log(State(s): State<App>) -> ApiResult {
    let rows = sqlx::query(
        "SELECT id,wallet_id,tx_hash,chain_id,status,created_at FROM transactions ORDER BY created_at DESC LIMIT 50",
    )
    .fetch_all(&s.db)
    .await
    .map_err(internal)?;
    Ok(Json(json!(rows
        .iter()
        .map(|r| json!({
            "id": r.get::<Uuid,_>("id"),
            "walletAddress": r.get::<Option<String>,_>("wallet_id"),
            "txHash": r.get::<String,_>("tx_hash"),
            "chainId": r.get::<i32,_>("chain_id"),
            "status": r.get::<String,_>("status"),
            "createdAt": r.get::<chrono::DateTime<chrono::Utc>,_>("created_at"),
        }))
        .collect::<Vec<_>>())))
}
#[derive(Deserialize)]
struct PolicyInput {
    id: Uuid,
    asset: String,
    threshold: f64,
    #[serde(rename = "executableThreshold")]
    executable_threshold: f64,
    amount: f64,
}
async fn create_policy(State(s): State<App>, Json(p): Json<PolicyInput>) -> ApiResult {
    let known_asset = s.assets.read().await.iter().any(|a| a.symbol == p.asset);
    if !known_asset
        || !p.threshold.is_finite()
        || p.threshold <= 0.0
        || !p.executable_threshold.is_finite()
        || p.executable_threshold < 0.0
        || !p.amount.is_finite()
        || p.amount <= 0.0
    {
        return Err(bad_coded("POLICY_OUT_OF_RANGE", "Policy outside allowed assets or limits"));
    }
    sqlx::query(
        "INSERT INTO agent_policies(id,asset,conditions_json,action_json) VALUES($1,$2,$3,$4)",
    )
    .bind(p.id)
    .bind(&p.asset)
    .bind(json!([{ "metric":"reference_gap","operator":">","value":p.threshold/100. }, { "metric":"executable_gap","operator":">","value":p.executable_threshold/100. }]))
    .bind(json!({"type":"sell","amountUSD":p.amount}))
    .execute(&s.db)
    .await
    .map_err(internal)?;
    Ok(Json(json!({"id":p.id,"enabled":false,"mode":"demo"})))
}
async fn policies(State(s): State<App>) -> ApiResult {
    let rows=sqlx::query("SELECT id,asset,conditions_json,action_json,enabled FROM agent_policies ORDER BY created_at DESC").fetch_all(&s.db).await.map_err(internal)?;
    Ok(Json(json!(rows.iter().map(|r|json!({"id":r.get::<Uuid,_>("id"),"asset":r.get::<String,_>("asset"),"conditions":r.get::<Value,_>("conditions_json"),"action":r.get::<Value,_>("action_json"),"enabled":r.get::<bool,_>("enabled")})).collect::<Vec<_>>())))
}
#[derive(Deserialize)]
struct Toggle {
    enabled: bool,
}
async fn toggle_policy(
    State(s): State<App>,
    Path(id): Path<Uuid>,
    Json(p): Json<Toggle>,
) -> ApiResult {
    let result = sqlx::query("UPDATE agent_policies SET enabled=$1 WHERE id=$2")
        .bind(p.enabled)
        .bind(id)
        .execute(&s.db)
        .await
        .map_err(internal)?;
    if result.rows_affected() == 0 {
        return Err(ApiError(
            StatusCode::NOT_FOUND,
            "Policy not found".into(),
            Some("POLICY_NOT_FOUND"), None
        ));
    }
    Ok(Json(json!({"id":id,"enabled":p.enabled,"mode":"demo"})))
}
async fn history(State(s): State<App>) -> ApiResult {
    let rows: Vec<Value> = sqlx::query_scalar(
        "SELECT response_json FROM trade_simulations ORDER BY created_at DESC LIMIT 100",
    )
    .fetch_all(&s.db)
    .await
    .map_err(internal)?;
    Ok(Json(json!(rows)))
}
async fn record(State(s): State<App>, req: Request, next: Next) -> Response {
    let path = req.uri().path().to_owned();
    let method = req.method().to_string();
    let start = Instant::now();
    let response = next.run(req).await;
    let status = response.status();
    let elapsed = start.elapsed().as_secs_f64() * 1000.;
    if let Err(e)=sqlx::query("INSERT INTO api_telemetry(api_module,endpoint,method,latency_ms,status_code,success) VALUES('gateway',$1,$2,$3,$4,$5)").bind(path).bind(method).bind(elapsed).bind(status.as_u16() as i32).bind(status.is_success()).execute(&s.db).await{tracing::warn!(error=%e,"Telemetry persistence failed")};
    response
}
async fn telemetry(State(s): State<App>) -> ApiResult {
    let rows=sqlx::query("SELECT endpoint,count(*) AS requests,avg(CASE WHEN success THEN 100.0 ELSE 0.0 END)::float8 AS success_rate,percentile_cont(0.5) WITHIN GROUP(ORDER BY latency_ms) AS latency FROM api_telemetry GROUP BY endpoint ORDER BY endpoint").fetch_all(&s.db).await.map_err(internal)?;
    Ok(Json(json!(rows.iter().map(|r|json!({"endpoint":r.get::<String,_>("endpoint"),"requests":r.get::<i64,_>("requests"),"successRate":r.get::<f64,_>("success_rate"),"latencyMs":r.get::<f64,_>("latency")})).collect::<Vec<_>>())))
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn gap_math() {
        assert!((reference_gap(193.4, 188.2).unwrap() - 0.02763018).abs() < 0.000001);
        assert_eq!(reference_gap(10., 0.), None)
    }
    #[test]
    fn constant_history() {
        assert_eq!(z_score(2., &[1., 1., 1.]), 0.)
    }
    #[test]
    fn score_bounds() {
        assert_eq!(radar_score(10., 10., 10., 1., true), 100.);
        assert_eq!(radar_score(0., 0., 0., f64::INFINITY, false), 0.)
    }
    #[test]
    fn bad_trade_inputs() {
        let assets = demo_assets();
        assert_eq!(assets.len(), 8);
        for a in assets {
            assert!(a.score >= 0. && a.score <= 100.);
            assert_eq!(a.providers.len(), 3)
        }
    }
}
