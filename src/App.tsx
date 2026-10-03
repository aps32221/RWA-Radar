import { useEffect, useRef, useState } from "react";
import {
  Activity,
  AlertTriangle,
  ArrowDownLeft,
  ArrowUpRight,
  ArrowRight,
  ArrowLeft,
  BarChart3,
  Bell,
  Check,
  ChevronDown,
  ChevronRight,
  Clock,
  Command,
  Download,
  ExternalLink,
  Globe,
  History,
  Layers,
  LayoutDashboard,
  Menu,
  List,
  Plus,
  Radar as RadarIcon,
  Radio,
  Search,
  Settings2,
  ShieldCheck,
  Sparkles,
  Wallet,
  X,
  Zap,
} from "lucide-react";
import {
  Area,
  AreaChart,
  CartesianGrid,
  ResponsiveContainer,
  Tooltip,
  XAxis,
  YAxis,
  ReferenceLine,
} from "recharts";
import { seed, money, compact, gap, type Asset } from "./data";
import { type Lang, translate, apiErrorMessage, defaultLang, LANGS } from "./i18n";
import { RadarScreen } from "./RadarScreen";
type Policy = {
  id: string;
  asset: string;
  threshold: number;
  executableThreshold: number;
  amount: number;
  enabled: boolean;
};
type Simulation = {
  id: string;
  asset: string;
  amountUSD: number;
  expectedOutput: number;
  priceImpact: number;
  gasEstimate: number;
  success: boolean;
  reason: string;
  reasonCode?: string;
  vars?: Record<string, string | number>;
  createdAt: string;
  mode?: string;
};
type RealTx = {
  id: string;
  walletAddress?: string;
  txHash: string;
  chainId: number;
  status: string;
  createdAt: string;
};
type Workspace = { id: string; name: string };
const DEFAULT_WORKSPACE: Workspace = { id: "default", name: "Personal" };
const wsKey = (base: string, wsId: string) => `${base}:${wsId}`;
const ALL_SECTORS = "ALL";
const ALL_PROVIDERS = "ALL";
// Nav consolidated from 7 pages down to 4: Market Radar's own view merged
// into Overview (they already rendered identical JSX), AI Agent + Policy
// Rules merged into Strategies (build vs manage tabs), and Transaction
// History + Developer Monitor merged into Activity (simulations / real
// transactions / developer tabs). Fewer top-level destinations, same data.
const nav = [
  ["overview", "nav.overview", LayoutDashboard],
  ["portfolio", "nav.portfolio", Wallet],
  ["strategies", "nav.strategies", Sparkles],
  ["activity", "nav.activity", History],
] as const;
function read<T>(key: string, fallback: T): T {
  try {
    return JSON.parse(localStorage.getItem(key) || "null") ?? fallback;
  } catch {
    return fallback;
  }
}
export default function App() {
  const [page, setPage] = useState(
      location.pathname.split("/")[1] || "overview",
    ),
    [selected, setSelected] = useState<Asset | undefined>(
      seed.find((a) => a.symbol === location.pathname.split("/")[2]),
    ),
    [assets, setAssets] = useState(seed),
    [apiMode, setApiMode] = useState<"offline" | "demo" | "live">("offline"),
    [lang, setLang] = useState<Lang>(read("radar-lang", defaultLang())),
    [agentEnabled, setAgentEnabled] = useState(false),
    [workspaces, setWorkspaces] = useState<Workspace[]>(
      read("radar-workspaces", [DEFAULT_WORKSPACE]),
    ),
    [activeWorkspace, setActiveWorkspace] = useState<string>(
      read("radar-active-workspace", DEFAULT_WORKSPACE.id),
    ),
    [workspaceMenuOpen, setWorkspaceMenuOpen] = useState(false),
    [newWorkspaceName, setNewWorkspaceName] = useState(""),
    [wallet, setWallet] = useState(
      read(wsKey("radar-wallet", activeWorkspace), ""),
    ),
    [portfolio, setPortfolio] = useState<
      {
        symbol: string;
        balance: string;
        usdPrice?: string;
        contractAddress?: string;
        riskFlagged: boolean;
      }[]
    >(),
    [portfolioLoading, setPortfolioLoading] = useState(false),
    [query, setQuery] = useState(""),
    [sector, setSector] = useState(ALL_SECTORS),
    [provider, setProvider] = useState(ALL_PROVIDERS),
    [sort, setSort] = useState("score"),
    [viewMode, setViewMode] = useState<"table" | "radar">("table"),
    [watch, setWatch] = useState<string[]>(
      read(wsKey("radar-watch", activeWorkspace), []),
    ),
    [onlyWatch, setOnlyWatch] = useState(false),
    [toast, setToast] = useState(""),
    [mobile, setMobile] = useState(false),
    [trade, setTrade] = useState<Asset>(),
    [tradeProvider, setTradeProvider] = useState(""),
    [amount, setAmount] = useState(5),
    [side, setSide] = useState("sell"),
    [sim, setSim] = useState<Simulation>(),
    [busy, setBusy] = useState(false),
    [signing, setSigning] = useState(false),
    [signResult, setSignResult] = useState<{ txHash: string; chainId: number }>(),
    [showQuoteDetails, setShowQuoteDetails] = useState(false),
    [confirmRealFunds, setConfirmRealFunds] = useState(false),
    [strategyTab, setStrategyTab] = useState<"build" | "manage">("build"),
    [activityTab, setActivityTab] = useState<
      "simulations" | "realtx" | "developer"
    >("simulations"),
    [realTx, setRealTx] = useState<RealTx[]>([]),
    [policies, setPolicies] = useState<Policy[]>(
      read(wsKey("radar-policies", activeWorkspace), []),
    ),
    [history, setHistory] = useState<Simulation[]>(
      read(wsKey("radar-history", activeWorkspace), []),
    ),
    [prompt, setPrompt] = useState(
      "If NVDA trades 3% above reference and executable premium remains above 2%, sell $5.",
    ),
    [answer, setAnswer] = useState(""),
    [range, setRange] = useState("24H"),
    [quote, setQuote] = useState<{
      expectedOutput: number;
      priceImpact: number;
      gasEstimate: number;
    }>(),
    [telemetry, setTelemetry] = useState<
      {
        endpoint: string;
        requests: number;
        successRate: number;
        latencyMs: number;
      }[]
    >([]),
    [explanations, setExplanations] = useState<
      Record<
        string,
        | { headline: string; explanation: string; caution: string }
        | "loading"
        | "error"
      >
    >({}),
    [aiFilterInput, setAiFilterInput] = useState(""),
    [aiFilterBusy, setAiFilterBusy] = useState(false),
    [aiFilter, setAiFilter] = useState<{
      matchedSymbols: string[];
      query: string | null;
      minGapPercent: number | null;
      minScore: number | null;
      liquidity: "low" | "high" | null;
      summary: string;
    }>(),
    [chatMessages, setChatMessages] = useState<
      { role: "user" | "assistant"; content: string }[]
    >([]),
    [chatInput, setChatInput] = useState(""),
    [chatBusy, setChatBusy] = useState(false),
    [chatProposal, setChatProposal] = useState<{
      asset: string;
      threshold: number;
      executableThreshold: number;
      amount: number;
    }>(),
    [replayData, setReplayData] = useState<{
      symbol: string;
      peakGapPercent: number;
      peakAt: string;
      resolved: boolean;
      points: { t: string; price: number; reference: number; gapPercent: number }[];
    }>(),
    [replayLoading, setReplayLoading] = useState(false);
  const t = (key: string, vars?: Record<string, string | number>) =>
    translate(lang, key, vars);
  const notify = (s: string) => {
    setToast(s);
    window.setTimeout(() => setToast(""), 4000);
  };
  const explainSignal = async (symbol: string) => {
    if (explanations[symbol] === "loading") return;
    setExplanations((e) => ({ ...e, [symbol]: "loading" }));
    try {
      const r = await fetch("/api/agent/explain", {
        method: "POST",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify({ asset: symbol }),
      });
      const d = await r.json();
      if (!r.ok) throw new Error(apiErrorMessage(lang, d, "notify.agentExplainFailed"));
      setExplanations((e) => ({
        ...e,
        [symbol]: {
          headline: d.headline || "",
          explanation: d.explanation || "",
          caution: d.caution || "",
        },
      }));
    } catch (err) {
      setExplanations((e) => ({ ...e, [symbol]: "error" }));
      notify((err as Error).message || t("notify.agentExplainFailed"));
    }
  };
  const applyAiFilter = async () => {
    const text = aiFilterInput.trim();
    if (!text || aiFilterBusy) return;
    setAiFilterBusy(true);
    try {
      const r = await fetch("/api/agent/filter", {
        method: "POST",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify({ prompt: text }),
      });
      const d = await r.json();
      if (!r.ok) throw new Error(apiErrorMessage(lang, d, "notify.agentFilterFailed"));
      setAiFilter(d);
    } catch (err) {
      notify((err as Error).message || t("notify.agentFilterFailed"));
    } finally {
      setAiFilterBusy(false);
    }
  };
  const clearAiFilter = () => {
    setAiFilter(undefined);
    setAiFilterInput("");
  };
  const sendChatMessage = async () => {
    const text = chatInput.trim();
    if (!text || chatBusy) return;
    const next = [...chatMessages, { role: "user" as const, content: text }];
    setChatMessages(next);
    setChatInput("");
    setChatProposal(undefined);
    setChatBusy(true);
    try {
      const r = await fetch("/api/agent/chat", {
        method: "POST",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify({ messages: next }),
      });
      const d = await r.json();
      if (!r.ok) throw new Error(apiErrorMessage(lang, d, "notify.agentChatFailed"));
      setChatMessages((m) => [...m, { role: "assistant", content: d.message || "" }]);
      if (d.kind === "proposal") {
        setChatProposal({
          asset: d.asset,
          threshold: d.threshold,
          executableThreshold: d.executableThreshold,
          amount: d.amount,
        });
      }
    } catch (err) {
      notify((err as Error).message || t("notify.agentChatFailed"));
      setChatMessages((m) => m.slice(0, -1));
      setChatInput(text);
    } finally {
      setChatBusy(false);
    }
  };
  const confirmChatProposal = async () => {
    const cp = chatProposal;
    if (!cp) return;
    const p: Policy = {
      id: crypto.randomUUID(),
      asset: cp.asset,
      threshold: cp.threshold,
      executableThreshold: cp.executableThreshold,
      amount: cp.amount,
      enabled: false,
    };
    setPolicies((ps) => [p, ...ps]);
    setChatProposal(undefined);
    setChatMessages((m) => [
      ...m,
      { role: "assistant", content: t("agentChat.createdNote", { symbol: p.asset }) },
    ]);
    if (apiMode !== "offline") {
      try {
        const r = await fetch("/api/policies", {
          method: "POST",
          headers: { "Content-Type": "application/json" },
          body: JSON.stringify(p),
        });
        if (!r.ok) throw Error();
        notify(t("notify.policySaved"));
      } catch {
        notify(t("notify.policySavedLocalOnly"));
      }
    } else notify(t("notify.policySavedDemo"));
  };
  const resetChat = () => {
    setChatMessages([]);
    setChatInput("");
    setChatProposal(undefined);
  };
  const loadReplay = async (symbol: string) => {
    setReplayLoading(true);
    try {
      const r = await fetch(`/api/radar/replay/${symbol}`);
      const d = await r.json();
      if (!r.ok) throw new Error(apiErrorMessage(lang, d, "notify.replayFailed"));
      setReplayData(d);
    } catch (err) {
      notify((err as Error).message || t("notify.replayFailed"));
    } finally {
      setReplayLoading(false);
    }
  };
  const go = (p: string, a?: Asset) => {
    setPage(p);
    setSelected(a);
    setMobile(false);
    setReplayData(undefined);
    window.history.pushState(
      {},
      "",
      p === "overview"
        ? "/"
        : p === "assets"
          ? `/assets/${a?.symbol}`
          : `/${p}`,
    );
  };
  useEffect(() => {
    const fn = () => {
      setPage(location.pathname.split("/")[1] || "overview");
      setSelected(
        seed.find((a) => a.symbol === location.pathname.split("/")[2]),
      );
    };
    window.addEventListener("popstate", fn);
    return () => window.removeEventListener("popstate", fn);
  }, []);
  useEffect(() => {
    fetch("/api/health")
      .then((r) => {
        if (!r.ok) throw Error();
        return r.json();
      })
      .then((d) => {
        setAgentEnabled(!!d.agentEnabled);
        setApiMode(d.mode === "live" ? "live" : "demo");
      })
      .catch(() => {});
    fetch("/api/assets")
      .then((r) => {
        if (!r.ok) throw Error();
        return r.json();
      })
      .then((d) => {
        if (Array.isArray(d) && d.length) {
          setAssets(d);
          setSelected(current => current ? d.find((a: Asset) => a.symbol === current.symbol) : current);
        }
      })
      .catch(() => {});
    fetch("/api/telemetry")
      .then((r) => r.json())
      .then((d) => Array.isArray(d) && setTelemetry(d))
      .catch(() => {});
  }, []);
  const wsRef = useRef(activeWorkspace);
  useEffect(() => {
    localStorage.setItem("radar-workspaces", JSON.stringify(workspaces));
  }, [workspaces]);
  useEffect(() => {
    localStorage.setItem("radar-active-workspace", JSON.stringify(activeWorkspace));
    wsRef.current = activeWorkspace;
    setWatch(read(wsKey("radar-watch", activeWorkspace), []));
    setWallet(read(wsKey("radar-wallet", activeWorkspace), ""));
    setPolicies(read(wsKey("radar-policies", activeWorkspace), []));
    setHistory(read(wsKey("radar-history", activeWorkspace), []));
    setTrade(undefined);
  }, [activeWorkspace]);
  useEffect(() => {
    localStorage.setItem(wsKey("radar-watch", wsRef.current), JSON.stringify(watch));
  }, [watch]);
  useEffect(() => {
    localStorage.setItem("radar-lang", JSON.stringify(lang));
  }, [lang]);
  useEffect(() => {
    localStorage.setItem(wsKey("radar-wallet", wsRef.current), JSON.stringify(wallet));
  }, [wallet]);
  useEffect(() => {
    if (!wallet || apiMode === "offline") {
      setPortfolio(undefined);
      return;
    }
    setPortfolioLoading(true);
    fetch(`/api/portfolio/${wallet}`)
      .then((r) => {
        if (!r.ok) throw Error();
        return r.json();
      })
      .then((d) => setPortfolio(Array.isArray(d) ? d : []))
      .catch(() => {
        setPortfolio(undefined);
        notify(t("notify.portfolioLoadFailed"));
      })
      .finally(() => setPortfolioLoading(false));
  }, [wallet, apiMode]);
  useEffect(() => {
    localStorage.setItem(wsKey("radar-policies", wsRef.current), JSON.stringify(policies));
  }, [policies]);
  useEffect(() => {
    localStorage.setItem(wsKey("radar-history", wsRef.current), JSON.stringify(history));
  }, [history]);
  useEffect(() => {
    if (apiMode === "offline" || page !== "activity") return;
    fetch('/api/history').then(r => {if (!r.ok) throw Error(); return r.json()}).then(d => {if (Array.isArray(d)) setHistory(d)}).catch(() => notify(t("notify.historyLoadFailed")));
    fetch('/api/telemetry').then(r => r.json()).then(d => {if (Array.isArray(d)) setTelemetry(d)}).catch(() => notify(t("notify.telemetryLoadFailed")));
    if (wallet) fetch(`/api/transactions`).then(r => {if (!r.ok) throw Error(); return r.json()}).then(d => {if (Array.isArray(d)) setRealTx(d)}).catch(() => {});
  }, [page, apiMode]);
  const liquiditySorted = [...assets].map((a) => a.liquidity).sort((a, b) => a - b);
  const liquidityP25 = liquiditySorted[Math.floor(liquiditySorted.length * 0.25)] ?? 0;
  const liquidityP75 =
    liquiditySorted[Math.floor(liquiditySorted.length * 0.75)] ?? Infinity;
  const filtered = assets
    .map(a => ({...a, price: provider === ALL_PROVIDERS ? a.price : a.providers.find(p => p.name === provider)!.price}))
    .filter(
      (a) =>
        (!onlyWatch || watch.includes(a.symbol)) &&
        (sector === ALL_SECTORS || a.sector === sector) &&
        (a.symbol + " " + a.name).toLowerCase().includes(query.toLowerCase()) &&
        (!aiFilter ||
          ((aiFilter.matchedSymbols.length === 0 ||
            aiFilter.matchedSymbols.includes(a.symbol)) &&
            (!aiFilter.query ||
              (a.symbol + " " + a.name)
                .toLowerCase()
                .includes(aiFilter.query.toLowerCase())) &&
            (aiFilter.minGapPercent == null ||
              Math.abs(gap(a)) >= aiFilter.minGapPercent) &&
            (aiFilter.minScore == null || a.score >= aiFilter.minScore) &&
            (aiFilter.liquidity !== "low" || a.liquidity <= liquidityP25) &&
            (aiFilter.liquidity !== "high" || a.liquidity >= liquidityP75))),
    )
    .sort((a, b) =>
      sort === "score"
        ? b.score - a.score
        : sort === "gap"
          ? Math.abs(gap(b)) - Math.abs(gap(a))
          : b.volume - a.volume,
    );
  const createWorkspace = () => {
    const name = newWorkspaceName.trim();
    if (!name) return;
    const id = crypto.randomUUID();
    setWorkspaces((ws) => [...ws, { id, name }]);
    setNewWorkspaceName("");
    setActiveWorkspace(id);
    setWorkspaceMenuOpen(false);
  };
  const deleteWorkspace = (id: string) => {
    if (workspaces.length <= 1) return;
    setWorkspaces((ws) => ws.filter((w) => w.id !== id));
    [
      wsKey("radar-watch", id),
      wsKey("radar-wallet", id),
      wsKey("radar-policies", id),
      wsKey("radar-history", id),
    ].forEach((k) => localStorage.removeItem(k));
    if (activeWorkspace === id) {
      const next = workspaces.find((w) => w.id !== id);
      if (next) setActiveWorkspace(next.id);
    }
  };
  const BSC_CHAIN_HEX = "0x38";
  const connectWallet = async () => {
    const eth = (window as any).ethereum;
    if (!eth) {
      notify(t("notify.walletNotDetected"));
      return;
    }
    try {
      const accounts: string[] = await eth.request({
        method: "eth_requestAccounts",
      });
      if (!accounts?.[0]) throw new Error("no account");
      try {
        await eth.request({
          method: "wallet_switchEthereumChain",
          params: [{ chainId: BSC_CHAIN_HEX }],
        });
      } catch (switchErr: any) {
        if (switchErr?.code === 4902) {
          await eth.request({
            method: "wallet_addEthereumChain",
            params: [
              {
                chainId: BSC_CHAIN_HEX,
                chainName: "BNB Smart Chain",
                nativeCurrency: { name: "BNB", symbol: "BNB", decimals: 18 },
                rpcUrls: ["https://bsc-dataseed.binance.org"],
                blockExplorerUrls: ["https://bscscan.com"],
              },
            ],
          });
        } else throw switchErr;
      }
      setWallet(accounts[0]);
      notify(
        t("notify.walletConnected", {
          addr: `${accounts[0].slice(0, 6)}…${accounts[0].slice(-4)}`,
        }),
      );
    } catch {
      notify(t("notify.walletConnectFailed"));
    }
  };
  const disconnectWallet = () => {
    setWallet("");
    setPortfolio(undefined);
    notify(t("notify.walletDisconnected"));
  };
  const openTrade = (a: Asset) => {
    setTrade(a);
    // Prefer bStocks by default: Ondo's RFQ route enforces a $5 minimum
    // order size (Binance error 40375), so small test amounts silently fail
    // unless bStocks is picked deliberately. `providers[0]` order isn't
    // stable across refreshes, so without this a user can land on Ondo.
    setTradeProvider(
      a.providers.find((p) => p.name === "bStocks")?.name ||
        a.providers[0]?.name ||
        "",
    );
    setSim(undefined);
    setQuote(undefined);
    setSignResult(undefined);
    setShowQuoteDetails(false);
    setConfirmRealFunds(false);
    setAmount(5);
  };
  const runTrade = async (kind: "quote" | "simulate") => {
    if (!trade || !Number.isFinite(amount) || amount <= 0) {
      notify(t("notify.invalidAmount"));
      return;
    }
    setBusy(true);
    try {
      let result;
      if (apiMode !== "offline") {
        const r = await fetch("/api/trades/" + kind, {
          method: "POST",
          headers: { "Content-Type": "application/json" },
          body: JSON.stringify({
            asset: trade.symbol,
            provider: tradeProvider || trade.providers[0]?.name,
            side,
            amountUSD: amount,
            ...(wallet ? { walletAddress: wallet } : {}),
          }),
        });
        result = await r.json();
        if (!r.ok)
          throw Error(apiErrorMessage(lang, result, "notify.apiRequestFailed"));
      } else {
        const impact = amount / trade.liquidity;
        result = {
          id: crypto.randomUUID(),
          asset: trade.symbol,
          amountUSD: amount,
          expectedOutput: amount * (1 - impact),
          priceImpact: impact,
          gasEstimate: 0.01,
          success: true,
          reason: t("notify.demoPolicyReason"),
          createdAt: new Date().toISOString(),
        };
      }
      if (kind === "quote") {
        setQuote(result);
        setShowQuoteDetails(false);
      } else {
        setSim(result);
        setHistory((h) => [result, ...h]);
      }
    } catch (e) {
      notify((e as Error).message);
    } finally {
      setBusy(false);
    }
  };
  const toHex = (v?: string) => {
    if (v === undefined) return undefined;
    if (v.startsWith("0x")) return v;
    try {
      return "0x" + BigInt(v).toString(16);
    } catch {
      return undefined;
    }
  };
  const waitForReceipt = async (eth: any, hash: string, label: string) => {
    for (let i = 0; i < 60; i++) {
      const receipt = await eth.request({
        method: "eth_getTransactionReceipt",
        params: [hash],
      });
      if (receipt) {
        if (receipt.status === "0x0")
          throw new Error(t("notify.txLandedButFailed", { label }));
        return receipt;
      }
      await new Promise((r) => setTimeout(r, 3000));
    }
    throw new Error(t("notify.confirmTimeout", { label }));
  };
  const signAndSend = async () => {
    if (!trade || !wallet) return;
    const eth = (window as any).ethereum;
    if (!eth) {
      notify(t("notify.walletNotDetected"));
      return;
    }
    setSigning(true);
    setSignResult(undefined);
    try {
      const r = await fetch("/api/trades/prepare", {
        method: "POST",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify({
          asset: trade.symbol,
          provider: tradeProvider || trade.providers[0]?.name,
          side,
          amountUSD: amount,
          walletAddress: wallet,
        }),
      });
      const plan = await r.json();
      if (!r.ok)
        throw new Error(apiErrorMessage(lang, plan, "notify.apiRequestFailed"));
      if (plan.approveTx) {
        notify(t("notify.confirmApprove"));
        const approveHash: string = await eth.request({
          method: "eth_sendTransaction",
          params: [
            {
              from: wallet,
              to: plan.approveTx.to,
              data: plan.approveTx.data,
              value: toHex(plan.approveTx.value) ?? "0x0",
              ...(plan.approveTx.gas
                ? { gas: toHex(plan.approveTx.gas) }
                : {}),
              ...(plan.approveTx.gasPrice
                ? { gasPrice: toHex(plan.approveTx.gasPrice) }
                : {}),
            },
          ],
        });
        notify(t("notify.waitingApprove"));
        await waitForReceipt(eth, approveHash, t("notify.approveLabel"));
      }
      notify(t("notify.confirmSwap"));
      const swapHash: string = await eth.request({
        method: "eth_sendTransaction",
        params: [
          {
            from: wallet,
            to: plan.swapTx.to,
            data: plan.swapTx.data,
            value: toHex(plan.swapTx.value) ?? "0x0",
            ...(plan.swapTx.gas ? { gas: toHex(plan.swapTx.gas) } : {}),
            ...(plan.swapTx.gasPrice
              ? { gasPrice: toHex(plan.swapTx.gasPrice) }
              : {}),
          },
        ],
      });
      setSignResult({ txHash: swapHash, chainId: plan.chainId ? Number(plan.chainId) : 56 });
      notify(t("notify.realTxSentWaiting"));
      fetch("/api/trades/record", {
        method: "POST",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify({
          txHash: swapHash,
          chainId: plan.chainId ? Number(plan.chainId) : 56,
          walletAddress: wallet,
        }),
      }).catch(() => {});
      await waitForReceipt(eth, swapHash, "");
      notify(t("notify.txConfirmed"));
    } catch (e) {
      notify((e as Error).message || t("notify.signOrSendFailed"));
    } finally {
      setSigning(false);
    }
  };
  const exportData = () => {
    const blob = new Blob(
      [
        [
          "symbol,price,reference,gap,radarScore",
          ...filtered.map(
            (a) =>
              `${a.symbol},${a.price},${a.reference},${gap(a).toFixed(4)},${a.score}`,
          ),
        ].join("\n"),
      ],
      { type: "text/csv" },
    );
    const url = URL.createObjectURL(blob);
    const link = document.createElement("a");
    link.href = url;
    link.download = "rwa-radar-demo.csv";
    link.click();
    URL.revokeObjectURL(url);
  };
  const createPolicy = async () => {
    let p: Policy | undefined;
    let parsedBy: "llm" | "rule" = "rule";
    if (agentEnabled && apiMode !== "offline") {
      try {
        const r = await fetch("/api/agent/parse", {
          method: "POST",
          headers: { "Content-Type": "application/json" },
          body: JSON.stringify({ prompt }),
        });
        const d = await r.json();
        if (r.ok) {
          p = {
            id: crypto.randomUUID(),
            asset: d.asset,
            threshold: d.threshold,
            executableThreshold: d.executableThreshold,
            amount: d.amount,
            enabled: false,
          };
          parsedBy = "llm";
        } else if (r.status !== 501) {
          notify(apiErrorMessage(lang, d, "notify.agentParseFailed"));
        }
      } catch {
        // network/LLM failure: silently fall through to the rule-based parser
      }
    }
    if (!p) {
      const asset = assets.find((a) => prompt.toUpperCase().includes(a.symbol));
      const nums = prompt.match(/\d+(?:\.\d+)?/g) || [];
      if (!asset || nums.length !== 3 || !/trades?\s*\d+(?:\.\d+)?%\s*above reference/i.test(prompt) || !/executable premium remains above\s*\d/i.test(prompt) || !/sell\s*\$\d/i.test(prompt) || /\bbuy\b/i.test(prompt)) {
        notify(agentEnabled ? t("notify.agentAndRuleFailed") : t("notify.ruleFormatHint"));
        return;
      }
      p = {
        id: crypto.randomUUID(),
        asset: asset.symbol,
        threshold: Number(nums[0]),
        executableThreshold: Number(nums[1]),
        amount: Number(nums[2]),
        enabled: false,
      };
    }
    if (p.amount <= 0 || !assets.some((a) => a.symbol === p!.asset)) {
      notify(t("notify.policyLimits"));
      return;
    }
    setPolicies((ps) => [p!, ...ps]);
    setAnswer(
      JSON.stringify(
        {
          asset: p.asset,
          conditions: [
            {
              metric: "reference_gap",
              operator: ">",
              value: p.threshold / 100,
            },
            {
              metric: "executable_gap",
              operator: ">",
              value: p.executableThreshold / 100,
            },
          ],
          action: { type: "sell", amountUSD: p.amount },
          enabled: false,
          parsedBy,
        },
        null,
        2,
      ),
    );
    if (apiMode !== "offline") {
      try {
        const r = await fetch("/api/policies", {
          method: "POST",
          headers: { "Content-Type": "application/json" },
          body: JSON.stringify(p),
        });
        if (!r.ok) throw Error();
        notify(t("notify.policySaved"));
      } catch {
        notify(t("notify.policySavedLocalOnly"));
      }
    } else notify(t("notify.policySavedDemo"));
  };
  const chart = (a: Asset) =>
    a.history
      .slice(range === "1H" ? -8 : range === "7D" ? -25 : -18)
      .map((value, i) => ({
        time: `${String(i).padStart(2, "0")}:00`,
        value,
        reference: a.reference,
      }));
  const isLive = apiMode === "live";
  const dateLocale = lang === "zh-TW" ? "zh-TW" : "en-US";
  const signalCount = assets.filter((a) => a.score >= 70).length;
  return (
    <div className="shell">
      <aside className={mobile ? "sidebar open" : "sidebar"}>
        <a
          className="brand"
          href="/"
          onClick={(e) => {
            e.preventDefault();
            go("overview");
          }}
        >
          <span className="brand-icon">
            <Radio size={25} />
          </span>
          RWA<span>RADAR</span>
        </a>
        <div className="workspace-wrap">
          <button
            className="workspace"
            onClick={() => setWorkspaceMenuOpen((o) => !o)}
            aria-expanded={workspaceMenuOpen}
          >
            <span className="workspace-icon">
              {(workspaces.find((w) => w.id === activeWorkspace)?.name || DEFAULT_WORKSPACE.name)[0].toUpperCase()}
            </span>
            <div>
              {workspaces.find((w) => w.id === activeWorkspace)?.name || DEFAULT_WORKSPACE.name}
              <small>{t("sidebar.personalWorkspace")}</small>
            </div>
            <ChevronDown size={15} />
          </button>
          {workspaceMenuOpen && (
            <div className="workspace-menu">
              {workspaces.map((w) => (
                <div className="workspace-menu-row" key={w.id}>
                  <button
                    className={w.id === activeWorkspace ? "selected" : ""}
                    onClick={() => {
                      setActiveWorkspace(w.id);
                      setWorkspaceMenuOpen(false);
                    }}
                  >
                    {w.name}
                  </button>
                  {workspaces.length > 1 && (
                    <button
                      className="icon-button"
                      aria-label={t("sidebar.deleteWorkspace")}
                      onClick={() => deleteWorkspace(w.id)}
                    >
                      <X size={13} />
                    </button>
                  )}
                </div>
              ))}
              <div className="workspace-menu-new">
                <input
                  placeholder={t("sidebar.newWorkspacePlaceholder")}
                  value={newWorkspaceName}
                  onChange={(e) => setNewWorkspaceName(e.target.value)}
                  onKeyDown={(e) => e.key === "Enter" && createWorkspace()}
                />
                <button
                  className="icon-button"
                  aria-label={t("sidebar.addWorkspace")}
                  onClick={createWorkspace}
                >
                  <Plus size={14} />
                </button>
              </div>
            </div>
          )}
        </div>
        <div className="nav-label">{t("sidebar.workspaceLabel")}</div>
        <nav>
          {nav.map(([id, labelKey, Icon]) => (
            <button
              key={id}
              className={page === id ? "active" : ""}
              onClick={() => go(id)}
            >
              <Icon size={19} />
              {t(labelKey)}
              {id === "overview" && signalCount > 0 && (
                <span className="nav-count">{signalCount}</span>
              )}
            </button>
          ))}
        </nav>
        <div className="sidebar-bottom">
          <div className="network">
            <i /> BNB Smart Chain <ExternalLink size={13} />
          </div>
          <div className="safety">
            <ShieldCheck size={20} />
            <div>
              {t("sidebar.safetyTitle")}<small>{t("sidebar.safetySubtitle")}</small>
            </div>
          </div>
          <div className="profile">
            <div className="avatar">C</div>
            <div>
              Explorer<small>Demo workspace</small>
            </div>
            <span className="demo-tag">MVP</span>
          </div>
        </div>
      </aside>
      <main>
        <header>
          <div className="breadcrumb">
            <button
              className="mobile-menu"
              onClick={() => setMobile(!mobile)}
              aria-label={t("header.openMenu")}
            >
              <Menu size={20} />
            </button>
            {t("sidebar.workspaceLabel")} <ChevronRight size={13} />{" "}
            <strong>
              {page === "assets"
                ? t("header.assetDetail")
                : t(nav.find((n) => n[0] === page)?.[1] || "header.overview")}
            </strong>
          </div>
          <div className="header-actions">
            <span className="status-dot" />
            <span className="source">{t(`source.${apiMode}`)}</span>
            <span className="header-divider" />
            <button
              className="lang-toggle"
              onClick={() => setLang(lang === "en" ? "zh-TW" : "en")}
              aria-label="Language"
            >
              <Globe size={16} />
              {LANGS.find((l) => l.code === lang)?.label}
            </button>
            <button
              aria-label={t("header.notifications")}
              onClick={() =>
                notify(
                  t(isLive ? "header.notifyToastLive" : "header.notifyToastDemo", {
                    count: signalCount,
                  }),
                )
              }
            >
              <Bell size={18} />
              <b className="notification-dot" />
            </button>
            <button
              className="wallet-button"
              onClick={wallet ? disconnectWallet : connectWallet}
            >
              <Wallet size={16} />
              {wallet ? `${wallet.slice(0, 6)}…${wallet.slice(-4)}` : t("header.connectWallet")}
            </button>
          </div>
        </header>
        <div className="content">
          <div className="page-heading">
            <div>
              <h1>
                {page === "overview"
                  ? t("page.title.overview")
                  : page === "assets"
                    ? t("page.title.assetAnalysis", { symbol: selected?.symbol || "NVDA" })
                    : t(nav.find((n) => n[0] === page)?.[1] || "page.title.overview")}
                <span className="live-pill">
                  <i />
                  {isLive ? "LIVE" : "DEMO"}
                </span>
              </h1>
            </div>
            {page === "overview" && (
              <button className="outline" onClick={exportData}>
                <Download size={16} />
                {t("page.exportData")}
              </button>
            )}
          </div>
          {page === "overview" && (
            <>
              <div className="hero-compact">
                <div className="hero-compact-copy">
                  <div className="hero-kicker">
                    <span /> THE MARKET NEVER SLEEPS
                  </div>
                  <p>{t("hero.copy")}</p>
                </div>
                <div className="hero-stats-strip">
                  <div>
                    <Layers size={14} />
                    <strong>{assets.length}</strong>
                    <small>{t("stats.trackedAssets")}</small>
                  </div>
                  <div>
                    <BarChart3 size={14} />
                    <strong>{compact(assets.reduce((n, a) => n + a.volume, 0))}</strong>
                    <small>{t("stats.volume24h")}</small>
                  </div>
                  <div className={signalCount > 0 ? "orange" : ""}>
                    <Activity size={14} />
                    <strong>{signalCount}</strong>
                    <small>{t("stats.signals")}</small>
                  </div>
                  <div>
                    <Clock size={14} />
                    <strong>{t("stats.marketStatusValue")}</strong>
                    <small>{t("stats.marketStatus")}</small>
                  </div>
                </div>
              </div>
              <div className="main-grid">
                <section className="panel market-panel">
                  <div className="panel-title">
                    <div>
                      <h3>
                        {t("marketPanel.title")} <span className="count">{assets.length}</span>
                      </h3>
                      <p>{t("marketPanel.subtitle")}</p>
                    </div>
                    <div className="view-toggle">
                      <button
                        className={viewMode === "table" ? "selected" : ""}
                        aria-label={t("marketPanel.tableView")}
                        onClick={() => setViewMode("table")}
                      >
                        <List size={16} />
                      </button>
                      <button
                        className={viewMode === "radar" ? "selected" : ""}
                        aria-label={t("marketPanel.radarView")}
                        onClick={() => setViewMode("radar")}
                      >
                        <RadarIcon size={16} />
                      </button>
                    </div>
                    <button
                      className="icon-button"
                      aria-label={t("marketPanel.toggleSort")}
                      onClick={() =>
                        setSort(sort === "score" ? "gap" : "score")
                      }
                    >
                      <Settings2 size={17} />
                    </button>
                  </div>
                  <div className="tabs">
                    <button
                      className={!onlyWatch ? "selected" : ""}
                      onClick={() => setOnlyWatch(false)}
                    >
                      {t("tabs.allAssets")}
                    </button>
                    <button
                      className={onlyWatch ? "selected" : ""}
                      onClick={() => setOnlyWatch(true)}
                    >
                      {t("tabs.watchlist")} <span>{watch.length}</span>
                    </button>
                    <span className="tabs-end">
                      <i />
                      {isLive ? t("tabs.liveSnapshot") : t("tabs.demoSnapshot")}
                    </span>
                  </div>
                  {viewMode === "radar" ? (
                    <RadarScreen
                      assets={filtered}
                      onSelect={(a) => go("assets", a)}
                      emptyLabel={t("table.empty")}
                    />
                  ) : (
                  <>
                  <div className="filters">
                    <label className="search">
                      <Search size={16} />
                      <input
                        placeholder={t("filters.searchPlaceholder")}
                        value={query}
                        onChange={(e) => setQuery(e.target.value)}
                      />
                      <kbd>⌕</kbd>
                    </label>
                    <select
                      aria-label={t("filters.sectorAria")}
                      value={sector}
                      onChange={(e) => setSector(e.target.value)}
                    >
                      <option value={ALL_SECTORS}>{t("filters.allSectors")}</option>
                      {[
                        "Technology",
                        "Automotive",
                        "Consumer",
                        "Financial",
                        "ETF",
                      ].map((s) => (
                        <option key={s} value={s}>{s}</option>
                      ))}
                    </select>
                    <select
                      aria-label={t("filters.providerAria")}
                      value={provider}
                      onChange={(e) => setProvider(e.target.value)}
                    >
                      <option value={ALL_PROVIDERS}>{t("filters.allProviders")}</option>
                      {["xStocks", "Ondo", "bStocks"].map((s) => (
                        <option key={s} value={s}>{s}</option>
                      ))}
                    </select>
                  </div>
                  {agentEnabled && (
                    <div className="ai-filter">
                      <div className="ai-filter-input">
                        <Sparkles size={14} />
                        <input
                          placeholder={t("filters.aiPlaceholder")}
                          value={aiFilterInput}
                          onChange={(e) => setAiFilterInput(e.target.value)}
                          onKeyDown={(e) => {
                            if (e.key === "Enter") applyAiFilter();
                          }}
                        />
                        <button
                          className="outline small"
                          disabled={aiFilterBusy || !aiFilterInput.trim()}
                          onClick={applyAiFilter}
                        >
                          {aiFilterBusy ? t("filters.aiThinking") : t("filters.aiApply")}
                        </button>
                      </div>
                      {aiFilter && (
                        <div className="ai-filter-chip">
                          <Sparkles size={12} />
                          {aiFilter.summary || t("filters.aiActive")}
                          <button onClick={clearAiFilter} aria-label={t("filters.aiClear")}>
                            <X size={12} />
                          </button>
                        </div>
                      )}
                    </div>
                  )}
                  <div className="table-scroll">
                    <table>
                      <thead>
                        <tr>
                          <th>{t("table.asset")}</th>
                          <th>{t("table.price")}</th>
                          <th>
                            <button onClick={() => setSort("gap")}>
                              {t("table.gap")} ↓
                            </button>
                          </th>
                          <th>{t("table.trend24h")}</th>
                          <th>
                            <button onClick={() => setSort("score")}>
                              {t("table.score")} ↓
                            </button>
                          </th>
                          <th />
                        </tr>
                      </thead>
                      <tbody>
                        {filtered.map((a) => (
                          <tr key={a.symbol} onClick={() => go("assets", a)}>
                            <td>
                              <div className="asset-cell">
                                <span className={`asset-logo ${a.symbol}`}>
                                  {a.symbol === "AAPL"
                                    ? "●"
                                    : a.symbol === "MSFT"
                                      ? "⊞"
                                      : a.symbol === "GOOGL"
                                        ? "G"
                                        : a.symbol === "SPY"
                                          ? "S&P"
                                          : a.symbol[0]}
                                </span>
                                <div>
                                  <strong>{a.symbol}</strong>
                                  <small>{a.name}</small>
                                </div>
                              </div>
                            </td>
                            <td>
                              <strong className="mono">
                                {money(
                                  provider === ALL_PROVIDERS
                                    ? a.price
                                    : a.providers.find(
                                        (p) => p.name === provider,
                                      )!.price,
                                )}
                              </strong>
                              <small className="provider-small">
                                {provider === ALL_PROVIDERS
                                  ? t(
                                      a.providers.length === 1
                                        ? "table.providerCountOne"
                                        : "table.providerCount",
                                      { n: a.providers.length },
                                    )
                                  : provider}
                              </small>
                            </td>
                            <td>
                              <span
                                className={gap(a) > 0 ? "positive" : "negative"}
                              >
                                {gap(a) > 0 ? "+" : ""}
                                {gap(a).toFixed(2)}% <ArrowUpRight size={12} />
                              </span>
                              <small className="muted">
                                vs. {money(a.reference)}
                              </small>
                            </td>
                            <td>
                              <Sparkline
                                values={a.history}
                                positive={a.change >= 0}
                              />
                              <small
                                className={
                                  a.change >= 0 ? "positive" : "negative"
                                }
                              >
                                {a.change > 0 ? "+" : ""}
                                {a.change.toFixed(2)}%
                              </small>
                            </td>
                            <td>
                              <span
                                className={`score ${a.score >= 80 ? "high" : a.score >= 60 ? "medium" : "low"}`}
                              >
                                <i />
                                {a.score}
                                <small>/ 100</small>
                              </span>
                            </td>
                            <td>
                              <button
                                className={
                                  watch.includes(a.symbol)
                                    ? "star checked"
                                    : "star"
                                }
                                aria-label={t("table.watchAria", { symbol: a.symbol })}
                                onClick={(e) => {
                                  e.stopPropagation();
                                  setWatch((w) =>
                                    w.includes(a.symbol)
                                      ? w.filter((s) => s !== a.symbol)
                                      : [...w, a.symbol],
                                  );
                                }}
                              >
                                ☆
                              </button>
                            </td>
                          </tr>
                        ))}
                      </tbody>
                    </table>
                    {filtered.length === 0 && (
                      <div className="empty">
                        {t("table.empty")}
                      </div>
                    )}
                  </div>
                  <div className="table-footer">
                    {t("table.footerShowing", { n: filtered.length })}{" "}
                    <span>
                      {t("table.footerNote")}{" "}
                      <ShieldCheck size={12} />
                    </span>
                  </div>
                  </>
                  )}
                </section>
                <div className="right-column">
                  <section className="panel signals">
                    <div className="panel-title">
                      <h3>
                        <span className="orange-dot" /> {t("signals.title")}
                      </h3>
                      <span className="soft-label">TOP 3</span>
                    </div>
                    {[...assets]
                      .sort((a, b) => b.score - a.score)
                      .slice(0, 3)
                      .map((a, i) => {
                        const exp = explanations[a.symbol];
                        return (
                          <div className="signal-card" key={a.symbol}>
                            <button
                              className="signal-card-main"
                              onClick={() => go("assets", a)}
                            >
                              <div>
                                <span
                                  className={`signal-label ${i === 0 ? "urgent" : ""}`}
                                >
                                  {i === 0 ? t("signals.highDeviation") : t("signals.priceDeviation")}
                                </span>
                                <small>{isLive ? t("tabs.liveSnapshot") : t("tabs.demoSnapshot")}</small>
                              </div>
                              <h4>
                                {t("signals.headline", { symbol: a.symbol })} <ArrowUpRight size={15} />
                              </h4>
                              <p>
                                {t("signals.body", { gap: gap(a).toFixed(2) })}
                              </p>
                              <div className="signal-bottom">
                                <span>
                                  {t("table.score")} <b>{a.score}</b>
                                </span>
                                <ChevronRight size={14} />
                              </div>
                            </button>
                            {agentEnabled && (
                              <div className="signal-explain">
                                <button
                                  className="text-link small"
                                  disabled={exp === "loading"}
                                  onClick={() => explainSignal(a.symbol)}
                                >
                                  <Sparkles size={13} />
                                  {exp === "loading"
                                    ? t("signals.explaining")
                                    : t("signals.explain")}
                                </button>
                                {exp && typeof exp === "object" && (
                                  <div className="signal-explanation">
                                    <strong>{exp.headline}</strong>
                                    <p>{exp.explanation}</p>
                                    {exp.caution && (
                                      <p className="signal-caution">
                                        <AlertTriangle size={12} /> {exp.caution}
                                      </p>
                                    )}
                                  </div>
                                )}
                                {exp === "error" && (
                                  <p className="signal-caution">
                                    {t("notify.agentExplainFailed")}
                                  </p>
                                )}
                              </div>
                            )}
                          </div>
                        );
                      })}
                    <button
                      className="text-link"
                      onClick={() => {
                        setOnlyWatch(false);
                        setSort("score");
                      }}
                    >
                      {t("signals.viewAll")} <ArrowRight size={14} />
                    </button>
                  </section>
                  <section className="agent-card">
                    <div className="agent-icon">
                      <Sparkles size={21} />
                    </div>
                    <h3>{t("agentCard.title")}</h3>
                    <p>
                      {t("agentCard.body1")}
                      <br />{t("agentCard.body2")}
                    </p>
                    <button
                      onClick={() => {
                        setStrategyTab("build");
                        go("strategies");
                      }}
                    >
                      {t("agentCard.cta")} <ArrowUpRight size={16} />
                    </button>
                  </section>
                </div>
              </div>
            </>
          )}
          {page === "assets" && selected && (
            <>
              <button className="back" onClick={() => go("overview")}>
                <ArrowLeft size={16} />
                {t("assetDetail.back")}
              </button>
              <div className="detail-header">
                <div className="asset-cell">
                  <span className={`asset-logo ${selected.symbol}`}>
                    {selected.symbol[0]}
                  </span>
                  <div>
                    <h2>
                      {selected.name} <span>{selected.symbol}</span>
                    </h2>
                    <p>{t("assetDetail.sectorLine", { sector: selected.sector })}</p>
                  </div>
                </div>
                <button className="primary" onClick={() => openTrade(selected)}>
                  <Zap size={16} />
                  {t("assetDetail.getQuote")}
                </button>
              </div>
              <div className="stats">
                <Stat
                  title={t("assetDetail.onchainPrice")}
                  value={money(selected.price)}
                  note={t("assetDetail.onchainPriceNote")}
                />
                <Stat
                  title={t("assetDetail.referencePrice")}
                  value={money(selected.reference)}
                  note={t("assetDetail.referencePriceNote")}
                />
                <Stat
                  title={t("assetDetail.referenceGap")}
                  value={`${gap(selected).toFixed(2)}%`}
                  note={t("assetDetail.referenceGapNote")}
                  orange
                />
                <Stat
                  title="Radar Score"
                  value={`${selected.score}`}
                  note={t("assetDetail.radarScoreNote")}
                />
              </div>
              <section className="panel chart-panel">
                <div className="panel-title">
                  <h3>
                    {t("chart.priceComparison")}{" "}
                    <span className="soft-label">
                      {isLive ? "LIVE" : "DEMO"}
                    </span>
                  </h3>
                  <div className="range">
                    {["1H", "24H", "7D"].map((r) => (
                      <button
                        key={r}
                        className={range === r ? "selected" : ""}
                        onClick={() => setRange(r)}
                      >
                        {r}
                      </button>
                    ))}
                  </div>
                </div>
                <ResponsiveContainer width="100%" height={300}>
                  <AreaChart data={chart(selected)}>
                    <defs>
                      <linearGradient
                        id="priceFill"
                        x1="0"
                        y1="0"
                        x2="0"
                        y2="1"
                      >
                        <stop
                          offset="0%"
                          stopColor="#6456ec"
                          stopOpacity={0.25}
                        />
                        <stop
                          offset="100%"
                          stopColor="#6456ec"
                          stopOpacity={0}
                        />
                      </linearGradient>
                    </defs>
                    <CartesianGrid
                      strokeDasharray="3 3"
                      vertical={false}
                      stroke="#e9edf3"
                    />
                    <XAxis
                      dataKey="time"
                      tick={{ fontSize: 11 }}
                      tickLine={false}
                    />
                    <YAxis
                      domain={["dataMin - 2", "dataMax + 2"]}
                      tick={{ fontSize: 11 }}
                      tickLine={false}
                    />
                    <Tooltip />
                    <ReferenceLine
                      y={selected.reference}
                      stroke="#15a593"
                      strokeDasharray="5 5"
                      label="Reference"
                    />
                    <Area
                      type="monotone"
                      dataKey="value"
                      stroke="#6456ec"
                      fill="url(#priceFill)"
                      strokeWidth={3}
                    />
                  </AreaChart>
                </ResponsiveContainer>
                <p className="muted">
                  {t("chart.note")}
                </p>
              </section>
              <div className="two-grid">
                <section className="panel padding">
                  <h3>{t("providers.title")}</h3>
                  <div className="provider-race">
                    {selected.providers.map((p) => {
                      const prices = selected.providers.map((x) => x.price);
                      const lo = Math.min(...prices);
                      const hi = Math.max(...prices);
                      const pct = hi > lo ? ((p.price - lo) / (hi - lo)) * 100 : 50;
                      const isLeader = p.price === hi && hi > lo;
                      return (
                        <div className="provider-lane" key={p.name}>
                          <span className="provider-lane-label">{p.name}</span>
                          <div className="provider-track">
                            <div
                              className={`provider-marker ${isLeader ? "leader" : ""}`}
                              style={{ left: `${pct}%` }}
                            >
                              {isLeader && <Zap size={10} />}
                            </div>
                          </div>
                          <span className="provider-lane-price">{money(p.price)}</span>
                        </div>
                      );
                    })}
                  </div>
                  {selected.providers.map((p) => (
                    <div className="data-row" key={p.name}>
                      <span>
                        <Layers size={16} /> {p.name}
                      </span>
                      <strong>{money(p.price)}</strong>
                      <span className="positive">
                        +{((p.price / selected.reference - 1) * 100).toFixed(2)}
                        %
                      </span>
                    </div>
                  ))}
                  <p className="muted">
                    {t("providers.spread", {
                      pct: (
                        (Math.max(...selected.providers.map((p) => p.price)) /
                          Math.min(...selected.providers.map((p) => p.price)) -
                          1) *
                        100
                      ).toFixed(2),
                    })}
                  </p>
                </section>
                <section className="panel padding">
                  <h3>
                    {t("liquidity.title")} <span className="soft-label">{t("liquidity.modelEstimate")}</span>
                  </h3>
                  {[10, 100, 1000].map((n) => (
                    <div className="data-row" key={n}>
                      <span>{money(n)}</span>
                      <div className="liquidity-bar">
                        <i
                          style={{
                            width: `${Math.min(100, (n / selected.liquidity) * 50000)}%`,
                          }}
                        />
                      </div>
                      <strong>
                        {((n / selected.liquidity) * 100).toFixed(3)}%
                      </strong>
                    </div>
                  ))}
                  <p className="muted">
                    {t("liquidity.note")}
                  </p>
                </section>
              </div>
              <section className="panel padding replay-panel">
                <div className="panel-title">
                  <h3>{t("replay.title")}</h3>
                  {!replayData && (
                    <button
                      className="outline"
                      disabled={replayLoading}
                      onClick={() => loadReplay(selected.symbol)}
                    >
                      <History size={16} />
                      {replayLoading ? t("replay.loading") : t("replay.trigger")}
                    </button>
                  )}
                </div>
                {!replayData && !replayLoading && (
                  <p className="muted">{t("replay.hint")}</p>
                )}
                {replayData && (
                  <>
                    <div
                      className={`replay-headline ${replayData.resolved ? "resolved" : "persisting"}`}
                    >
                      <strong>
                        {t("replay.peakLine", {
                          symbol: replayData.symbol,
                          pct: Math.abs(replayData.peakGapPercent).toFixed(1),
                          when: new Date(replayData.peakAt).toLocaleString(dateLocale),
                        })}
                      </strong>
                      <span className="soft-label">
                        {replayData.resolved ? t("replay.resolved") : t("replay.persisting")}
                      </span>
                    </div>
                    <ResponsiveContainer width="100%" height={200}>
                      <AreaChart
                        data={replayData.points.map((p) => ({
                          time: new Date(p.t).toLocaleTimeString(dateLocale, {
                            hour: "2-digit",
                            minute: "2-digit",
                          }),
                          price: p.price,
                          reference: p.reference,
                        }))}
                      >
                        <defs>
                          <linearGradient id="replayFill" x1="0" y1="0" x2="0" y2="1">
                            <stop offset="0%" stopColor="#e9a06c" stopOpacity={0.4} />
                            <stop offset="100%" stopColor="#e9a06c" stopOpacity={0} />
                          </linearGradient>
                        </defs>
                        <CartesianGrid vertical={false} stroke="#f0f2f6" />
                        <XAxis dataKey="time" tick={{ fontSize: 11 }} interval="preserveStartEnd" />
                        <YAxis hide domain={["dataMin", "dataMax"]} />
                        <Tooltip formatter={(v: number) => money(v)} />
                        <Area
                          type="monotone"
                          dataKey="reference"
                          stroke="#15a593"
                          fill="none"
                          strokeDasharray="5 5"
                          strokeWidth={2}
                        />
                        <Area
                          type="monotone"
                          dataKey="price"
                          stroke="#e9a06c"
                          fill="url(#replayFill)"
                          strokeWidth={3}
                          animationDuration={1800}
                        />
                      </AreaChart>
                    </ResponsiveContainer>
                    <p className="muted">{t("replay.caveat")}</p>
                  </>
                )}
              </section>
            </>
          )}
          {page === "strategies" && (
            <>
              <div className="tabs">
                <button
                  className={strategyTab === "build" ? "selected" : ""}
                  onClick={() => setStrategyTab("build")}
                >
                  {t("strategies.tabBuild")}
                </button>
                <button
                  className={strategyTab === "manage" ? "selected" : ""}
                  onClick={() => setStrategyTab("manage")}
                >
                  {t("strategies.tabManage")} <span>{policies.length}</span>
                </button>
              </div>
              {strategyTab === "build" ? (
                <div className="two-grid">
                  <section className="panel padding agent-builder">
                    <div className="agent-icon">
                      <Sparkles />
                    </div>
                    <h2>{t("agentPage.title")}</h2>
                    <p className="muted">
                      {agentEnabled ? t("agentPage.descLLM") : t("agentPage.descRule")}
                    </p>
                    {agentEnabled ? (
                      <div className="agent-chat">
                        {chatMessages.length === 0 ? (
                          <p className="agent-chat-hint">{t("agentChat.hint")}</p>
                        ) : (
                          <div className="agent-chat-log">
                            {chatMessages.map((m, i) => (
                              <div className={`agent-chat-msg ${m.role}`} key={i}>
                                {m.content}
                              </div>
                            ))}
                            {chatBusy && (
                              <div className="agent-chat-msg assistant loading">
                                {t("agentChat.thinking")}
                              </div>
                            )}
                            {chatProposal && (
                              <button className="primary full-width" onClick={confirmChatProposal}>
                                <Check size={16} />
                                {t("agentChat.confirm")}
                              </button>
                            )}
                          </div>
                        )}
                        <div className="agent-chat-input">
                          <input
                            placeholder={t("agentChat.placeholder")}
                            value={chatInput}
                            onChange={(e) => setChatInput(e.target.value)}
                            onKeyDown={(e) => {
                              if (e.key === "Enter") sendChatMessage();
                            }}
                          />
                          <button
                            className="primary"
                            disabled={chatBusy || !chatInput.trim()}
                            onClick={sendChatMessage}
                          >
                            <ArrowRight size={16} />
                          </button>
                        </div>
                        {chatMessages.length > 0 && (
                          <button className="text-link small" onClick={resetChat}>
                            {t("agentChat.reset")}
                          </button>
                        )}
                      </div>
                    ) : (
                      <>
                        <label htmlFor="prompt">{t("agentPage.yourStrategy")}</label>
                        <textarea
                          id="prompt"
                          value={prompt}
                          onChange={(e) => setPrompt(e.target.value)}
                        />
                        <button className="primary" onClick={createPolicy}>
                          <Sparkles size={16} />
                          {t("agentPage.parseButton")}
                        </button>
                        {answer && <pre>{answer}</pre>}
                      </>
                    )}
                  </section>
                  <section className="panel padding">
                    <h3>{t("agentPage.boundariesTitle")}</h3>
                    {[
                      t("agentPage.boundary1"),
                      t("agentPage.boundary2"),
                      t("agentPage.boundary3"),
                      t("agentPage.boundary4"),
                      t("agentPage.boundary5"),
                      t("agentPage.boundary6"),
                    ].map((s) => (
                      <div className="data-row" key={s}>
                        <ShieldCheck size={18} />
                        {s}
                        <Check size={16} />
                      </div>
                    ))}
                    <button
                      className="outline"
                      onClick={() => openTrade(assets[0])}
                    >
                      {t("agentPage.manualSimulate")} <ArrowRight size={16} />
                    </button>
                  </section>
                </div>
              ) : (
                <section className="panel padding">
                  <div className="panel-title">
                    <h3>{t("policiesPage.title")}</h3>
                    <button
                      className="primary"
                      onClick={() => setStrategyTab("build")}
                    >
                      <Plus size={16} />
                      {t("policiesPage.newPolicy")}
                    </button>
                  </div>
                  {policies.length === 0 ? (
                    <div className="empty">
                      <Settings2 size={36} />
                      <h3>{t("policiesPage.emptyTitle")}</h3>
                      <p>{t("policiesPage.emptyBody")}</p>
                    </div>
                  ) : (
                    policies.map((p) => (
                      <div className="data-row" key={p.id}>
                        <div>
                          <strong>{t("policiesPage.watchLabel", { symbol: p.asset })}</strong>
                          <small>
                            {t("policiesPage.thresholdLine", {
                              threshold: p.threshold,
                              executableThreshold: p.executableThreshold,
                              amount: money(p.amount),
                            })}
                          </small>
                        </div>
                        <span className="soft-label">
                          {p.enabled ? t("policiesPage.watching") : t("policiesPage.disabled")}
                        </span>
                        <button
                          className="outline"
                          onClick={() =>
                            setPolicies((ps) =>
                              ps.map((x) =>
                                x.id === p.id ? { ...x, enabled: !x.enabled } : x,
                              ),
                            )
                          }
                        >
                          {p.enabled ? t("policiesPage.disable") : t("policiesPage.enableLocal")}
                        </button>
                        <button
                          className="icon-button"
                          aria-label={t("policiesPage.deleteAria")}
                          onClick={() =>
                            setPolicies((ps) => ps.filter((x) => x.id !== p.id))
                          }
                        >
                          <X size={16} />
                        </button>
                      </div>
                    ))
                  )}
                  <p className="muted">
                    {t("policiesPage.footnote")}
                  </p>
                </section>
              )}
            </>
          )}
          {page === "portfolio" && (
            <section className="panel padding">
              {!wallet ? (
                <div className="empty">
                  <Wallet size={42} />
                  <h2>{t("portfolioPage.title")}</h2>
                  <p>{t("portfolioPage.notConnected")}</p>
                  <button className="primary" onClick={connectWallet}>
                    {t("header.connectWallet")} <ArrowUpRight size={16} />
                  </button>
                  <button
                    className="text-link"
                    onClick={() => go("overview")}
                  >
                    {t("portfolioPage.exploreMarket")}
                  </button>
                </div>
              ) : (
                <>
                  <div className="panel-title">
                    <h3>{t("portfolioPage.holdingsTitle")}</h3>
                    <span className="soft-label">{wallet}</span>
                  </div>
                  <p className="muted">
                    {t("portfolioPage.readonlyNote")}
                  </p>
                  {portfolioLoading ? (
                    <p className="muted">{t("portfolioPage.loading")}</p>
                  ) : apiMode === "offline" ? (
                    <div className="empty">
                      <Wallet size={36} />
                      <h3>{t("portfolioPage.apiOfflineTitle")}</h3>
                      <p>{t("portfolioPage.apiOfflineBody")}</p>
                    </div>
                  ) : !portfolio ? (
                    <div className="empty">
                      <Wallet size={36} />
                      <h3>{t("portfolioPage.noDataTitle")}</h3>
                      <p>{t("portfolioPage.noDataBody")}</p>
                    </div>
                  ) : portfolio.length === 0 ? (
                    <div className="empty">
                      <Wallet size={36} />
                      <h3>{t("portfolioPage.emptyHoldingsTitle")}</h3>
                    </div>
                  ) : (
                    portfolio.map((t2) => (
                      <div
                        className="data-row"
                        key={t2.symbol + t2.contractAddress}
                      >
                        <strong>{t2.symbol}</strong>
                        <span>{t2.balance}</span>
                        {t2.usdPrice && (
                          <span>
                            {money(Number(t2.balance) * Number(t2.usdPrice))}
                          </span>
                        )}
                      </div>
                    ))
                  )}
                </>
              )}
            </section>
          )}
          {page === "activity" && (
            <>
              <div className="tabs">
                <button
                  className={activityTab === "simulations" ? "selected" : ""}
                  onClick={() => setActivityTab("simulations")}
                >
                  {t("activity.tabSimulations")}
                </button>
                <button
                  className={activityTab === "realtx" ? "selected" : ""}
                  onClick={() => setActivityTab("realtx")}
                >
                  {t("activity.tabRealTx")} <span>{realTx.length}</span>
                </button>
                <button
                  className={activityTab === "developer" ? "selected" : ""}
                  onClick={() => setActivityTab("developer")}
                >
                  {t("activity.tabDeveloper")}
                </button>
              </div>
              {activityTab === "simulations" && (
                <section className="panel padding">
                  {history.length > 0 && (() => {
                    const totalVolume = history.reduce((n, h) => n + h.amountUSD, 0);
                    const uniqueAssets = new Set(history.map((h) => h.asset));
                    const caughtSymbols = new Set(
                      history
                        .filter((h) => (assets.find((a) => a.symbol === h.asset)?.score ?? 0) >= 70)
                        .map((h) => h.asset),
                    );
                    const bestCatch = history.reduce<{ symbol: string; score: number } | undefined>(
                      (best, h) => {
                        const score = assets.find((a) => a.symbol === h.asset)?.score;
                        if (score != null && (!best || score > best.score)) {
                          return { symbol: h.asset, score };
                        }
                        return best;
                      },
                      undefined,
                    );
                    const simDates = new Set(history.map((h) => new Date(h.createdAt).toDateString()));
                    let streak = 0;
                    const cursor = new Date();
                    while (simDates.has(cursor.toDateString())) {
                      streak++;
                      cursor.setDate(cursor.getDate() - 1);
                    }
                    return (
                      <div className="stats scoreboard">
                        <div className="stat">
                          <div className="stat-top">
                            <span>{t("scoreboard.simulations")}</span>
                            <History size={16} />
                          </div>
                          <div className="stat-value">{history.length}</div>
                          <p>{t("scoreboard.uniqueAssets", { n: uniqueAssets.size })}</p>
                        </div>
                        <div className="stat">
                          <div className="stat-top">
                            <span>{t("scoreboard.volume")}</span>
                            <BarChart3 size={16} />
                          </div>
                          <div className="stat-value">{compact(totalVolume)}</div>
                          <p>{t("scoreboard.volumeNote")}</p>
                        </div>
                        <div className="stat orange">
                          <div className="stat-top">
                            <span>{t("scoreboard.caught")}</span>
                            <Activity size={16} />
                          </div>
                          <div className="stat-value">{caughtSymbols.size}</div>
                          <p>{t("scoreboard.caughtNote")}</p>
                        </div>
                        <div className="stat">
                          <div className="stat-top">
                            <span>{t("scoreboard.streak")}</span>
                            <Zap size={16} />
                          </div>
                          <div className="stat-value">
                            {streak}
                            <small>{t("scoreboard.days")}</small>
                          </div>
                          <p>
                            {bestCatch
                              ? t("scoreboard.bestCatch", { symbol: bestCatch.symbol, score: bestCatch.score })
                              : t("scoreboard.noBestCatch")}
                          </p>
                        </div>
                      </div>
                    );
                  })()}
                  {history.length === 0 ? (
                    <div className="empty">
                      <History size={36} />
                      <h3>{t("historyPage.emptyTitle")}</h3>
                      <p>{t("historyPage.emptyBody")}</p>
                      <button
                        className="primary"
                        onClick={() => openTrade(assets[0])}
                      >
                        {t("historyPage.simulateNvda")}
                      </button>
                    </div>
                  ) : (
                    history.map((s) => (
                      <div className="data-row" key={s.id}>
                        <span className="transaction-icon">
                          <ArrowDownLeft size={18} />
                        </span>
                        <div>
                          <strong>
                            {s.asset} · {money(s.amountUSD)}
                          </strong>
                          <small>
                            {new Date(s.createdAt).toLocaleString(dateLocale)}
                          </small>
                        </div>
                        <span className={s.success ? "positive" : "negative"}>
                          {s.success ? t("historyPage.passed") : t("historyPage.blocked")}
                        </span>
                        {s.mode === "live-sign" && (
                          <span className="soft-label danger">{t("historyPage.liveSignTag")}</span>
                        )}
                        <span>{money(s.expectedOutput)}</span>
                      </div>
                    ))
                  )}
                </section>
              )}
              {activityTab === "realtx" && (
                <section className="panel padding">
                  {realTx.length === 0 ? (
                    <div className="empty">
                      <ArrowUpRight size={36} />
                      <h3>{t("activity.realTxEmptyTitle")}</h3>
                      <p>{t("activity.realTxEmptyBody")}</p>
                    </div>
                  ) : (
                    realTx.map((r) => (
                      <div className="data-row" key={r.id}>
                        <span className="transaction-icon">
                          <ArrowUpRight size={18} />
                        </span>
                        <div>
                          <strong>
                            <a
                              href={`https://bscscan.com/tx/${r.txHash}`}
                              target="_blank"
                              rel="noreferrer"
                            >
                              {r.txHash.slice(0, 10)}…
                            </a>
                          </strong>
                          <small>
                            {new Date(r.createdAt).toLocaleString(dateLocale)}
                          </small>
                        </div>
                        <span className="soft-label">{r.status}</span>
                        <span>chain {r.chainId}</span>
                      </div>
                    ))
                  )}
                </section>
              )}
              {activityTab === "developer" && (
                <section className="panel padding">
                  <p className="muted">
                    {t("telemetryPage.subtitle")}
                  </p>
                  {telemetry.length === 0 ? (
                    <div className="empty">
                      <BarChart3 size={36} />
                      <h3>{t("telemetryPage.emptyTitle")}</h3>
                      <p>{t("telemetryPage.emptyBody")}</p>
                    </div>
                  ) : (
                    telemetry.map((tm) => (
                      <div className="data-row" key={tm.endpoint}>
                        <code>{tm.endpoint}</code>
                        <span>{tm.requests} requests</span>
                        <span>{tm.successRate.toFixed(1)}% success</span>
                        <span>{tm.latencyMs.toFixed(1)} ms</span>
                      </div>
                    ))
                  )}
                </section>
              )}
            </>
          )}
          <footer>
            <span>
              <Radio size={14} /> RWA RADAR <i /> See the signal. Understand the
              risk.
            </span>
            <span>
              {t("footer.disclaimer")} <span className="footer-dot">●</span> BNB
              Chain ecosystem
            </span>
          </footer>
        </div>
      </main>
      {toast && (
        <div className="toast" role="status">
          <ShieldCheck size={18} />
          {toast}
          <button onClick={() => setToast("")} aria-label={t("toast.closeAria")}>
            <X size={16} />
          </button>
        </div>
      )}
      {trade && (
        <div className="modal-backdrop" onClick={() => setTrade(undefined)}>
          <section
            className="modal"
            role="dialog"
            aria-modal="true"
            aria-label={t("header.assetDetail")}
            onClick={(e) => e.stopPropagation()}
          >
            <div className="panel-title">
              <h2>{t("tradeModal.title", { symbol: trade.symbol })}</h2>
              <button
                className="icon-button"
                onClick={() => setTrade(undefined)}
                aria-label={t("tradeModal.closeAria")}
              >
                <X />
              </button>
            </div>
            <p className="muted">
              {isLive && wallet
                ? t("tradeModal.liveWalletNote")
                : t("tradeModal.demoNote")}
            </p>

            <div className="step-label">1 · {t("tradeModal.stepSetup")}</div>
            <label>
              {t("tradeModal.issuer")}
              <select
                value={tradeProvider}
                onChange={(e) => {
                  setTradeProvider(e.target.value);
                  setQuote(undefined);
                  setSim(undefined);
                  setSignResult(undefined);
                }}
              >
                {trade.providers.map((p) => (
                  <option key={p.name} value={p.name}>
                    {p.name} · {money(p.price)}
                  </option>
                ))}
              </select>
            </label>
            <label>
              {t("tradeModal.side")}
              <select
                value={side}
                onChange={(e) => {
                  setSide(e.target.value);
                  setQuote(undefined);
                  setSim(undefined);
                }}
              >
                <option value="sell">{t("tradeModal.sell")}</option>
                <option value="buy">{t("tradeModal.buy")}</option>
              </select>
            </label>
            <label>
              {t("tradeModal.amountUsd")}
              <input
                type="number"
                min="0.01"
                step="0.01"
                value={amount}
                onChange={(e) => {
                  setAmount(+e.target.value);
                  setSim(undefined);
                  setQuote(undefined);
                }}
              />
            </label>

            <div className="step-label">2 · {t("tradeModal.stepQuote")}</div>
            <button
              className="outline full-width"
              disabled={busy}
              onClick={() => runTrade("quote")}
            >
              {t("tradeModal.getQuote")}
            </button>
            {quote && (
              <div className="quote-summary">
                <p className="quote-plain">
                  {t("tradeModal.quotePlain", {
                    output: money(quote.expectedOutput),
                    gas: money(quote.gasEstimate),
                  })}
                </p>
                <button
                  className="text-link small"
                  onClick={() => setShowQuoteDetails((v) => !v)}
                >
                  {showQuoteDetails
                    ? t("tradeModal.hideDetails")
                    : t("tradeModal.showDetails")}
                </button>
                {showQuoteDetails && (
                  <>
                    <div className="data-row">
                      <span>{t("tradeModal.expectedOutput")}</span>
                      <strong>{money(quote.expectedOutput)}</strong>
                    </div>
                    <div className="data-row">
                      <span>{t("tradeModal.priceImpact")}</span>
                      <strong>{(quote.priceImpact * 100).toFixed(4)}%</strong>
                    </div>
                    <div className="data-row">
                      <span>{t("tradeModal.estGas")}</span>
                      <strong>{money(quote.gasEstimate)}</strong>
                    </div>
                  </>
                )}
              </div>
            )}

            <div className="step-label">3 · {t("tradeModal.stepAct")}</div>
            <div className="action-lane">
              <div className="action-lane-head">
                <strong>{t("tradeModal.laneDemo")}</strong>
                <span className="soft-label">{t("tradeModal.laneDemoTag")}</span>
              </div>
              <button
                className="primary full-width"
                disabled={busy || !quote}
                onClick={() => runTrade("simulate")}
              >
                {busy ? t("tradeModal.processing") : t("tradeModal.runSimulation")}
              </button>
              {sim && (
                <div
                  className={`simulation-result ${sim.success ? "passed" : "blocked"}`}
                >
                  <strong>{sim.success ? t("tradeModal.simPassed") : t("tradeModal.simBlocked")}</strong>
                  <p>{sim.success ? sim.reason : t(`error.${sim.reasonCode}`, sim.vars)}</p>
                  <small>{t("tradeModal.simId", { id: sim.id })}</small>
                </div>
              )}
            </div>

            {isLive && wallet ? (
              <div className="action-lane real">
                <div className="action-lane-head">
                  <strong>{t("tradeModal.laneReal")}</strong>
                  <span className="soft-label danger">{t("tradeModal.laneRealTag")}</span>
                </div>
                <p className="muted small">
                  {t("tradeModal.realFundsWarningBody", {
                    wallet: `${wallet.slice(0, 6)}…${wallet.slice(-4)}`,
                    amount: money(amount),
                  })}
                </p>
                <label className="confirm-row">
                  <input
                    type="checkbox"
                    checked={confirmRealFunds}
                    onChange={(e) => setConfirmRealFunds(e.target.checked)}
                  />
                  {t("tradeModal.confirmRealFunds")}
                </label>
                <button
                  className="danger-button full-width"
                  disabled={signing || !quote || !confirmRealFunds}
                  onClick={signAndSend}
                >
                  {signing ? t("tradeModal.signSending") : t("tradeModal.signAndSend")}
                </button>
                {signResult && (
                  <div className="simulation-result passed">
                    <strong>{t("tradeModal.realTxSent")}</strong>
                    <p>
                      {t("tradeModal.txHash")}
                      <a
                        href={`https://bscscan.com/tx/${signResult.txHash}`}
                        target="_blank"
                        rel="noreferrer"
                      >
                        {signResult.txHash.slice(0, 10)}…
                      </a>
                    </p>
                  </div>
                )}
              </div>
            ) : (
              <p className="muted small disabled-hint">
                {!wallet
                  ? t("tradeModal.disabledNeedWallet")
                  : t("tradeModal.disabledNeedLive")}
              </p>
            )}
          </section>
        </div>
      )}
    </div>
  );
}
function Stat({
  title,
  value,
  note,
  icon,
  extra,
  orange,
}: {
  title: string;
  value: string;
  note: string;
  icon?: React.ReactNode;
  extra?: string;
  orange?: boolean;
}) {
  return (
    <section className={`stat ${orange ? "orange" : ""}`}>
      <div className="stat-top">
        {title}
        <span>{icon}</span>
      </div>
      <div className="stat-value">
        {value}
        <small>{extra}</small>
      </div>
      <p>
        {orange && <span className="orange-dot" />}
        {note}
      </p>
    </section>
  );
}
function Sparkline({
  values,
  positive,
}: {
  values: number[];
  positive: boolean;
}) {
  const min = Math.min(...values),
    max = Math.max(...values);
  return (
    <svg width="84" height="24" viewBox="0 0 84 24" aria-label="price trend">
      <polyline
        points={values
          .map(
            (n, i) =>
              `${(i / (values.length - 1)) * 84},${22 - ((n - min) / (max - min || 1)) * 20}`,
          )
          .join(" ")}
        fill="none"
        stroke={positive ? "#1ca68b" : "#e06f76"}
        strokeWidth="1.7"
      />
    </svg>
  );
}
