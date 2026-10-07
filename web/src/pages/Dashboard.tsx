import { Show, createMemo, createResource, createSignal, onCleanup, onMount } from "solid-js";
import { ErrorState, Loading, Segmented } from "@/components/ui";
import { tpl } from "@/i18n";
import { overviewText } from "@/i18n/overview";
import { api } from "@/lib/api";
import { onServerEvent } from "@/lib/events";
import { fmtTime } from "@/lib/format";
import { onPortfolioEvent } from "@/lib/portfolio";
import "@/styles/overview.css";
import { ASSET_RANGES, AssetChart, type AssetRange } from "./overview/AssetChart";
import { Holdings } from "./overview/Holdings";
import { KpiStrip } from "./overview/KpiStrip";
import { MarketBoard } from "./overview/MarketBoard";
import { RecentPlans, Restrictions, SectorAllocation, StrategyCard } from "./overview/SideCards";
import { TodayPlans } from "./overview/TodayPlans";
import { store, stored, throttle } from "./overview/util";

const PERIODS = ["1D", "5D", "1M", "3M", "6M", "YTD", "1Y"] as const;
type Period = (typeof PERIODS)[number];

export default function Dashboard() {
  const t = overviewText;
  const [dashboard, { refetch }] = createResource(() => api.dashboard());
  const [period, setPeriod] = createSignal<Period>(stored("board.period", "1D", PERIODS));
  const [board, { refetch: refetchBoard }] = createResource(period, (p) => api.board(p));
  const [view, setView] = createSignal(stored("board.view", "board", ["board", "asset"] as const));
  const [symbol, setSymbol] = createSignal("NVDA");
  const [range, setRange] = createSignal<AssetRange>(stored("asset.range", "6M", ASSET_RANGES));
  const [hidden, setHidden] = createSignal<Set<string>>(new Set());
  let chartCard!: HTMLDivElement;

  onMount(() => {
    try {
      const saved = localStorage.getItem("hone-quant.board.symbol");
      if (saved && /^[A-Z.\-]{1,10}$/.test(saved)) setSymbol(saved);
    } catch {
      /* storage unavailable */
    }
    const refreshAll = throttle(() => {
      refetch();
      refetchBoard();
    }, 4000);
    const refreshQuotes = throttle(() => {
      refetch();
      refetchBoard();
    }, 30_000);
    const offs = [
      onPortfolioEvent(["plan", "account", "strategy", "settings", "universe"], refreshAll),
      onServerEvent(["quotes"], refreshQuotes),
    ];
    onCleanup(() => offs.forEach((off) => off()));
  });

  const data = () => dashboard.latest;
  const targets = () => data()?.targets?.weights ?? null;
  const investedTarget = createMemo(() => {
    const id = data()?.targets?.plan_id;
    const plan = id ? [...(data()?.plans ?? []), ...(data()?.recent_plans ?? [])].find((p) => p.id === id) : undefined;
    if (plan?.invested_target != null) return plan.invested_target;
    const w = targets();
    return w ? Object.values(w).reduce((a, b) => a + b, 0) : null;
  });
  const position = () => data()?.valuation.positions.find((p) => p.symbol === symbol());

  const pick = (next: string) => {
    setSymbol(next);
    store("board.symbol", next);
    setView("asset");
    store("board.view", "asset");
    chartCard?.scrollIntoView({ behavior: "smooth", block: "start" });
  };

  const toggleSector = (id: string) => {
    const all = (board.latest?.sectors ?? []).map((s) => s.id);
    setHidden((current) => {
      // From "all visible", a click focuses on that sector; afterwards clicks toggle.
      if (current.size === 0) return new Set(all.filter((s) => s !== id));
      const next = new Set(current);
      if (next.has(id)) next.delete(id);
      else next.add(id);
      return next.size >= all.length ? new Set<string>() : next;
    });
  };

  return (
    <Show when={data()} fallback={dashboard.error ? <ErrorState error={dashboard.error} onRetry={refetch} /> : <Loading />}>
      {(d) => (
        <div class="stack overview">
          <KpiStrip valuation={d().valuation} invested_target={investedTarget()} universe={board.latest?.items.length ?? 64} />

          <div class="card market-card" ref={chartCard}>
            <div class="card-head market-head">
              <div style={{ "min-width": 0 }}>
                <h2>{t().board.title}</h2>
                <div class="sub">
                  <Show when={view() === "board" && board.latest} fallback={<span>{t().board.sub_asset}</span>}>
                    {tpl(t().board.sub_board, {
                      n: board.latest!.items.length,
                      range: board.latest!.from === board.latest!.to ? board.latest!.to : `${board.latest!.from} – ${board.latest!.to}`,
                    })}
                    <Show when={board.latest!.live}>
                      {" · "}
                      <span class="live-dot" />
                      {t().board.sub_live} · {tpl(t().board.as_of, { time: fmtTime(board.latest!.as_of) })}
                    </Show>
                  </Show>
                </div>
              </div>
              <div class="market-controls">
              <Segmented
                value={view()}
                onChange={(v) => {
                  setView(v);
                  store("board.view", v);
                }}
                options={[
                  { value: "board", label: t().board.tab_board },
                  { value: "asset", label: t().board.tab_asset },
                ]}
              />
              <Show when={view() === "board"}>
                <Segmented
                  value={period()}
                  onChange={(v) => {
                    setPeriod(v);
                    store("board.period", v);
                  }}
                  options={PERIODS.map((p) => ({ value: p, label: p }))}
                  label={t().board.period}
                />
              </Show>
              </div>
            </div>
            <div class="card-body market-body">
              <Show when={view() === "board"}>
                <Show when={board.latest} fallback={board.error ? <ErrorState error={board.error} onRetry={refetchBoard} /> : <Loading />}>
                  <MarketBoard
                    board={board.latest!}
                    hidden={hidden()}
                    onToggleSector={toggleSector}
                    onShowAll={() => setHidden(new Set<string>())}
                    onPick={pick}
                    busy={board.loading}
                  />
                </Show>
              </Show>
              <Show when={view() === "asset"}>
                <AssetChart
                  symbol={symbol()}
                  range={range()}
                  board={board.latest}
                  position={position()}
                  target={targets()?.[symbol()] ?? (targets() ? 0 : null)}
                  onSymbol={(s) => {
                    setSymbol(s);
                    store("board.symbol", s);
                  }}
                  onRange={(r) => {
                    setRange(r);
                    store("asset.range", r);
                  }}
                />
              </Show>
            </div>
          </div>

          <div class="overview-row">
            <TodayPlans market={d().market} plans={d().plans} onChanged={() => refetch()} />
            <SectorAllocation valuation={d().valuation} targets={targets()} board={board.latest} />
            <div class="stack" style={{ "min-width": 0 }}>
              <StrategyCard dashboard={d()} />
              <Restrictions restrictions={d().restrictions} />
              <RecentPlans plans={d().recent_plans} />
            </div>
          </div>

          <Holdings valuation={d().valuation} targets={targets()} board={board.latest} onPick={pick} />
        </div>
      )}
    </Show>
  );
}
