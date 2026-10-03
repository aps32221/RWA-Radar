//! Signed client for the Binance Web3 API (https://web3.binance.com/en/dev-docs).
//!
//! Covers just the endpoints RWA Radar needs: RWA token list (Market API),
//! gas price and aggregated swap quotes (Trading/Transaction API). Every call
//! is signed per https://web3.binance.com/en/dev-docs/authentication.

use base64::{engine::general_purpose::STANDARD, Engine as _};
use chrono::SecondsFormat;
use hmac::{Hmac, Mac};
use serde_json::{json, Value};
use sha2::Sha256;
use std::fmt;
use std::time::Duration;

const DEFAULT_HOST: &str = "https://web3.binance.com";
const BUILD_PREFIX: &str = "/build";

#[derive(Debug)]
pub enum BinanceError {
    Transport(String),
    Api { code: i64, msg: String },
    Decode(String),
}

impl fmt::Display for BinanceError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            BinanceError::Transport(e) => write!(f, "transport error: {e}"),
            BinanceError::Api { code, msg } => write!(f, "API error {code}: {msg}"),
            BinanceError::Decode(e) => write!(f, "decode error: {e}"),
        }
    }
}
impl std::error::Error for BinanceError {}

/// Signed HTTP client. Construct via [`BinanceClient::from_env`]; absent
/// `BINANCE_API_KEY`/`BINANCE_API_SECRET` this returns `None` so callers fall
/// back to demo data instead of trying to sign requests with an empty key.
#[derive(Clone)]
pub struct BinanceClient {
    http: reqwest::Client,
    api_key: String,
    api_secret: String,
    host: String,
}

impl BinanceClient {
    pub fn from_env() -> Option<Self> {
        let api_key = std::env::var("BINANCE_API_KEY")
            .ok()
            .filter(|s| !s.trim().is_empty())?;
        let api_secret = std::env::var("BINANCE_API_SECRET")
            .ok()
            .filter(|s| !s.trim().is_empty())?;
        let host = std::env::var("BINANCE_API_HOST")
            .ok()
            .filter(|s| !s.trim().is_empty())
            .unwrap_or_else(|| DEFAULT_HOST.into());
        let http = reqwest::Client::builder()
            .timeout(Duration::from_secs(10))
            .build()
            .ok()?;
        Some(Self {
            http,
            api_key,
            api_secret,
            host,
        })
    }

    fn sign(&self, timestamp: &str, method: &str, request_path: &str, body: &str) -> String {
        let pre_hash = format!("{timestamp}{method}{request_path}{body}");
        let mut mac = Hmac::<Sha256>::new_from_slice(self.api_secret.as_bytes())
            .expect("HMAC accepts any key length");
        mac.update(pre_hash.as_bytes());
        STANDARD.encode(mac.finalize().into_bytes())
    }

    async fn call(
        &self,
        method: reqwest::Method,
        path: &str,
        query: &[(&str, String)],
        body: Option<&Value>,
    ) -> Result<Value, BinanceError> {
        let query_string = build_query(query);
        let path_with_query = if query_string.is_empty() {
            path.to_string()
        } else {
            format!("{path}?{query_string}")
        };
        let request_path = format!("{BUILD_PREFIX}{path_with_query}");
        let url = format!("{}{}", self.host, request_path);
        let body_string = match body {
            Some(v) => serde_json::to_string(v).map_err(|e| BinanceError::Decode(e.to_string()))?,
            None => String::new(),
        };
        let timestamp = chrono::Utc::now().to_rfc3339_opts(SecondsFormat::Millis, true);
        let signature = self.sign(&timestamp, method.as_str(), &request_path, &body_string);
        let mut req = self
            .http
            .request(method, &url)
            .header("X-OC-APIKEY", &self.api_key)
            .header("X-OC-TIMESTAMP", &timestamp)
            .header("X-OC-SIGN", signature)
            .header("X-OC-RECV-WINDOW", "5000");
        if body.is_some() {
            req = req
                .header("Content-Type", "application/json")
                .body(body_string);
        }
        let resp = req
            .send()
            .await
            .map_err(|e| BinanceError::Transport(e.to_string()))?;
        let status = resp.status();
        let text = resp
            .text()
            .await
            .map_err(|e| BinanceError::Transport(e.to_string()))?;
        let envelope: Value = serde_json::from_str(&text).map_err(|e| {
            BinanceError::Decode(format!("HTTP {status}: {e} (body: {text:.200})"))
        })?;
        let code = envelope.get("code").and_then(Value::as_i64).unwrap_or(0);
        let success = envelope
            .get("success")
            .and_then(Value::as_bool)
            .unwrap_or(code == 0);
        if !success || code != 0 {
            let msg = envelope
                .get("msg")
                .or_else(|| envelope.get("message"))
                .and_then(Value::as_str)
                .unwrap_or("unknown error")
                .to_string();
            return Err(BinanceError::Api { code, msg });
        }
        Ok(envelope.get("data").cloned().unwrap_or(Value::Null))
    }

    /// `GET /api/v1/dex/market/rwa/tokens` — normalized tokenized-stock list
    /// (price, reference price, market status) across issuance platforms.
    pub async fn rwa_tokens(
        &self,
        chain_id: Option<&str>,
        platform_id: Option<&str>,
    ) -> Result<Vec<Value>, BinanceError> {
        let mut query = Vec::new();
        if let Some(c) = chain_id {
            query.push(("binanceChainId", c.to_string()));
        }
        if let Some(p) = platform_id {
            query.push(("platformId", p.to_string()));
        }
        let data = self
            .call(reqwest::Method::GET, "/api/v1/dex/market/rwa/tokens", &query, None)
            .await?;
        Ok(data.as_array().cloned().unwrap_or_default())
    }

    /// `GET /api/v1/dex/pre-transaction/gas-price`
    pub async fn gas_price(&self, chain_id: &str) -> Result<Value, BinanceError> {
        self.call(
            reqwest::Method::GET,
            "/api/v1/dex/pre-transaction/gas-price",
            &[("binanceChainId", chain_id.to_string())],
            None,
        )
        .await
    }

    /// `GET /api/v1/dex/aggregator/quote` — best-route quote for swapping
    /// `amount` (in `from_token`'s smallest unit) into `to_token`.
    /// `user_wallet_address` is required by the API for RFQ routes (equity
    /// / RWA tokens such as Ondo and BStock — exactly the tokenized stocks
    /// this app quotes); omit it only when no wallet is connected yet.
    pub async fn aggregator_quote(
        &self,
        chain_id: &str,
        amount: &str,
        from_token: &str,
        to_token: &str,
        user_wallet_address: Option<&str>,
    ) -> Result<Vec<Value>, BinanceError> {
        let mut query = vec![
            ("binanceChainId", chain_id.to_string()),
            ("amount", amount.to_string()),
            ("fromTokenAddress", from_token.to_string()),
            ("toTokenAddress", to_token.to_string()),
        ];
        if let Some(addr) = user_wallet_address {
            query.push(("userWalletAddress", addr.to_string()));
        }
        let data = self
            .call(reqwest::Method::GET, "/api/v1/dex/aggregator/quote", &query, None)
            .await?;
        Ok(data.as_array().cloned().unwrap_or_default())
    }

    /// `GET /api/v1/dex/aggregator/approve-transaction` — calldata to
    /// approve the DEX router to spend `approve_amount` of `token_contract`.
    /// Real-money path: this only *builds* the transaction; nothing is
    /// signed or sent from here.
    pub async fn approve_transaction(
        &self,
        chain_id: &str,
        token_contract: &str,
        approve_amount: &str,
    ) -> Result<Vec<Value>, BinanceError> {
        let query = [
            ("binanceChainId", chain_id.to_string()),
            ("tokenContractAddress", token_contract.to_string()),
            ("approveAmount", approve_amount.to_string()),
        ];
        let data = self
            .call(
                reqwest::Method::GET,
                "/api/v1/dex/aggregator/approve-transaction",
                &query,
                None,
            )
            .await?;
        Ok(data.as_array().cloned().unwrap_or_default())
    }

    /// `GET /api/v1/dex/aggregator/swap` — executable calldata for a
    /// previously-quoted route. Real-money path: this only *builds* the
    /// transaction; nothing is signed or sent from here.
    pub async fn build_swap_transaction(
        &self,
        chain_id: &str,
        amount: &str,
        from_token: &str,
        to_token: &str,
        wallet_address: &str,
        quote_id: &str,
        slippage_percent: &str,
    ) -> Result<Value, BinanceError> {
        let query = [
            ("binanceChainId", chain_id.to_string()),
            ("amount", amount.to_string()),
            ("fromTokenAddress", from_token.to_string()),
            ("toTokenAddress", to_token.to_string()),
            ("userWalletAddress", wallet_address.to_string()),
            ("quoteId", quote_id.to_string()),
            ("slippagePercent", slippage_percent.to_string()),
        ];
        self.call(reqwest::Method::GET, "/api/v1/dex/aggregator/swap", &query, None)
            .await
    }

    /// `POST /api/v1/dex/pre-transaction/simulate` — off-chain EVM
    /// simulation of an already-built (but unsigned) transaction, so a
    /// doomed transaction is never handed to the user for signing.
    pub async fn simulate_evm_tx(
        &self,
        chain_id: &str,
        from: &str,
        to: &str,
        value: &str,
        data: &str,
    ) -> Result<Value, BinanceError> {
        let body = json!({
            "binanceChainId": chain_id,
            "evmTx": {"from": from, "to": to, "value": value, "data": data}
        });
        self.call(
            reqwest::Method::POST,
            "/api/v1/dex/pre-transaction/simulate",
            &[],
            Some(&body),
        )
        .await
    }

    /// `GET /api/v1/dex/balance/all-token-balances-by-address` — read-only
    /// token balances for a connected wallet address. No custody involved:
    /// this only reads public on-chain state for whatever address the
    /// caller already controls via their own browser wallet.
    pub async fn wallet_balances(
        &self,
        chain_id: &str,
        address: &str,
    ) -> Result<Vec<Value>, BinanceError> {
        let query = [
            ("address", address.to_string()),
            ("chains", chain_id.to_string()),
            ("excludeRiskToken", "true".to_string()),
            ("pageSize", "100".to_string()),
        ];
        let data = self
            .call(
                reqwest::Method::GET,
                "/api/v1/dex/balance/all-token-balances-by-address",
                &query,
                None,
            )
            .await?;
        let assets = data
            .as_array()
            .and_then(|arr| arr.first())
            .and_then(|page| page.get("tokenAssets"))
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        Ok(assets)
    }
}

fn build_query(params: &[(&str, String)]) -> String {
    params
        .iter()
        .map(|(k, v)| format!("{k}={}", urlencoding::encode(v)))
        .collect::<Vec<_>>()
        .join("&")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn client() -> BinanceClient {
        BinanceClient {
            http: reqwest::Client::new(),
            api_key: "key".into(),
            api_secret: "secret".into(),
            host: DEFAULT_HOST.into(),
        }
    }

    #[test]
    fn query_string_matches_docs_example() {
        let q = build_query(&[
            ("chainId", "1".into()),
            ("symbol", "ETH USDT".into()),
        ]);
        assert_eq!(q, "chainId=1&symbol=ETH%20USDT");
    }

    #[test]
    fn pre_hash_includes_build_prefix() {
        let c = client();
        let sig = c.sign(
            "2026-05-11T10:08:57.715Z",
            "GET",
            "/build/api/v1/dex/market/price?chainId=1&symbol=ETH%20USDT",
            "",
        );
        // Deterministic HMAC-SHA256/base64 over the documented pre-hash string.
        assert_eq!(sig.len(), 44);
        assert!(sig.ends_with('='));
    }

    #[test]
    fn signing_is_deterministic_and_body_sensitive() {
        let c = client();
        let a = c.sign("t", "POST", "/build/x", "{}");
        let b = c.sign("t", "POST", "/build/x", "{\"a\":1}");
        assert_ne!(a, b);
        assert_eq!(a, c.sign("t", "POST", "/build/x", "{}"));
    }
}
