import { For, Show, createMemo, createSignal, onCleanup, onMount } from "solid-js";
import { Chart } from "@/components/Chart";
import { Empty } from "@/components/ui";
import { locale, pick } from "@/i18n";
import { overviewText } from "@/i18n/overview";
import { BOARD_GRID, boardOption, layoutBoard } from "@/lib/charts/builders";
import { readPalette } from "@/lib/charts/echarts";
import { fmtPct, polarity } from "@/lib/format";
import type { Board } from "@/lib/types";
import { sectorLabel } from "./util";

const CHART_HEIGHT = 430;
/** Narrowest readable column; below this the board scrolls horizontally. */
const MIN_COLUMN = 13;

let measureCanvas: HTMLCanvasElement | null = null;
function textWidth(text: string, font: string): number {
  measureCanvas ??= document.createElement("canvas");
  const ctx = measureCanvas.getContext("2d");
  if (!ctx) return text.length * 7;
  ctx.font = font;
  return ctx.measureText(text).width;
}

/**
 * Every company's move over the period as one candle (normalised to the close before the
 * period), grouped by sector, with current vs target weights underneath.
 */
export function MarketBoard(props: {
  board: Board;
  hidden: Set<string>;
  onToggleSector: (id: string) => void;
  onShowAll: () => void;
  onPick: (symbol: string) => void;
  busy: boolean;
}) {
  const t = overviewText;
  let wrap!: HTMLDivElement;
  const [width, setWidth] = createSignal(0);

  onMount(() => {
    const observer = new ResizeObserver(() => setWidth(wrap.clientWidth));
    observer.observe(wrap);
    setWidth(wrap.clientWidth);
    onCleanup(() => observer.disconnect());
  });

  const visible = createMemo(() => {
    const hidden = props.hidden;
    return hidden.size ? new Set(props.board.sectors.map((s) => s.id).filter((id) => !hidden.has(id))) : null;
  });
  const layout = createMemo(() => layoutBoard(props.board, visible()));
  const plotWidth = createMemo(() => {
    const needed = layout().slots.length * MIN_COLUMN + BOARD_GRID.left + BOARD_GRID.right;
    return Math.max(width(), needed);
  });
  const column = createMemo(() => (plotWidth() - BOARD_GRID.left - BOARD_GRID.right) / Math.max(1, layout().slots.length));

  const sectorStats = createMemo(() =>
    props.board.sectors
      .map((sector) => {
        const members = props.board.items.filter((i) => i.sector_id === sector.id);
        const valid = members.map((m) => m.close_pct).filter((v): v is number => v != null);
        return {
          sector,
          count: members.length,
          avg: valid.length ? valid.reduce((a, b) => a + b, 0) / valid.length : null,
        };
      })
      .filter((s) => s.count > 0),
  );

  const headerLabel = (id: string, available: number) => {
    const full = sectorLabel(props.board.sectors, id);
    const font = "600 11px system-ui, sans-serif";
    if (textWidth(full, font) <= available - 6) return full;
    return sectorLabel(props.board.sectors, id, true);
  };

  const option = () => {
    const l = layout();
    if (!l.slots.length) return null;
    const c = t();
    return boardOption(l, readPalette(), {
      weight: c.board.weight,
      target: c.board.target,
      change: c.board.change,
      open: c.board.open,
      high: c.board.high,
      low: c.board.low,
      close: c.board.close,
      weightAxis: c.board.weight_axis,
      name: (i) => pick(i, "name"),
      sector: (id) => sectorLabel(props.board.sectors, id),
    });
  };

  const onClick = (params: any) => {
    const slot = layout().slots[params?.dataIndex];
    if (slot) props.onPick(slot.symbol);
  };

  return (
    <div class="board">
      <div class="board-scroll" ref={wrap}>
        <Show when={layout().slots.length} fallback={<Empty title={t().board.empty} icon="candle" />}>
          <div style={{ width: `${plotWidth()}px` }}>
            <div class="board-sectors" aria-hidden="true">
              <For each={layout().spans}>
                {(span) => {
                  const left = () => BOARD_GRID.left + span.start * column();
                  const w = () => span.count * column();
                  // Labels may spill into the empty spacer column on each side.
                  const room = () => w() + column();
                  return (
                    <div class="board-sector" style={{ left: `${left()}px`, width: `${w()}px` }} title={sectorLabel(props.board.sectors, span.sector.id)}>
                      <span class="name" style={{ "max-width": `${room()}px` }}>
                        {headerLabel(span.sector.id, room())}
                      </span>
                      <span class={`avg num ${polarity(span.avg)}`}>{fmtPct(span.avg, { dp: 1, sign: true })}</span>
                    </div>
                  );
                }}
              </For>
            </div>
            <Chart option={option} height={CHART_HEIGHT} onClick={onClick} busy={props.busy} ariaLabel={t().board.title} />
          </div>
        </Show>
      </div>
      <div class="board-foot">
        <div class="board-legend" aria-label="legend">
          <span>
            <i class="lg-candle up" />
            {t().board.legend_up}
          </span>
          <span>
            <i class="lg-candle down" />
            {t().board.legend_down}
          </span>
          <span>
            <i class="lg-bar" />
            {t().board.legend_weight}
          </span>
          <span>
            <i class="lg-target" />
            {t().board.legend_target}
          </span>
        </div>
        <span class="muted xs hide-sm">{t().board.click_hint}</span>
      </div>
      <div class="sector-chips" role="group" aria-label={t().board.sectors}>
        <button class="sector-chip" aria-pressed={props.hidden.size === 0} onClick={() => props.onShowAll()}>
          {t().board.all_sectors}
        </button>
        <For each={sectorStats()}>
          {(s) => (
            <button
              class="sector-chip"
              aria-pressed={!props.hidden.has(s.sector.id)}
              onClick={() => props.onToggleSector(s.sector.id)}
              title={locale() === "zh" ? s.sector.summary_zh : s.sector.summary_en}
            >
              <span>{sectorLabel(props.board.sectors, s.sector.id)}</span>
              <span class="muted">{s.count}</span>
              <b class={`num ${polarity(s.avg)}`}>{fmtPct(s.avg, { dp: 1, sign: true })}</b>
            </button>
          )}
        </For>
      </div>
    </div>
  );
}
