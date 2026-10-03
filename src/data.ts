export type Asset = {
  symbol: string;
  name: string;
  sector: string;
  price: number;
  reference: number;
  change: number;
  volume: number;
  liquidity: number;
  score: number;
  providers: { name: string; price: number }[];
  history: number[];
};
export const seed: Asset[] = [
  ["NVDA", "NVIDIA", "Technology", 193.4, 188.2, 3.24, 2840000, 1240000, 94],
  ["TSLA", "Tesla", "Automotive", 258.62, 252.1, -1.82, 1960000, 860000, 87],
  ["AAPL", "Apple", "Technology", 232.85, 230.54, 1.16, 3210000, 2450000, 72],
  [
    "MSFT",
    "Microsoft",
    "Technology",
    428.76,
    426.8,
    0.84,
    1540000,
    1820000,
    58,
  ],
  ["AMZN", "Amazon", "Consumer", 218.34, 219.6, -0.62, 980000, 920000, 46],
  [
    "GOOGL",
    "Alphabet",
    "Technology",
    192.18,
    191.72,
    1.42,
    1240000,
    1640000,
    35,
  ],
  ["COIN", "Coinbase", "Financial", 284.56, 281.2, 4.28, 1720000, 680000, 79],
  ["SPY", "S&P 500 ETF", "ETF", 602.45, 601.8, 0.38, 4620000, 3850000, 24],
].map((r, i) => ({
  symbol: r[0] as string,
  name: r[1] as string,
  sector: r[2] as string,
  price: r[3] as number,
  reference: r[4] as number,
  change: r[5] as number,
  volume: r[6] as number,
  liquidity: r[7] as number,
  score: r[8] as number,
  providers: [
    { name: "xStocks", price: r[3] as number },
    { name: "Ondo", price: +((r[4] as number) * 1.008).toFixed(2) },
    { name: "bStocks", price: +((r[4] as number) * 1.01).toFixed(2) },
  ],
  history: Array.from(
    { length: 25 },
    (_, j) =>
      +(
        (r[4] as number) +
        Math.sin(j * 0.7 + i) * 0.8 +
        (j / 24) * ((r[3] as number) - (r[4] as number))
      ).toFixed(2),
  ),
}));
export const money = (n: number) =>
  new Intl.NumberFormat("en-US", {
    style: "currency",
    currency: "USD",
    maximumFractionDigits: 2,
  }).format(n);
export const compact = (n: number) =>
  "$" +
  new Intl.NumberFormat("en-US", {
    notation: "compact",
    maximumFractionDigits: 2,
  }).format(n);
export const gap = (a: Asset) => (a.price / a.reference - 1) * 100;
