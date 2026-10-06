/**
 * Pieces shared by the three tabs of /trades: filter state, the filter toolbar, the summary
 * strip and CSV export.
 */
import { type JSX, Show } from "solid-js";
import { Icon } from "@/components/Icon";
import { downloadCsv, toast } from "@/components/ui";
import { tpl } from "@/i18n";
import { common } from "@/i18n/common";
import { fmtNum } from "@/lib/format";
import { DateRange, SymbolPicker, marketDate } from "./components";
import type { NameIndex } from "./names";
import { activityText } from "./text";

export interface TradeFilters {
  symbol: string;
  from: string;
  to: string;
  status: string;
  kind: string;
}

export interface TabProps {
  filters: TradeFilters;
  setFilters: (patch: Partial<TradeFilters>) => void;
  names: () => NameIndex | undefined;
  /** Bumped by live events; tabs refetch when it changes. */
  tick: () => number;
}

export function Toolbar(props: {
  filters: TradeFilters;
  setFilters: (patch: Partial<TradeFilters>) => void;
  names: NameIndex | undefined;
  dateHint: string;
  extra?: JSX.Element;
  extraActive?: boolean;
}) {
  const a = activityText;
  const active = () => !!(props.filters.symbol || props.filters.from || props.filters.to || props.extraActive);
  return (
    <div class="act-toolbar" role="search">
      <div class="act-field">
        <span class="act-field-label">{a().symbol.label}</span>
        <SymbolPicker value={props.filters.symbol} names={props.names} onChange={(symbol) => props.setFilters({ symbol })} />
      </div>
      <div class="act-field">
        <span class="act-field-label" title={props.dateHint}>
          {a().range.label}
          <Icon name="info" size={12} />
        </span>
        <DateRange from={props.filters.from} to={props.filters.to} onChange={(from, to) => props.setFilters({ from, to })} />
      </div>
      {props.extra}
      <Show when={active()}>
        <button
          type="button"
          class="btn ghost sm act-clear"
          onClick={() => props.setFilters({ symbol: "", from: "", to: "", status: "", kind: "" })}
        >
          <Icon name="x" size={13} />
          {a().clear_filters}
        </button>
      </Show>
    </div>
  );
}

export function Summary(props: { children: JSX.Element; note?: string | null; loading?: boolean }) {
  return (
    <div class="act-summary">
      <div class="kpis act-kpis" classList={{ refetching: !!props.loading }}>
        {props.children}
      </div>
      <Show when={props.note}>
        <p class="act-summary-note">
          <Icon name="info" size={13} />
          {props.note}
        </p>
      </Show>
    </div>
  );
}

/** "hone-quant-fills-2026-10-05-NVDA.csv" */
export function csvName(kind: string, filters: TradeFilters): string {
  const parts = ["hone-quant", kind, marketDate()];
  if (filters.symbol) parts.push(filters.symbol);
  if (filters.from || filters.to) parts.push(`${filters.from || "start"}_${filters.to || "now"}`);
  return `${parts.join("-")}.csv`;
}

export function exportRows(
  filename: string,
  header: string[],
  rows: (string | number | null | undefined)[][],
  truncatedAt: number | null,
) {
  const a = activityText();
  if (!rows.length) {
    toast(a.csv.nothing, undefined, "warning");
    return;
  }
  downloadCsv(filename, header, rows);
  if (truncatedAt !== null) toast(tpl(a.csv.truncated, { n: fmtNum(truncatedAt, 0) }), undefined, "warning", 6000);
  else toast(tpl(a.csv.exported, { n: fmtNum(rows.length, 0) }), undefined, "success", 2600);
}

export function ExportButton(props: { onClick: () => void; busy?: boolean; disabled?: boolean }) {
  return (
    <button type="button" class="btn sm" onClick={() => props.onClick()} disabled={props.busy || props.disabled}>
      <Icon name="download" size={14} />
      {props.busy ? activityText().csv.exporting : common().actions.export_csv}
    </button>
  );
}

export function truncationNote(all: { truncated: boolean; rows: unknown[]; total: number } | undefined): string | null {
  if (!all?.truncated) return null;
  return tpl(activityText().summary.truncated, { n: fmtNum(all.rows.length, 0), total: fmtNum(all.total, 0) });
}
