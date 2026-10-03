//! Optional LLM-backed natural-language policy parser (architecture doc
//! section 11, "Natural Language Strategy DSL"), via the OpenAI Responses
//! API. This is a proposal layer only: the LLM extracts four numbers from
//! free text, and `main.rs`'s `/api/agent/parse` handler re-validates every
//! one against the same hard limits `create_policy` already enforces before
//! returning anything to the client — the model never controls a
//! transaction directly (section 21, Rule 2).
//!
//! Falls back entirely to the frontend's rule-based parser when
//! `OPENAI_API_KEY` is unset, via [`OpenAiClient::from_env`] returning
//! `None`.

use serde_json::{json, Value};
use std::fmt;
use std::time::Duration;

const DEFAULT_HOST: &str = "https://api.openai.com/v1/responses";
const DEFAULT_MODEL: &str = "gpt-6-astra";

const SYSTEM_PROMPT: &str = "You convert a user's natural-language trading-alert request (English or Traditional Chinese) into a strict trigger for RWA Radar, a tokenized-stock anomaly monitor. \
asset: the stock ticker symbol the user is referring to, in uppercase exactly as it would appear on a US exchange (e.g. NVDA, AAPL, TSLA). If they name a company rather than a ticker, convert it to the ticker you're confident it maps to. \
referenceGapPercent: how far the on-chain token price must exceed the off-chain reference stock price, as a percent (e.g. 3 for 3%). \
executableGapPercent: how far the executable/on-chain premium after slippage must exceed reference, as a percent. \
amountUSD: the USD size of a single sell that fires when triggered. \
Extract only these four values from what the user actually wrote; do not invent conditions, and do not exceed what a reasonable reading of the text supports. The caller independently validates the ticker against its own tracked-asset list, so if you're unsure just return your best reading of what the user typed.";

const EXPLAIN_SYSTEM_PROMPT: &str = "You write short, engaging, plain-language color commentary on a single tokenized-stock anomaly signal from RWA Radar, for a retail user who isn't a quant. \
You'll receive a JSON object with the asset's on-chain price, off-chain reference price, the percent gap between them, a 0-100 radar score, a liquidity proxy, the spread between issuers quoting this asset, and whether the underlying market is currently open or closed. \
Use ONLY the numbers given — never invent a news event, earnings report, or other real-world cause you don't actually know happened. Ground your reasoning in plausible market-structure explanations instead: thin after-hours liquidity, a stale or delayed reference price, cross-exchange arbitrage lag, wide issuer spreads suggesting one quote is out of sync. \
Keep the tone conversational and a little punchy, never robotic. Never tell the user to buy, sell, or that something is a 'good opportunity' — you're explaining what the numbers probably mean, not giving investment advice. If the gap looks more like a data or liquidity artifact than a real tradable premium (e.g. the market is closed, liquidity is very thin, or the gap is implausibly large), say so plainly in the caution field; otherwise return an empty string for caution.";

const CHAT_SYSTEM_PROMPT: &str = "You're having a short back-and-forth (English or Traditional Chinese) with a user who wants to set up a trading-alert rule for RWA Radar, a tokenized-stock anomaly monitor — instead of making them fill out a form, you help them think it through. \
The rule has four parts: asset (an uppercase US exchange ticker), referenceGapPercent (how far the on-chain price must exceed the off-chain reference price, as a percent), executableGapPercent (how far the executable/on-chain premium after slippage must exceed reference, as a percent), and amountUSD (the USD size of the sell that fires when triggered). \
On each turn, decide: if you don't yet have enough to state a complete, unambiguous rule, set kind='question' and ask ONE short clarifying question in `message` (don't ask about parts the user already specified or that have an obvious sane default) — leave the four rule fields null. \
Once you have enough, set kind='proposal': write a brief, plain-language restatement of the full rule in `message` (e.g. 'Got it — alert when NVDA's on-chain premium clears 3% and the executable premium holds above 2%, sized at $5.'), and fill in all four fields with your best values. \
If the user responds to a proposal asking for a change, revise and propose again (kind='proposal') with the updated numbers rather than re-asking a question, unless their new message introduces genuine new ambiguity. \
Never invent an asset, condition, or number the user didn't imply; if they only gave a company name, convert it to the ticker you're confident it maps to.";

const FILTER_SYSTEM_PROMPT: &str = "You translate a user's plain-language description (English or Traditional Chinese) of the tokenized-stock assets they want to see into a structured filter for RWA Radar's asset table. \
You'll receive the full list of currently tracked `TICKER: Company Name` pairs as the user message, followed by their description on its own line prefixed `Request:`. \
matchedSymbols: if the user described an industry, theme, or group (e.g. 'semiconductor stocks', 'airlines', 'big banks'), use your own knowledge of these companies to return every ticker from the provided list whose company clearly belongs to that group; return an empty array if the request isn't about a specific industry/theme, or the user named one specific company (use `query` for that instead). \
query: a literal ticker or company-name substring if the user named one specific company/ticker; null otherwise. \
minGapPercent: the minimum ABSOLUTE reference-gap percent they asked for (e.g. 50 for 'premium over 50%' or 'gap bigger than 50%'); null if not mentioned. \
minScore: the minimum 0-100 radar score they asked for (e.g. 'score above 70'); null if not mentioned. \
liquidity: 'low' if they asked for thin/low/illiquid assets, 'high' if they asked for liquid/deep/high-volume assets, null if not mentioned — never guess a number, liquidity scale isn't something you can calibrate. \
summary: restate what you're filtering to in under 12 words, in the SAME language the user wrote their request in. \
Only set fields the request actually supports; leave everything else null/empty rather than guessing.";

#[derive(Debug)]
pub enum AgentError {
    Transport(String),
    Refused(String),
    Decode(String),
}
impl fmt::Display for AgentError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            AgentError::Transport(e) => write!(f, "transport error: {e}"),
            AgentError::Refused(e) => write!(f, "model refused: {e}"),
            AgentError::Decode(e) => write!(f, "decode error: {e}"),
        }
    }
}
impl std::error::Error for AgentError {}

#[derive(Clone)]
pub struct OpenAiClient {
    http: reqwest::Client,
    api_key: String,
    model: String,
    host: String,
}

impl OpenAiClient {
    pub fn from_env() -> Option<Self> {
        let api_key = std::env::var("OPENAI_API_KEY")
            .ok()
            .filter(|s| !s.trim().is_empty())?;
        let model = std::env::var("OPENAI_MODEL")
            .ok()
            .filter(|s| !s.trim().is_empty())
            .unwrap_or_else(|| DEFAULT_MODEL.into());
        let host = std::env::var("OPENAI_API_HOST")
            .ok()
            .filter(|s| !s.trim().is_empty())
            .unwrap_or_else(|| DEFAULT_HOST.into());
        let http = reqwest::Client::builder()
            .timeout(Duration::from_secs(20))
            .build()
            .ok()?;
        Some(Self {
            http,
            api_key,
            model,
            host,
        })
    }

    /// Extract `{asset, referenceGapPercent, executableGapPercent, amountUSD}`
    /// from free text via Structured Outputs. Returns the raw parsed JSON;
    /// the caller (main.rs) still owns all safety validation.
    pub async fn parse_policy_intent(&self, prompt: &str) -> Result<Value, AgentError> {
        let schema = json!({
            "type": "object",
            "properties": {
                "asset": {"type": "string", "description": "Uppercase US exchange ticker symbol"},
                "referenceGapPercent": {"type": "number"},
                "executableGapPercent": {"type": "number"},
                "amountUSD": {"type": "number"}
            },
            "required": ["asset", "referenceGapPercent", "executableGapPercent", "amountUSD"],
            "additionalProperties": false
        });
        let input = vec![
            json!({"role": "system", "content": SYSTEM_PROMPT}),
            json!({"role": "user", "content": prompt}),
        ];
        self.structured_call(input, "policy_intent", schema).await
    }

    /// Multi-turn version of `parse_policy_intent`: instead of one-shot
    /// extraction, the model can ask a clarifying question before
    /// committing to a proposal, and revise a proposal across turns. The
    /// caller supplies the full `(role, content)` history so far (no
    /// server-side session state); `main.rs` still re-validates any
    /// `kind:"proposal"` response the same way it validates the one-shot
    /// parser's output before it's ever usable to create a policy.
    pub async fn chat_policy_intent(&self, history: &[(String, String)]) -> Result<Value, AgentError> {
        let schema = json!({
            "type": "object",
            "properties": {
                "kind": {"type": "string", "enum": ["question", "proposal"]},
                "message": {"type": "string", "description": "The clarifying question, or a plain-language restatement of the proposed rule"},
                "asset": {"type": ["string", "null"], "description": "Uppercase US exchange ticker symbol; null while still kind=question"},
                "referenceGapPercent": {"type": ["number", "null"]},
                "executableGapPercent": {"type": ["number", "null"]},
                "amountUSD": {"type": ["number", "null"]}
            },
            "required": ["kind", "message", "asset", "referenceGapPercent", "executableGapPercent", "amountUSD"],
            "additionalProperties": false
        });
        let mut input = vec![json!({"role": "system", "content": CHAT_SYSTEM_PROMPT})];
        input.extend(
            history
                .iter()
                .map(|(role, content)| json!({"role": role, "content": content})),
        );
        self.structured_call(input, "policy_chat_turn", schema)
            .await
    }

    /// Translate a free-text description of desired assets into a
    /// structured filter over the tracked-asset list ("natural-language
    /// filter" feature). `tracked` is rendered as `TICKER: Name` lines so
    /// the model can map an industry/theme description (which isn't a
    /// field in our own data) onto real tickers using its own knowledge of
    /// what those companies are.
    pub async fn filter_assets(
        &self,
        tracked: &[(String, String)],
        request: &str,
    ) -> Result<Value, AgentError> {
        let schema = json!({
            "type": "object",
            "properties": {
                "matchedSymbols": {"type": "array", "items": {"type": "string"}},
                "query": {"type": ["string", "null"]},
                "minGapPercent": {"type": ["number", "null"]},
                "minScore": {"type": ["number", "null"]},
                "liquidity": {"type": ["string", "null"], "enum": ["low", "high", null]},
                "summary": {"type": "string"}
            },
            "required": ["matchedSymbols", "query", "minGapPercent", "minScore", "liquidity", "summary"],
            "additionalProperties": false
        });
        let list = tracked
            .iter()
            .map(|(sym, name)| format!("{sym}: {name}"))
            .collect::<Vec<_>>()
            .join("\n");
        let user_content = format!("{list}\n\nRequest: {request}");
        let input = vec![
            json!({"role": "system", "content": FILTER_SYSTEM_PROMPT}),
            json!({"role": "user", "content": user_content}),
        ];
        self.structured_call(input, "asset_filter", schema).await
    }

    /// Turn an anomaly signal's already-computed numbers into a short,
    /// plain-language explanation for a non-quant user ("signal interpret"
    /// feature). Grounded entirely in the numbers passed in `context` — the
    /// model is explicitly told not to invent facts or give trade advice,
    /// since this is color commentary on a number that's already real, not
    /// a recommendation.
    pub async fn explain_signal(&self, context: &Value) -> Result<Value, AgentError> {
        let schema = json!({
            "type": "object",
            "properties": {
                "headline": {"type": "string", "description": "One punchy sentence capturing the key takeaway, written for a retail audience, no jargon"},
                "explanation": {"type": "string", "description": "1-3 plain-language sentences on why this gap likely exists, grounded only in the numbers provided — plausible market-structure reasoning (thin liquidity, stale reference, cross-exchange lag), not a definitive claim"},
                "caution": {"type": "string", "description": "A short caveat if this looks more like a data/liquidity artifact than a real tradable gap; empty string if nothing stands out"}
            },
            "required": ["headline", "explanation", "caution"],
            "additionalProperties": false
        });
        let input = vec![
            json!({"role": "system", "content": EXPLAIN_SYSTEM_PROMPT}),
            json!({"role": "user", "content": context.to_string()}),
        ];
        self.structured_call(input, "signal_explanation", schema)
            .await
    }

    async fn structured_call(
        &self,
        input: Vec<Value>,
        schema_name: &str,
        schema: Value,
    ) -> Result<Value, AgentError> {
        let body = json!({
            "model": self.model,
            "input": input,
            "text": {
                "format": {
                    "type": "json_schema",
                    "name": schema_name,
                    "strict": true,
                    "schema": schema
                }
            }
        });
        let resp = self
            .http
            .post(&self.host)
            .bearer_auth(&self.api_key)
            .json(&body)
            .send()
            .await
            .map_err(|e| AgentError::Transport(e.to_string()))?;
        let status = resp.status();
        let text = resp
            .text()
            .await
            .map_err(|e| AgentError::Transport(e.to_string()))?;
        if !status.is_success() {
            return Err(AgentError::Transport(format!("HTTP {status}: {text:.300}")));
        }
        let value: Value =
            serde_json::from_str(&text).map_err(|e| AgentError::Decode(e.to_string()))?;
        let output = value.get("output").and_then(Value::as_array).cloned().unwrap_or_default();
        let message = output
            .iter()
            .find(|o| o.get("type").and_then(Value::as_str) == Some("message"));
        let content = message
            .and_then(|m| m.get("content"))
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        for item in &content {
            match item.get("type").and_then(Value::as_str) {
                Some("refusal") => {
                    let msg = item
                        .get("refusal")
                        .and_then(Value::as_str)
                        .unwrap_or("model declined to answer")
                        .to_string();
                    return Err(AgentError::Refused(msg));
                }
                Some("output_text") => {
                    let text = item
                        .get("text")
                        .and_then(Value::as_str)
                        .ok_or_else(|| AgentError::Decode("missing output text".into()))?;
                    return serde_json::from_str(text)
                        .map_err(|e| AgentError::Decode(e.to_string()));
                }
                _ => {}
            }
        }
        Err(AgentError::Decode("no parsable message in response".into()))
    }
}
