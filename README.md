# RWA Radar

[English version](README.en.md)

代幣化股票市場雷達 MVP。React 19 + TypeScript + Vite、Rust/Axum + PostgreSQL、Nginx，繁體中文介面。

開發過程中踩過的 API 坑、文件落差與重新設計建議寫在 [`docs/dx-report.md`](docs/dx-report.md)（[英文版](docs/dx-report-en.md)）。

## 啟動

```sh
npm ci
npm run dev
```

前端不依賴後端即可試用，明確標示本機示範資料，觀察清單、策略與模擬紀錄存於 localStorage。

完整容器環境：

```sh
docker compose up --build -d
```

開啟 http://localhost:3000 。Nginx 提供 SPA 與 `/api` 反向代理。PostgreSQL 使用持久化 volume，僅綁定本機 55432；API 預設綁定本機 8180（`compose.yaml` 可調整，容器間仍固定走內部 `api:8080`，不受此影響）。此為單一示範工作空間，未提供登入與多租戶隔離；請勿直接公開至網際網路。

本機 Rust 開發（PowerShell）：

```powershell
docker compose up -d db
$env:DATABASE_URL='postgres://radar:radar_dev_only@localhost:55432/radar'
# 選用：串接真實 Binance Web3 API（於 https://web3.binance.com/en/dev-portal 申請）
$env:BINANCE_API_KEY='...'
$env:BINANCE_API_SECRET='...'
# 選用：啟用 LLM 自然語言策略解析（於 https://platform.openai.com 申請）
$env:OPENAI_API_KEY='...'
cargo run --manifest-path backend/Cargo.toml
```

API 啟動時執行 migration。若設定 `BINANCE_API_KEY`/`BINANCE_API_SECRET`（見 `.env.example`），啟動與每 60 秒的背景工作會改向 Binance Web3 API 的 RWA Data 端點（`/api/v1/dex/market/rwa/tokens`）取得真實代幣化股票價格與參考價，並依 underlying ticker 正規化、寫入快照；未設定或初次取得失敗則回退為 8 項標的、3 個示範發行商的示範資料。資料庫憑證可透過 `.env` 覆寫；不要提交正式憑證或 API 金鑰。

## 已實作

- 市場總覽、搜尋、產業/發行商篩選、分數/價差排序、觀察清單、CSV 匯出。
- 資產價格圖、發行商比較、參考价差、模型流動性曲線。
- Rust 參考價差、歷史 Z-score、加權雷達異常分數。
- 示範報價及政策模擬；任一追蹤中資產皆可（無資產白名單）。單筆／每日金額與滑價上限已依使用者明確要求移除，僅保留基本輸入健檢（金額須為正數、≤ $100,000）。
- 限定格式自然語言策略解析與可審閱 DSL；策略預設停用。
- 模擬紀錄持久化、API 遙測與中位延遲統計。
- 響應式版面、空資料與錯誤提示、Nginx SPA fallback。
- **Binance Web3 API 串接（設定金鑰後生效）**：簽章 HTTP client（`backend/src/binance.rs`，HMAC-SHA256、`X-OC-*` headers，符合官方 authentication 規格）；RWA Data API 取得跨發行商（Ondo/BStock）代幣化股票即時價、參考價與開盤狀態，正規化為單一 underlying ticker（`backend/src/live.rs`）；`/api/trades/quote`、`/api/trades/simulate` 在已知合約地址時改用 Trading API 的 DEX aggregator 取得真實可執行報價與價格衝擊，並用 Transaction API 的即時 gas price 估算 gas 成本；找不到合約對應或呼叫失敗時自動回退示範公式。`/api/health` 回報目前是 `live` 或 `demo` 模式。
- **LLM Agent 串接（設定 `OPENAI_API_KEY` 後生效）**：都走 OpenAI Responses API 的 Structured Outputs（schema 約束輸出，`backend/src/agent.rs`），LLM 一律只做「提案」，不直接寫入策略或控制交易。
  - `POST /api/agent/parse`：把自由格式的中／英文策略描述一次解析成 `{asset, referenceGapPercent, executableGapPercent, amountUSD}`。後端仍以 `create_policy` 完全相同的規則（須為追蹤中資產、金額須為正數、門檻 0–100%）二次驗證，且要求資產代號必須真的出現在使用者輸入的文字裡，避免模型自行帶入一個文字裡沒有的代號。前端（`src/App.tsx` `createPolicy`）在 LLM 未啟用時靜默回退到規則式解析器；`/api/health` 的 `agentEnabled` 欄位供前端據以顯示正確文案。
  - `POST /api/agent/chat`：`/api/agent/parse` 的多輪對話版——同一套欄位與驗證規則，差別是 LLM 可以在資訊不足時先反問一個問題（`kind:"question"`），等湊齊足夠資訊才提出完整提案（`kind:"proposal"`），使用者也能針對已提出的提案用白話要求調整，不必整個重填。Strategies 頁 Build 分頁在 `agentEnabled` 時改用這個端點，呈現成聊天介面。
  - `POST /api/agent/filter`：把「用口語描述想找的資產」解析成資產表的篩選條件。請求會連同目前所有追蹤資產的「代號: 公司名稱」清單一起送給 LLM，讓它用自己對產業的知識把描述（例如「半導體股」）對應到真實存在的代號，回傳的 `matchedSymbols` 會再跟後端實際追蹤清單取交集，過濾掉任何沒有在清單裡的代號；流動性「低/高」刻意不讓 LLM 猜絕對數值，改由前端用目前即時資料的 25/75 百分位數換算。
  - `POST /api/agent/explain`：針對單一資產目前的異常訊號（缺口、Radar Score、流動性、發行商價差、開盤狀態）生成一句標題＋1–3 句白話解釋＋必要時的風險提醒。System prompt 明確要求只能根據傳入的數字做市場結構面的合理推論（流動性薄、參考價滯後等），不能編造新聞或財報，也不能給買賣建議。Markets 頁 Focus Signals 卡片上的「Interpret this signal」即呼叫此端點。
- **真實錢包連接與餘額（讀取）**：前端可透過瀏覽器注入式錢包（如 MetaMask）連接，自動切換／新增 BSC（chain 56）；連上後 Portfolio 頁呼叫 `GET /api/portfolio/{address}`（`backend/src/binance.rs` `wallet_balances`，包 Wallet API 的 `all-token-balances-by-address`）顯示該地址在 BSC 上的真實代幣餘額。這條路徑完全唯讀——後端只查詢公開鏈上資料，不涉及任何簽章或私鑰。
- **真實錢包簽署（BSC 主網，動用真實資金——經使用者明確確認後才串接）**：資產交易 modal 在「已連接錢包＋即時模式」時會出現獨立、以警示色標示的「簽署並送出真實交易」按鈕，與上方示範用的「取得新報價／執行模擬」完全分開，不會誤觸。原本僅開放 NVDA/TSLA，後經使用者明確要求已擴大到所有追蹤中資產；單筆／每日金額與滑價上限後續也依使用者明確要求整個移除（`policy_allowed`/`policy_block_reason` 已刪除），僅留 `validate` 的基本輸入健檢。流程（`POST /api/trades/prepare`，`backend/src/main.rs`）：① 取一筆新鮮的 Trading API 報價；② 建立一筆稽核用的模擬紀錄（不再有每日額度佔用邏輯，純粹記錄這次真實簽署嘗試）；③ 用 Trading API 的 `approve-transaction`／`swap` 建構 ERC-20 授權與 swap 的未簽章 calldata（兩者皆原樣帶上 Binance 回傳的即時 `gasPrice`，而非讓錢包自行估算——曾經特意拿掉這個欄位，結果一筆真實送出的交易因為錢包自己的預設 gas price 過低而卡住、最後被錢包自動取消，改回帶上後就穩定許多）；④ 用 Transaction API 的 `pre-transaction/simulate` 做鏈上模擬，只有預測會成功（`status:"SUCCESS"`）才把交易回傳給前端。這裡有一個例外：若模擬失敗的唯一原因是 allowance 不足、而這筆交易前面確實排了一筆 approve（也就是說這筆 allowance 馬上就會被補上），不會當成致命錯誤擋下——因為模擬器一次只吃單筆交易，沒辦法知道緊接著要送出的 approve 會改變鏈上狀態；其餘任何失敗原因（包含真的餘額不足）仍然照擋不誤。後端在這整個流程中**完全不接觸私鑰**：前端用 `eth_sendTransaction` 交給已連接的瀏覽器錢包簽署並送出（先送授權、等確認、再送 swap），送出後才用 `POST /api/trades/record` 把真實 tx hash 寫進 `transactions` 表（唯讀稽核紀錄，不影響簽署本身）。`GET /api/transactions` 可查最近 50 筆。
- **多工作空間（前端本機）**：側邊欄「Workspace」切換器可新增／切換／刪除具名工作空間，每個工作空間各自獨立保存觀察清單、策略、模擬紀錄與已連接錢包位址（皆以 `localStorage` 依 workspace id 命名空間隔離）。後端已無每日額度概念，故此處不再有跨工作空間共用預算的問題。
- **雷達螢幕視覺化**：Markets 頁 Asset Radar 面板可切換表格／雷達兩種檢視（`src/RadarScreen.tsx`）。雷達畫面是一個 SVG 元件：深色圓形畫面、同心圈代表 Radar Score 級距、持續旋轉的掃描光暈是純 CSS `conic-gradient` 動畫；每個資產是一個光點，離圓心距離對應 Radar Score（分數越高越靠邊緣），顏色對應溢價（橘）或折價（綠），角度用代號字串雜湊成固定值（同一資產每次重新整理都落在同一位置，不會因為輪詢刷新而亂跳），點擊光點直接跳轉到該資產詳情頁。
- **發行商競速**：資產詳情頁的「Issuer Comparison」從純文字清單改成賽道視覺——每個發行商一條跑道，標記位置依報價在該資產所有發行商報價區間中的相對位置計算，報價最高者標橘色並帶閃電圖示；標記用 CSS `transition` 做位移動畫，資料每 60 秒刷新或切換資產時會「滑動」到新位置。
- **歷史訊號回放**：`GET /api/radar/replay/{symbol}`（`backend/src/main.rs` `radar_replay`）直接查詢 `market_snapshots` 這張表——它從即時整合上線後就一直在記錄每個資產每 60 秒一筆的快照，目前已累積數百萬筆、橫跨數天的真實歷史。端點找出該資產歷史上偏離最大的單一時刻，回傳前後各 3 小時、依分鐘聚合、降採樣到約 60 個點的價格／參考價／缺口序列，並依「結尾缺口是否已縮小到峰值一半以下」標示 `resolved: true/false`。資產詳情頁新增「Signal replay」面板呼叫此端點並畫出折線圖。測試中發現有些資產的參考價會長時間卡住不動，導致缺口一直很大卻「持平」不收斂——這其實是參考價資料滯後的假訊號，不是真的市場波動，面板文案有特別註明這個落差，細節見 `docs/dx-report.md`。
- **模擬記分板**：Activity 頁「模擬紀錄」分頁上方新增一排統計卡（已執行模擬次數、模擬總額、目前分數 ≥70 的已模擬資產數、連續活躍天數＋最佳捕獲），純前端從既有的 `history`（localStorage 持久化的模擬紀錄）即時算出，沒有新增後端，也刻意不做「理論損益」這種在目前資料結構下沒有意義、容易誤導的數字。

## 尚需正式供應商串接

RWA 現貨價格與可執行報價可為真實資料（見上），但以下仍是示範或尚未啟用：

- 24h 漲跌 `change` 目前以近期快照窗（60 秒一次、最多約 30 分鐘）估算動能，並非真正的日曆 24 小時漲跌；需要長時間運行累積歷史或改抓專門端點。
- `liquidity` 目前是 `volume24H` 的量級代理，並非真實 DEX 深度曲線；正式應改抓 `top-liquidity` 端點依合約地址加總流動性池。
- 沒有辦法區分「參考價真的在大幅波動」跟「參考價資料滯後、鏈上交易還在跑」這兩種情況——實測發現有些資產（KLAC、CVNA、SOXS、ENLV）休市時參考價會直接停更，缺口因此被拉到很誇張但其實是死的、不會收斂，單看當下的 gap 數字跟真正會收斂的訊號長得一模一樣。目前只能靠拉長時間窗自己比對歷史快照判斷，API 沒有提供新鮮度（freshness）欄位；細節與數字見 `docs/dx-report.md`。
- `/api/trades/simulate`（示範/政策路徑）本身仍是政策/數值模擬，不做鏈上 EVM 模擬；`/api/trades/execute` 固定回傳 501，不產生假 tx hash、不保存私鑰、不廣播交易——這條路徑刻意維持唯讀／示範，真實簽署走的是下面說明的獨立 `/api/trades/prepare` 路徑。
- 目前沒有合約白名單機制（僅信任 RWA Data API 回傳的合約地址，並要求 Transaction API 模擬預測成功），也還沒有認證與 rate limiting、Redis 快取（目前每次即時呼叫皆為 fresh request，符合「報價不快取」原則，但也代表沒有 API 呼叫節流）。approve 金額目前總是等於本次交易所需的確切數量（不做「已有足夠額度就跳過」的最佳化），略增加 gas 成本但避免殘留過量授權。
- Ondo/BStock 屬於 Trading API 的 RFQ 路由，`GET /aggregator/quote` 需要帶 `userWalletAddress` 才會回傳真實報價（否則回退示範公式，已修正並在 `Trade`/`PrepareTrade` 都會帶上已連接的錢包地址）；另外 Ondo 目前有 **$5 最低下單金額**的限制（Binance 錯誤碼 40375），這是發行商自己的規定，非本專案可調整——金額低於 $5 時建議改用 bStocks 或提高金額。

策略「啟用」僅保存本機示範狀態，未提供自動交易工作器（沒有 Signal Worker 依已啟用策略自動下單）。側邊欄的「Workspace」是前端本機概念（分開保存觀察清單/策略/紀錄/錢包，方便示範情境切換），與後端完全無關。

### Phase 4（真實錢包＋真實交易）：兩條路徑，只做了前端錢包簽署那條

研究 Binance Agentic Wallet（`developers.binance.com/.../agentic-wallet`）後發現，它**不是**像 RWA Data／Trading／Transaction API 那樣可從後端呼叫的簽章 REST API：它是 MPC keyless 錢包，私鑰與使用者自己的 Binance 帳戶綁定，透過 QR code 登入 Binance App 授權；操作介面是安裝給 AI 助理的 **Skills**（`npx skills add binance/binance-skills-hub/skills/binance-web3/binance-agentic-wallet`），以自然語言對話下單，不是給網頁後端呼叫的端點。`rwa-radar-api`（服務任意使用者的多租戶後端）沒有文件化的方式可以「代表某個使用者」呼叫它去簽署並廣播交易——這條路徑若要用，是由 AI 代理人本身（例如 Claude，透過已安裝的 Skill）直接對登入的個人執行，不經過這個 repo 的網頁。`/api/trades/execute` 因此維持回傳 501。

已實作、且經使用者明確確認要接上真實資金後才做的，是**前端錢包簽署**這條路徑：後端（`/api/trades/prepare`）用 Trading API 建構未簽章的 approve/swap 交易、用 Transaction API 做 EVM 模擬，全程不接觸私鑰；瀏覽器端已連接的錢包（MetaMask 等）用 `eth_sendTransaction` 自行簽署並廣播到 BSC 主網，成功後由 `/api/trades/record` 記錄真實 tx hash。細節見上方「已實作」。

沒做、也刻意留到之後的：合約白名單（目前信任 RWA Data API 回傳的地址）、approve 額度最佳化、多筆交易佇列/自動化（Signal Worker 依已啟用策略自動下單）。

## API

| Method | Path | 功能 |
| --- | --- | --- |
| GET | `/api/health` | DB 健康狀態與模式（`live`／`demo`） |
| GET | `/api/assets` | 正規化資產（有金鑰時為即時 RWA 資料，否則示範） |
| GET | `/api/assets/{symbol}` | 資產詳情 |
| GET | `/api/radar?minScore=70` | 排序後異常清單 |
| GET | `/api/radar/replay/{symbol}` | 該資產歷史最大缺口時刻前後 6 小時的價格回放 |
| POST | `/api/trades/quote` | 新鮮報價（已知合約地址時走 DEX aggregator，否則示範公式） |
| POST | `/api/trades/simulate` | 原子限額驗證並存檔（沿用上述報價） |
| POST | `/api/trades/execute` | 501：後端代為執行未啟用（設計如此，見下方 Phase 4 說明） |
| POST | `/api/trades/prepare` | 建構真實 approve/swap 交易＋鏈上模擬，供前端錢包簽署（需 `BINANCE_API_KEY`，否則 501） |
| POST | `/api/trades/record` | 記錄前端錢包已簽署送出的真實 tx hash（稽核用） |
| GET | `/api/transactions` | 最近 50 筆真實交易紀錄 |
| POST | `/api/agent/parse` | 自然語言 → 策略門檻，一次解析（有 `OPENAI_API_KEY` 時為 LLM，否則 501） |
| POST | `/api/agent/chat` | 自然語言 → 策略門檻，多輪對話版（可反問、可修改提案） |
| POST | `/api/agent/filter` | 自然語言 → 資產表篩選條件 |
| POST | `/api/agent/explain` | 單一資產異常訊號 → 白話解讀 |
| GET | `/api/portfolio/{address}` | 真實錢包餘額（唯讀，有 `BINANCE_API_KEY` 時為真實資料，否則 501） |
| GET/POST | `/api/policies` | 策略列表 / 建立停用策略 |
| PATCH | `/api/policies/{id}` | 更新狀態 |
| GET | `/api/history` | 最近 100 筆模擬 |
| GET | `/api/telemetry` | 實測請求數、成功率、中位延遲 |

Trade request：`{"asset":"NVDA","provider":"Ondo","side":"sell","amountUSD":1}`（`provider` 須為該資產實際擁有的發行商之一，示範資料下為 `xStocks`/`Ondo`/`bStocks`，即時資料下依 RWA Data API 回傳而定，常見為 `Ondo`/`bStocks`）。`prepare` 另需 `walletAddress`。

## 驗證

```sh
npm run build
cargo test --manifest-path backend/Cargo.toml
node --test tests/api.test.mjs
```

API 整合測試需先啟動 PostgreSQL 與 API；僅使用示範 $0.01 並留下可稽核模擬紀錄。

Golden path：Markets → NVDA → 取得報價 → 執行模擬 → Activity 查看交易紀錄。正式 Execute/BSC confirmation 需完成上列供應商串接才可驗證。

## 授權

[MIT License](LICENSE)。
