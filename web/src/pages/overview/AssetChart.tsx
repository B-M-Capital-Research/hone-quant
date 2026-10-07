import { A } from "@solidjs/router";
import { For, Show, createMemo, createResource, createSignal, onCleanup, onMount } from "solid-js";
import { Chart } from "@/components/Chart";
import { Icon } from "@/components/Icon";
import { Empty, ErrorState, Pct, Segmented, WeightBar } from "@/components/ui";
import { locale, pick } from "@/i18n";
import { overviewText } from "@/i18n/overview";
import { api } from "@/lib/api";
import { aggregateBars, candleOption } from "@/lib/charts/builders";
import { readPalette } from "@/lib/charts/echarts";
import { fmtMoney, fmtPct, fmtPrice, fmtQty, polarity, toNumber } from "@/lib/format";
import { onPortfolioEvent } from "@/lib/portfolio";
import type { Board, PositionView } from "@/lib/types";
import { sectorLabel, throttle } from "./util";

export const ASSET_RANGES = ["1D", "5D", "1M", "3M", "6M", "1Y", "5Y", "MAX"] as const;
export type AssetRange = (typeof ASSET_RANGES)[number];

export function AssetChart(props: {
  symbol: string;
  range: AssetRange;
  board: Board | undefined;
  position: PositionView | undefined;
  target: number | null;
  onSymbol: (symbol: string) => void;
  onRange: (range: AssetRange) => void;
}) {
  const t = overviewText;
  const [bars, { refetch }] = createResource(
    () => [props.symbol, props.range] as const,
    ([symbol, range]) => api.bars(symbol, range),
  );

  onMount(() => {
    const refresh = throttle(() => refetch(), 20_000);
    const off = onPortfolioEvent(["quotes", "account"], () => {
      if (props.range === "1D" || props.range === "5D" || bars.latest?.live) refresh();
    });
    onCleanup(off);
  });

  // Symbols in board order (sector, then period performance) for the picker and stepping.
  const groups = createMemo(() => {
    const b = props.board;
    if (!b) return [];
    return b.sectors
      .map((sector) => ({
        sector,
        items: b.items.filter((i) => i.sector_id === sector.id).sort((x, y) => x.symbol.localeCompare(y.symbol)),
      }))
      .filter((g) => g.items.length);
  });
  const order = createMemo(() => groups().flatMap((g) => g.items.map((i) => i.symbol)));
  const step = (delta: number) => {
    const list = order();
    if (!list.length) return;
    const idx = list.indexOf(props.symbol);
    props.onSymbol(list[(idx + delta + list.length) % list.length]);
  };

  const data = () => bars.latest;
  const quote = () => data()?.quote ?? null;
  const asset = () => data()?.asset ?? null;
  const lastPrice = () => quote()?.price ?? data()?.bars.at(-1)?.c ?? null;
  // Five years of daily candles would be under a pixel wide: 5Y shows weeks and MAX months.
  const series = createMemo(() => {
    const d = data();
    if (!d) return null;
    const period = d.interval !== "1day" ? null : props.range === "5Y" ? "week" : props.range === "MAX" ? "month" : null;
    return period ? aggregateBars(d.bars, d.sma50, d.sma200, period) : { bars: d.bars, sma50: d.sma50, sma200: d.sma200, interval: d.interval };
  });
  let chartWrap: HTMLDivElement | undefined;
  const [plotWidth, setPlotWidth] = createSignal(0);
  onMount(() => {
    // Grid insets of the candle chart (left 60, right 62).
    const measure = () => setPlotWidth(Math.max(0, (chartWrap?.clientWidth ?? 0) - 122));
    const observer = new ResizeObserver(measure);
    if (chartWrap) observer.observe(chartWrap);
    measure();
    onCleanup(() => observer.disconnect());
  });

  const option = () => {
    const d = data();
    const s = series();
    if (!d || !s || !s.bars.length) return null;
    const c = t().asset;
    return candleOption(
      {
        bars: s.bars,
        interval: s.interval,
        sma50: s.sma50,
        sma200: s.sma200,
        trades: d.trades.map((tr) => ({ t: tr.t, side: tr.side, qty: toNumber(tr.qty) ?? 0, price: toNumber(tr.price) ?? 0 })),
        lastPrice: lastPrice(),
        plotWidth: plotWidth() || undefined,
      },
      readPalette(),
      {
        open: c.open,
        high: c.high,
        low: c.low,
        close: c.close,
        volume: c.volume,
        sma50: c.sma50,
        sma200: c.sma200,
        buy: c.buy,
        sell: c.sell,
        change: c.change,
        weekOf: c.week_of,
      },
    );
  };

  const onKey = (event: KeyboardEvent) => {
    if (event.key === "ArrowLeft") step(-1);
    if (event.key === "ArrowRight") step(1);
  };

  return (
    <div class="asset-view">
      <div class="asset-toolbar">
        <div class="row" style={{ gap: "6px" }}>
          <button class="btn icon sm" onClick={() => step(-1)} aria-label="previous">
            <Icon name="chevron_left" size={16} />
          </button>
          <select class="select asset-select" value={props.symbol} onChange={(e) => props.onSymbol(e.currentTarget.value)} onKeyDown={onKey} aria-label={t().asset.pick}>
            <For each={groups()}>
              {(g) => (
                <optgroup label={sectorLabel(props.board?.sectors, g.sector.id)}>
                  <For each={g.items}>
                    {(i) => (
                      // `selected` as well as the select's value: options arrive after the value
                      // is set (the board loads later), and a select left without its option
                      // would otherwise show the first company while the chart shows another.
                      <option value={i.symbol} selected={i.symbol === props.symbol}>{`${i.symbol} · ${pick(i, "name")}`}</option>
                    )}
                  </For>
                </optgroup>
              )}
            </For>
            <Show when={!order().includes(props.symbol)}>
              <option value={props.symbol} selected>
                {props.symbol}
              </option>
            </Show>
          </select>
          <button class="btn icon sm" onClick={() => step(1)} aria-label="next">
            <Icon name="chevron_right" size={16} />
          </button>
        </div>
        <div class="spacer" />
        <Segmented value={props.range} onChange={(v) => props.onRange(v)} options={ASSET_RANGES.map((r) => ({ value: r, label: r }))} label={t().asset.range} />
      </div>

      <Show when={data()}>
        {(d) => (
          <div class="asset-head">
            <div class="asset-id">
              <div class="row" style={{ gap: "8px", "align-items": "baseline" }}>
                <span class="ticker" style={{ "font-size": "18px" }}>
                  {d().symbol}
                </span>
                <span class="subtle">{asset() ? pick(asset()!, "name") : d().benchmark ? pick(d().benchmark!, "name") : ""}</span>
              </div>
              <div class="muted xs">
                <Show when={asset()}>
                  {sectorLabel(props.board?.sectors, asset()!.sector_id)} · {pick(asset()!, "subtype")}
                </Show>
              </div>
            </div>
            <div class="asset-price">
              <span class="num price">{fmtPrice(lastPrice())}</span>
              <Show when={quote()}>
                <span class="xs">
                  <Pct value={quote()!.change_pct} />{" "}
                  <span class={`num ${polarity(quote()!.change)}`}>{quote()!.change != null ? `(${quote()!.change! > 0 ? "+" : quote()!.change! < 0 ? "−" : ""}${fmtPrice(Math.abs(quote()!.change!))})` : ""}</span>
                </span>
              </Show>
            </div>
            <div class="asset-stat">
              <span class="label">{t().asset.position}</span>
              <Show when={props.position && props.position.qty > 0} fallback={<span class="muted">{t().asset.no_position}</span>}>
                <span class="num">
                  {fmtQty(props.position!.qty)} @ {fmtPrice(props.position!.avg_cost)}
                </span>
                <span class={`num xs ${polarity(props.position!.unrealized_pnl)}`}>
                  {fmtMoney(props.position!.unrealized_pnl, { sign: true })} ({fmtPct(props.position!.unrealized_pct, { sign: true })})
                </span>
              </Show>
            </div>
            <div class="asset-stat">
              <span class="label">
                {t().asset.weight} / {t().asset.target}
              </span>
              <span class="num">
                {fmtPct(props.position?.weight ?? 0, { dp: 2 })} / {props.target == null ? "—" : fmtPct(props.target, { dp: 2 })}
              </span>
              <WeightBar weight={props.position?.weight ?? 0} target={props.target} max={0.06} />
            </div>
          </div>
        )}
      </Show>

      <div ref={chartWrap}>
        <Show when={!bars.error} fallback={<ErrorState error={bars.error} onRetry={refetch} />}>
          <Show when={data() && data()!.bars.length === 0}>
            <Empty title={t().asset.empty} icon="candle" />
          </Show>
          <Show when={!data() || data()!.bars.length > 0}>
            <Chart option={option} height={430} busy={bars.loading} ariaLabel={props.symbol} />
          </Show>
        </Show>
      </div>

      <Show when={asset()}>
        <div class="asset-foot">
          <Show when={locale() === "zh" && asset()!.role_zh}>
            <p class="small subtle asset-role">
              <span class="muted">{t().asset.role}：</span>
              {asset()!.role_zh}
            </p>
          </Show>
          <div class="row" style={{ gap: "8px" }}>
            <A class="btn sm ghost" href={`/trades?tab=fills&symbol=${encodeURIComponent(props.symbol)}`}>
              <Icon name="trades" size={14} /> {t().asset.view_trades}
            </A>
            <A class="btn sm ghost" href={`/universe?symbol=${encodeURIComponent(props.symbol)}`}>
              <Icon name="universe" size={14} /> {sectorLabel(props.board?.sectors, asset()!.sector_id)}
            </A>
          </div>
        </div>
      </Show>
    </div>
  );
}
