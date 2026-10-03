# RWA Radar

[繁體中文版](README.md)

Tokenized-stock market radar MVP. React 19 + TypeScript + Vite, Rust/Axum + PostgreSQL, Nginx. The UI itself is Traditional-Chinese-first; this document describes the project in English.

The API gotchas, documentation gaps, and redesign suggestions hit during development are written up in [`docs/dx-report-en.md`](docs/dx-report-en.md) ([Chinese version](docs/dx-report.md)).

## Running it

```sh
npm ci
npm run dev
```

The frontend works without the backend — it clearly labels local demo data, and watchlist/strategies/simulation history live in `localStorage`.

Full container stack:

```sh
docker compose up --build -d
```

Open http://localhost:3000. Nginx serves the SPA and reverse-proxies `/api`. PostgreSQL uses a persistent volume and only binds to localhost:55432; the API defaults to localhost:8180 (adjustable in `compose.yaml` — inter-container traffic still always goes over the internal `api:8080`, unaffected by this). This is a single demo workspace with no login or multi-tenant isolation; don't expose it directly to the internet.

Local Rust development (PowerShell):

```powershell
docker compose up -d db
$env:DATABASE_URL='postgres://radar:radar_dev_only@localhost:55432/radar'
# optional: wire up the real Binance Web3 API (apply at https://web3.binance.com/en/dev-portal)
$env:BINANCE_API_KEY='...'
$env:BINANCE_API_SECRET='...'
# optional: enable LLM natural-language strategy parsing (apply at https://platform.openai.com)
$env:OPENAI_API_KEY='...'
cargo run --manifest-path backend/Cargo.toml
```

The API runs migrations on startup. If `BINANCE_API_KEY`/`BINANCE_API_SECRET` are set (see `.env.example`), both startup and the 60-second background worker switch to Binance Web3 API's RWA Data endpoint (`/api/v1/dex/market/rwa/tokens`) for real tokenized-stock prices and reference prices, normalized by underlying ticker and written to snapshots; if unset or the first fetch fails, it falls back to 8 demo tickers across 3 demo issuers. DB credentials can be overridden via `.env` — never commit real credentials or API keys.

## What's implemented

- Market overview, search, sector/issuer filters, score/gap sorting, watchlist, CSV export.
- Asset price chart, issuer comparison, reference-price gap, modeled liquidity curve.
- Rust-side reference-gap math, historical z-score, weighted radar anomaly score.
- Demo quoting and policy simulation; any tracked asset works (no asset whitelist). The per-trade/daily amount and slippage caps were removed entirely at explicit user request, leaving only basic input sanity checks (amount must be positive, ≤ $100,000).
- Constrained-format natural-language strategy parsing with an auditable DSL; strategies default to disabled.
- Persisted simulation history, API telemetry, median-latency stats.
- Responsive layout, empty/error states, Nginx SPA fallback.
- **Binance Web3 API integration (active once keys are set)**: a signing HTTP client (`backend/src/binance.rs`, HMAC-SHA256, `X-OC-*` headers, matching the official authentication spec); the RWA Data API pulls live price, reference price, and open/closed status across issuers (Ondo/bStocks), normalized to a single underlying ticker (`backend/src/live.rs`); `/api/trades/quote` and `/api/trades/simulate` switch to the Trading API's DEX aggregator for real executable quotes and price impact whenever a contract mapping is known, and use the Transaction API's live gas price to estimate gas cost; falls back to the demo formula automatically when no contract mapping exists or a call fails. `/api/health` reports whether the service is currently in `live` or `demo` mode.
- **LLM agent integration (active once `OPENAI_API_KEY` is set)**: all routed through OpenAI's Responses API with Structured Outputs (schema-constrained output, `backend/src/agent.rs`); the LLM only ever proposes, it never writes a strategy or controls a trade directly.
  - `POST /api/agent/parse`: parses a free-form Chinese/English strategy description in one shot into `{asset, referenceGapPercent, executableGapPercent, amountUSD}`. The backend re-validates every value against the exact same rules `create_policy` enforces (must be a tracked asset, amount must be positive, thresholds 0–100%), and requires the asset ticker to literally appear in the user's own text so the model can't quietly introduce a ticker that was never mentioned. The frontend (`src/App.tsx`'s `createPolicy`) falls back silently to the rule-based parser when the LLM isn't enabled; `/api/health`'s `agentEnabled` field lets the frontend show the right copy.
  - `POST /api/agent/chat`: the multi-turn version of `/api/agent/parse` — same fields and validation, except the model can ask one clarifying question first when it doesn't have enough (`kind:"question"`), and only proposes a complete rule once it does (`kind:"proposal"`); the user can also ask for a change to an existing proposal in plain language instead of refilling a form. The Strategies page's Build tab switches to this endpoint and renders it as a chat UI whenever `agentEnabled` is true.
  - `POST /api/agent/filter`: turns a plain-language description of the assets someone wants into filter criteria for the asset table. The request sends the LLM the full `ticker: company name` list of every currently tracked asset, so it can map a description (e.g. "semiconductor stocks") onto real tickers using its own knowledge of those companies; the returned `matchedSymbols` is intersected with the backend's actual tracked-symbol set to drop anything not really on the list. "Low"/"high" liquidity is deliberately never a number the LLM guesses — the frontend converts it using the 25th/75th percentile of the current live data instead.
  - `POST /api/agent/explain`: generates a one-line headline, 1–3 plain-language sentences, and an optional caution for a single asset's current anomaly signal (gap, radar score, liquidity, issuer spread, market status). The system prompt explicitly limits it to plausible market-structure reasoning from the numbers given (thin liquidity, a stale reference price, etc.) — it can't invent news or earnings, and it can't give buy/sell advice. The "Interpret this signal" link on the Markets page's Focus Signals cards calls this endpoint.
- **Real wallet connection and balances (read-only)**: the frontend connects via an injected browser wallet (e.g. MetaMask), auto-switching/adding BSC (chain 56); once connected, the Portfolio page calls `GET /api/portfolio/{address}` (`backend/src/binance.rs`'s `wallet_balances`, wrapping the Wallet API's `all-token-balances-by-address`) to show that address's real BSC token balances. This path is entirely read-only — the backend only queries public on-chain data, no signing or private keys involved.
- **Real wallet signing (BSC mainnet, spends real funds — only wired up after explicit user confirmation)**: the asset trade modal shows a separate, warning-colored "Sign & send real transaction" button whenever a wallet is connected and the app is in live mode, kept entirely apart from the demo "Get quote / Run simulation" controls above it so it can't be triggered by accident. Originally limited to NVDA/TSLA, later expanded to every tracked asset at explicit user request; the per-trade/daily amount and slippage caps were later removed entirely, also at explicit request (`policy_allowed`/`policy_block_reason` have been deleted), leaving only `validate`'s basic input sanity checks. The flow (`POST /api/trades/prepare`, `backend/src/main.rs`): ① get a fresh Trading API quote; ② write an audit-only simulation record (no more daily-budget bookkeeping, it's purely a log of this real-signing attempt); ③ use the Trading API's `approve-transaction`/`swap` to build unsigned ERC-20 approval and swap calldata (both carry Binance's own live `gasPrice` as-is, rather than letting the wallet estimate it — that field was deliberately stripped once, and a real submitted transaction got stuck because the wallet's own default gas price was too low for the moment and eventually got auto-cancelled by the wallet itself; forwarding it again made things noticeably more reliable); ④ run the Transaction API's `pre-transaction/simulate` on-chain simulation, and only hand the transaction back to the frontend if it predicts success (`status:"SUCCESS"`). One exception: if the only reason the simulation failed is insufficient allowance, and there's genuinely an approve transaction queued ahead of this one (meaning that allowance is about to be granted), it's not treated as fatal — the simulator only ever sees one transaction at a time, so it has no way to know the approve sitting right before it is about to change on-chain state. Every other failure reason (including an actual balance shortfall) is still blocked exactly as before. The backend **never touches a private key** anywhere in this flow: the frontend hands the unsigned transactions to the connected browser wallet via `eth_sendTransaction` (approval first, wait for confirmation, then the swap), and only after sending does `POST /api/trades/record` log the real tx hash into the `transactions` table (a read-only audit record — it doesn't affect the signing itself). `GET /api/transactions` returns the most recent 50.
- **Multiple workspaces (frontend-local)**: the sidebar's Workspace switcher can create/switch/delete named workspaces, each independently keeping its own watchlist, strategies, simulation history, and connected wallet address (all namespaced in `localStorage` by workspace id). The backend no longer has any daily-budget concept, so there's no longer a shared-budget-across-workspaces problem to worry about here.
- **Radar screen visualization**: the Markets page's Asset Radar panel can switch between a table view and a radar view (`src/RadarScreen.tsx`). The radar view is an SVG component: a dark circular display, concentric rings marking radar-score bands, and a continuously rotating sweep that's a pure CSS `conic-gradient` animation. Each asset is a dot — distance from center maps to its radar score (higher score, closer to the edge), color marks premium (orange) vs. discount (green), and the angle is a stable hash of the ticker string (so a given asset always lands in the same spot across refreshes instead of jittering around on every poll). Clicking a dot navigates straight to that asset's detail page.
- **Issuer race**: the asset detail page's "Issuer Comparison" went from a plain text list to a race-track visualization — one lane per issuer, with a marker positioned by that issuer's price relative to the full price range across issuers for this asset, and the highest quote highlighted orange with a lightning-bolt icon. The marker's position animates via a CSS `transition`, so it visibly slides into its new spot whenever data refreshes (every 60s) or you switch assets.
- **Historical signal replay**: `GET /api/radar/replay/{symbol}` (`backend/src/main.rs`'s `radar_replay`) queries `market_snapshots` directly — a table that's been recording one row per tracked asset roughly every 60 seconds ever since the live integration went up, now holding millions of rows across several days of real history. The endpoint finds that asset's single biggest historical gap, returns a minute-bucketed, ~60-point-downsampled series of price/reference/gap for the 3 hours before and after it, and flags `resolved: true/false` based on whether the gap at the end of the window has shrunk to less than half its peak. The asset detail page's new "Signal replay" panel calls this and draws the trajectory as a chart. Testing surfaced that some assets' reference prices get stuck for long stretches, producing a gap that stays large but perfectly flat instead of resolving — that's a stale-reference artifact, not real market movement, and the panel copy calls this out explicitly; see `docs/dx-report.md` for the full writeup.
- **Simulation scoreboard**: the Activity page's Simulations tab now shows a row of stat cards above the list — simulations run, total simulated volume, number of simulated assets currently scoring 70+, current daily streak, and best catch — computed entirely on the frontend from the existing `history` (the `localStorage`-persisted simulation log). No new backend involved, and deliberately no fabricated "theoretical P&L" figure, since that number wouldn't actually mean anything given the current data shape and would be more misleading than useful.

## Still needs real vendor wiring

RWA spot pricing and executable quotes can be real data (see above), but the following are still demo-shaped or not yet enabled:

- 24h `change` is currently estimated from a recent snapshot window (polled every 60s, roughly a 30-minute lookback), not a true calendar 24-hour change; needs either longer accumulated history or a dedicated endpoint.
- `liquidity` is currently a magnitude proxy based on `volume24H`, not a real DEX depth curve; the proper fix is pulling the `top-liquidity` endpoint and summing pools by contract address.
- There's no way to tell "the reference price is genuinely moving a lot" apart from "the reference price has gone stale while on-chain trading keeps running" — testing turned up several assets (KLAC, CVNA, SOXS, ENLV) whose reference price simply stops updating while the market is closed, stretching the gap to something dramatic-looking that's actually dead and will never converge, and looking identical to a real resolving signal if you only look at the current gap number. Right now the only way to tell is widening the time window and comparing historical snapshots yourself — the API doesn't expose a freshness field. Details and real numbers are in `docs/dx-report.md`.
- `/api/trades/simulate` (the demo/policy path) is still a policy/numeric simulation, not an on-chain EVM simulation; `/api/trades/execute` always returns 501 and never fabricates a tx hash, stores a private key, or broadcasts anything — that path is intentionally kept read-only/demo, and real signing goes through the separate `/api/trades/prepare` path described below.
- No contract allowlist yet (only trusts the contract address the RWA Data API returns, and requires the Transaction API's simulation to predict success), and no auth or rate limiting, no Redis cache (every live call is currently a fresh request, consistent with "never cache a quote," but also means there's no throttling on outbound API calls). The approve amount is always exactly what this specific trade needs (no "skip if already approved enough" optimization) — slightly more gas per trade, but avoids leaving excess allowance sitting around.
- Ondo/bStocks are Trading API RFQ routes — `GET /aggregator/quote` only returns a real quote if `userWalletAddress` is included (otherwise it silently falls back to the demo formula; this has been fixed, and both `Trade` and `PrepareTrade` now always include the connected wallet address). Separately, Ondo currently enforces a real **$5 minimum order size** (Binance error code 40375) — that's the issuer's own rule, not something this project can adjust; below $5, switch to bStocks or raise the amount.

A strategy being "enabled" only persists local demo state — there's no automated trading worker (no Signal Worker placing orders against enabled strategies). The sidebar's "Workspace" concept is purely frontend-local (keeping watchlist/strategies/history/wallet separate per demo scenario) and has nothing to do with the backend.

### Phase 4 (real wallet + real trading): two paths, only built one of them

Investigating Binance Agentic Wallet (`developers.binance.com/.../agentic-wallet`) turned up that it's **not** a backend-callable signing REST API the way RWA Data/Trading/Transaction are: it's an MPC keyless wallet with the private key tied to a person's own Binance account, authorized via QR-code login to the Binance App; the interface is a **Skill** installed for an AI assistant (`npx skills add binance/binance-skills-hub/skills/binance-web3/binance-agentic-wallet`), driven by natural-language conversation — not an endpoint a web backend calls. `rwa-radar-api` (a backend serving arbitrary users) has no documented way to call it "on behalf of a user" to sign and broadcast a transaction — using that path for real means the AI agent itself (e.g. Claude, via the installed Skill) acting directly on a signed-in individual's own account, never going through this repo's web app. `/api/trades/execute` therefore still returns 501.

What's actually implemented — and was only wired up to real funds after explicit user confirmation — is the **frontend wallet signing** path: the backend (`/api/trades/prepare`) builds unsigned approve/swap transactions via the Trading API and runs an EVM simulation via the Transaction API, never touching a private key; the already-connected browser wallet (MetaMask, etc.) signs and broadcasts them to BSC mainnet itself via `eth_sendTransaction`. See "What's implemented" above for the full detail.

Not done, and deliberately left for later: a contract allowlist (currently trusts whatever address the RWA Data API returns), approve-amount optimization, and any kind of multi-transaction queue/automation (a Signal Worker placing orders against enabled strategies).

## API

| Method | Path | What it does |
| --- | --- | --- |
| GET | `/api/health` | DB health and mode (`live`/`demo`) |
| GET | `/api/assets` | Normalized assets (live RWA data when a key is set, demo otherwise) |
| GET | `/api/assets/{symbol}` | Asset detail |
| GET | `/api/radar?minScore=70` | Sorted anomaly list |
| GET | `/api/radar/replay/{symbol}` | Price replay around that asset's biggest historical gap, ±3 hours |
| POST | `/api/trades/quote` | A fresh quote (DEX aggregator when the contract mapping is known, demo formula otherwise) |
| POST | `/api/trades/simulate` | Atomic limit validation and persisted record (reuses the quote above) |
| POST | `/api/trades/execute` | 501: backend-executed trading is disabled by design (see Phase 4 above) |
| POST | `/api/trades/prepare` | Builds a real approve/swap transaction + on-chain simulation for frontend wallet signing (needs `BINANCE_API_KEY`, 501 otherwise) |
| POST | `/api/trades/record` | Logs a real tx hash the frontend wallet already signed and sent (audit only) |
| GET | `/api/transactions` | Most recent 50 real transaction records |
| POST | `/api/agent/parse` | Natural language → strategy thresholds, one-shot (LLM when `OPENAI_API_KEY` is set, 501 otherwise) |
| POST | `/api/agent/chat` | Natural language → strategy thresholds, multi-turn (can ask questions, can revise a proposal) |
| POST | `/api/agent/filter` | Natural language → asset-table filter criteria |
| POST | `/api/agent/explain` | A single asset's anomaly signal → a plain-language explanation |
| GET | `/api/portfolio/{address}` | Real wallet balances (read-only; real data when `BINANCE_API_KEY` is set, 501 otherwise) |
| GET/POST | `/api/policies` | List strategies / create a disabled strategy |
| PATCH | `/api/policies/{id}` | Update enabled state |
| GET | `/api/history` | Most recent 100 simulations |
| GET | `/api/telemetry` | Real request counts, success rate, median latency |

Trade request: `{"asset":"NVDA","provider":"Ondo","side":"sell","amountUSD":1}` (`provider` must be one of that asset's actual issuers — `xStocks`/`Ondo`/`bStocks` in demo data, whatever the RWA Data API returns in live data, which in practice is almost always `Ondo` or `bStocks`). `prepare` additionally needs `walletAddress`.

## Verifying it

```sh
npm run build
cargo test --manifest-path backend/Cargo.toml
node --test tests/api.test.mjs
```

The API integration tests need PostgreSQL and the API running first; they only ever use a demo $0.01 amount and leave an auditable simulation record behind.

Golden path: Markets → NVDA → get a quote → run a simulation → check Activity for the record. Verifying real Execute/BSC confirmation requires completing the vendor wiring listed above.

## License

[MIT License](LICENSE).
