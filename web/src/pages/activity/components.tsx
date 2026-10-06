/**
 * Building blocks shared by the activity pages: tab bar with counts, symbol picker, date range,
 * pager, dual-zone time cell, JSON viewer and the live-connection badge.
 */
import { A } from "@solidjs/router";
import { For, Index, type JSX, Show, createEffect, createMemo, createSignal, createUniqueId, on, onCleanup, untrack } from "solid-js";
import { Icon } from "@/components/Icon";
import { Segmented, toast } from "@/components/ui";
import { locale, tpl } from "@/i18n";
import { connected } from "@/lib/events";
import { DASH, MARKET_TZ, fmtDateTime, fmtDual, fmtNum, zoneLabel } from "@/lib/format";
import { displayTz } from "@/lib/prefs";
import { serverNow } from "@/lib/session";
import { XIcon } from "./icons";
import { type NameIndex, groupName, nameOf } from "./names";
import { activityText } from "./text";
import { addDays, dateIn } from "./util";

const t = activityText;

// ---------------------------------------------------------------------------------------------
// Tabs
// ---------------------------------------------------------------------------------------------

export interface TabDef<T extends string> {
  id: T;
  label: string;
  count?: number | null;
  attention?: boolean;
}

export function TabBar<T extends string>(props: { tabs: TabDef<T>[]; value: T; onChange: (id: T) => void; label?: string }) {
  const onKey = (event: KeyboardEvent) => {
    if (event.key !== "ArrowRight" && event.key !== "ArrowLeft") return;
    const container = event.currentTarget as HTMLElement;
    const index = props.tabs.findIndex((tab) => tab.id === props.value);
    const next = props.tabs[(index + (event.key === "ArrowRight" ? 1 : props.tabs.length - 1)) % props.tabs.length];
    props.onChange(next.id);
    requestAnimationFrame(() => container.querySelector<HTMLElement>('[aria-selected="true"]')?.focus());
  };
  // Index (not For): the tab list is rebuilt when counts change, the buttons should not be.
  return (
    <div class="tabs act-tabs" role="tablist" aria-label={props.label} onKeyDown={onKey}>
      <Index each={props.tabs}>
        {(tab) => (
          <button
            type="button"
            role="tab"
            aria-selected={props.value === tab().id}
            tabIndex={props.value === tab().id ? 0 : -1}
            onClick={() => props.onChange(tab().id)}
          >
            {tab().label}
            <Show when={tab().count !== undefined && tab().count !== null}>
              <span class="act-count" classList={{ attention: !!tab().attention }}>
                {fmtNum(tab().count, 0)}
              </span>
            </Show>
          </button>
        )}
      </Index>
    </div>
  );
}

// ---------------------------------------------------------------------------------------------
// Symbol picker
// ---------------------------------------------------------------------------------------------

type PickerRow =
  | { kind: "group"; label: string }
  | { kind: "option"; symbol: string; name: string; index: number };

export function SymbolPicker(props: { value: string; onChange: (symbol: string) => void; names: NameIndex | undefined }) {
  const id = createUniqueId();
  const [open, setOpen] = createSignal(false);
  const [query, setQuery] = createSignal("");
  const [active, setActive] = createSignal(0);
  let root: HTMLDivElement | undefined;
  let trigger: HTMLButtonElement | undefined;
  let input: HTMLInputElement | undefined;
  let list: HTMLDivElement | undefined;

  const rows = createMemo<PickerRow[]>(() => {
    const index = props.names;
    const q = query().trim().toLowerCase();
    const zh = locale() === "zh";
    const out: PickerRow[] = [];
    let i = 0;
    if (!q) out.push({ kind: "option", symbol: "", name: t().symbol.all, index: i++ });
    if (!index) return out;
    if (q) {
      const scored: { symbol: string; name: string; score: number }[] = [];
      for (const group of index.groups) {
        for (const entry of group.items) {
          const sym = entry.symbol.toLowerCase();
          const names = `${entry.name_zh} ${entry.name_en}`.toLowerCase();
          const score = sym === q ? 0 : sym.startsWith(q) ? 1 : sym.includes(q) ? 2 : names.includes(q) ? 3 : -1;
          if (score >= 0) scored.push({ symbol: entry.symbol, name: zh ? entry.name_zh : entry.name_en, score });
        }
      }
      scored.sort((a, b) => a.score - b.score || a.symbol.localeCompare(b.symbol));
      for (const s of scored) out.push({ kind: "option", symbol: s.symbol, name: s.name, index: i++ });
      return out;
    }
    for (const group of index.groups) {
      out.push({ kind: "group", label: group.id === "__removed" ? t().symbol.removed : groupName(group) });
      for (const entry of group.items) out.push({ kind: "option", symbol: entry.symbol, name: zh ? entry.name_zh : entry.name_en, index: i++ });
    }
    return out;
  });
  const options = createMemo(() => rows().filter((r): r is Extract<PickerRow, { kind: "option" }> => r.kind === "option"));

  const close = (focusTrigger = false) => {
    setOpen(false);
    setQuery("");
    if (focusTrigger) trigger?.focus();
  };
  const choose = (symbol: string) => {
    close(true);
    if (symbol !== props.value) props.onChange(symbol);
  };
  const openPicker = () => {
    setQuery("");
    setOpen(true);
    const current = options().findIndex((o) => o.symbol === props.value);
    setActive(Math.max(0, current));
    requestAnimationFrame(() => {
      input?.focus();
      scrollActive();
    });
  };
  const scrollActive = () => list?.querySelector<HTMLElement>(`[data-index="${active()}"]`)?.scrollIntoView({ block: "nearest" });

  createEffect(on(query, () => setActive(0), { defer: true }));

  const onKey = (event: KeyboardEvent) => {
    const count = options().length;
    if (event.key === "ArrowDown") {
      event.preventDefault();
      setActive((a) => Math.min(count - 1, a + 1));
      scrollActive();
    } else if (event.key === "ArrowUp") {
      event.preventDefault();
      setActive((a) => Math.max(0, a - 1));
      scrollActive();
    } else if (event.key === "Enter") {
      event.preventDefault();
      const option = options()[active()];
      if (option) choose(option.symbol);
    } else if (event.key === "Escape") {
      event.preventDefault();
      close(true);
    } else if (event.key === "Tab") {
      close();
    }
  };

  const onDocDown = (event: MouseEvent) => {
    if (open() && root && !root.contains(event.target as Node)) close();
  };
  document.addEventListener("mousedown", onDocDown);
  onCleanup(() => document.removeEventListener("mousedown", onDocDown));

  return (
    <div class="act-picker" ref={root}>
      <button
        ref={trigger}
        type="button"
        class="select act-picker-trigger"
        classList={{ set: !!props.value }}
        aria-haspopup="listbox"
        aria-expanded={open()}
        aria-label={`${t().symbol.label}: ${props.value || t().symbol.all}`}
        onClick={() => (open() ? close() : openPicker())}
      >
        <Show when={props.value} fallback={<span class="muted">{t().symbol.all}</span>}>
          <span class="ticker">{props.value}</span>
          <span class="act-picker-name">{nameOf(props.names, props.value)}</span>
        </Show>
      </button>
      <Show when={props.value}>
        <button type="button" class="act-picker-clear" aria-label={t().symbol.clear} title={t().symbol.clear} onClick={() => props.onChange("")}>
          <Icon name="x" size={13} />
        </button>
      </Show>
      <Show when={open()}>
        <div class="act-pop" role="dialog" aria-label={t().symbol.label}>
          <div class="act-pop-search">
            <Icon name="search" size={15} />
            <input
              ref={input}
              class="act-pop-input"
              role="combobox"
              aria-expanded="true"
              aria-controls={`${id}-list`}
              aria-activedescendant={options()[active()] ? `${id}-opt-${active()}` : undefined}
              placeholder={t().symbol.search}
              autocomplete="off"
              spellcheck={false}
              value={query()}
              onInput={(e) => setQuery(e.currentTarget.value)}
              onKeyDown={onKey}
            />
          </div>
          <div class="act-pop-list" role="listbox" id={`${id}-list`} ref={list}>
            <For each={rows()}>
              {(row) =>
                row.kind === "group" ? (
                  <div class="act-pop-group" role="presentation">
                    {row.label}
                  </div>
                ) : (
                  <div
                    id={`${id}-opt-${row.index}`}
                    data-index={row.index}
                    role="option"
                    aria-selected={row.symbol === props.value}
                    class="act-option"
                    classList={{ active: active() === row.index, chosen: row.symbol === props.value }}
                    onMouseDown={(e) => e.preventDefault()}
                    onMouseMove={() => active() !== row.index && setActive(row.index)}
                    onClick={() => choose(row.symbol)}
                  >
                    <Show when={row.symbol} fallback={<span class="act-option-all">{row.name}</span>}>
                      <span class="ticker">{row.symbol}</span>
                      <span class="act-option-name">{row.name}</span>
                    </Show>
                    <Show when={row.symbol === props.value}>
                      <Icon name="check" size={14} class="act-option-check" />
                    </Show>
                  </div>
                )
              }
            </For>
            <Show when={rows().length === 0}>
              <div class="act-pop-empty">{t().symbol.no_match}</div>
            </Show>
          </div>
        </div>
      </Show>
    </div>
  );
}

// ---------------------------------------------------------------------------------------------
// Date range
// ---------------------------------------------------------------------------------------------

/** The trading date (New York) on the server clock. */
export const marketDate = () => dateIn(serverNow(), MARKET_TZ);

export function DateRange(props: { from: string; to: string; onChange: (from: string, to: string) => void }) {
  // The clock ticks every second; only a change of date may rebuild the presets.
  const today = createMemo(() => marketDate());
  const presets = createMemo(() => {
    const d = today();
    return [
      { value: "all", label: t().range.all, from: "", to: "" },
      { value: "today", label: t().range.today, from: d, to: d },
      { value: "d7", label: t().range.d7, from: addDays(d, -6), to: d },
      { value: "d30", label: t().range.d30, from: addDays(d, -29), to: d },
      { value: "ytd", label: t().range.ytd, from: `${d.slice(0, 4)}-01-01`, to: d },
    ];
  });
  const current = () => presets().find((p) => p.from === props.from && p.to === props.to)?.value ?? "";
  const invalid = () => !!props.from && !!props.to && props.from > props.to;
  return (
    <div class="act-range" role="group" aria-label={t().range.label}>
      <Segmented
        value={current()}
        label={t().range.label}
        options={presets().map((p) => ({ value: p.value, label: p.label }))}
        onChange={(value) => {
          const preset = presets().find((p) => p.value === value);
          if (preset) props.onChange(preset.from, preset.to);
        }}
      />
      <div class="act-dates" classList={{ invalid: invalid() }} title={invalid() ? t().range.invalid : undefined}>
        <input
          type="date"
          class="input"
          aria-label={t().range.from}
          value={props.from}
          max={today()}
          onChange={(e) => props.onChange(e.currentTarget.value, props.to)}
        />
        <span class="act-dates-sep" aria-hidden="true">
          –
        </span>
        <input
          type="date"
          class="input"
          aria-label={t().range.to}
          value={props.to}
          max={today()}
          onChange={(e) => props.onChange(props.from, e.currentTarget.value)}
        />
      </div>
    </div>
  );
}

// ---------------------------------------------------------------------------------------------
// Pager
// ---------------------------------------------------------------------------------------------

export function Pager(props: {
  offset: number;
  limit: number;
  total: number;
  onOffset: (offset: number) => void;
  onLimit?: (limit: number) => void;
  sizes?: number[];
  extra?: JSX.Element;
}) {
  const pages = () => Math.max(1, Math.ceil(props.total / props.limit));
  const page = () => Math.min(pages(), Math.floor(props.offset / props.limit) + 1);
  return (
    <div class="card-foot act-pager">
      <span class="num">
        {props.total > 0
          ? tpl(t().pager.range, {
              from: fmtNum(props.offset + 1, 0),
              to: fmtNum(Math.min(props.offset + props.limit, props.total), 0),
              total: fmtNum(props.total, 0),
            })
          : t().pager.none}
      </span>
      {props.extra}
      <span class="spacer" />
      <Show when={props.onLimit}>
        <select
          class="select act-size"
          aria-label={t().pager.size}
          value={String(props.limit)}
          onChange={(e) => props.onLimit?.(Number(e.currentTarget.value))}
        >
          <For each={props.sizes ?? [25, 50, 100, 200]}>{(n) => <option value={String(n)}>{tpl(t().pager.size_option, { n })}</option>}</For>
        </select>
      </Show>
      <div class="act-pages">
        <button
          type="button"
          class="btn sm icon"
          disabled={page() <= 1}
          aria-label={t().pager.prev}
          title={t().pager.prev}
          onClick={() => props.onOffset(Math.max(0, props.offset - props.limit))}
        >
          <Icon name="chevron_left" size={14} />
        </button>
        <span class="num act-pageno">{tpl(t().pager.page, { page: page(), pages: pages() })}</span>
        <button
          type="button"
          class="btn sm icon"
          disabled={page() >= pages()}
          aria-label={t().pager.next}
          title={t().pager.next}
          onClick={() => props.onOffset(props.offset + props.limit)}
        >
          <Icon name="chevron_right" size={14} />
        </button>
      </div>
    </div>
  );
}

// ---------------------------------------------------------------------------------------------
// Cells
// ---------------------------------------------------------------------------------------------

/** Local and New York time on two lines (fmtDual), with the full timestamp on hover. */
export function DualTime(props: { value: string | null | undefined; date?: boolean }) {
  const parts = () => {
    const value = props.value;
    if (!value) return [DASH];
    const split = fmtDual(value, props.date ?? true).split(" · ");
    const year = dateIn(new Date(value).getTime(), displayTz()).slice(0, 4);
    const thisYear = untrack(() => dateIn(serverNow(), displayTz()).slice(0, 4));
    if ((props.date ?? true) && year !== thisYear) split[0] = `${year}-${split[0]}`;
    return split;
  };
  const full = () =>
    props.value ? `${fmtDateTime(props.value)} ${zoneLabel(displayTz())} · ${fmtDateTime(props.value, MARKET_TZ)} ET` : undefined;
  return (
    <span class="act-time" title={full()}>
      <span class="primary num">{parts()[0]}</span>
      <Show when={parts()[1]}>
        <span class="secondary num">{parts()[1]}</span>
      </Show>
    </span>
  );
}

export function SymbolCell(props: { symbol: string | null | undefined; names: NameIndex | undefined; onFilter?: (symbol: string) => void }) {
  return (
    <Show when={props.symbol} fallback={<span class="muted">{DASH}</span>}>
      <div class="name-cell act-symbol">
        <Show when={props.onFilter} fallback={<span class="ticker">{props.symbol}</span>}>
          <button
            type="button"
            class="ticker act-ticker"
            title={tpl(t().symbol.filter_by, { symbol: props.symbol })}
            onClick={() => props.onFilter?.(props.symbol!)}
          >
            {props.symbol}
          </button>
        </Show>
        <span class="name">{nameOf(props.names, props.symbol)}</span>
      </div>
    </Show>
  );
}

export function PlanLink(props: { id: number | null | undefined }) {
  return (
    <Show when={props.id} fallback={<span class="muted">{DASH}</span>}>
      <A class="act-plan-link num" href={`/plans/${props.id}`}>
        #{props.id}
        <XIcon name="arrow_right" size={12} />
      </A>
    </Show>
  );
}

export function LiveBadge() {
  return (
    <span class="act-live" classList={{ on: connected() }} role="status">
      <span class="dot" />
      {connected() ? t().live : t().offline}
    </span>
  );
}

// ---------------------------------------------------------------------------------------------
// JSON
// ---------------------------------------------------------------------------------------------

const TOKEN = /("(?:\\u[a-fA-F0-9]{4}|\\[^u]|[^\\"])*"(\s*:)?|\b(?:true|false|null)\b|-?\d+(?:\.\d*)?(?:[eE][+-]?\d+)?)/g;

function tokens(text: string): { text: string; cls: string }[] {
  const out: { text: string; cls: string }[] = [];
  let last = 0;
  for (const match of text.matchAll(TOKEN)) {
    const index = match.index ?? 0;
    if (index > last) out.push({ text: text.slice(last, index), cls: "" });
    const value = match[0];
    let cls = "num";
    if (value.startsWith('"')) cls = match[2] ? "key" : "str";
    else if (value === "true" || value === "false") cls = "bool";
    else if (value === "null") cls = "null";
    out.push({ text: value, cls });
    last = index + value.length;
  }
  if (last < text.length) out.push({ text: text.slice(last), cls: "" });
  return out;
}

export async function copyText(text: string): Promise<boolean> {
  try {
    await navigator.clipboard.writeText(text);
    return true;
  } catch {
    try {
      const area = document.createElement("textarea");
      area.value = text;
      area.style.position = "fixed";
      area.style.opacity = "0";
      document.body.appendChild(area);
      area.select();
      const ok = document.execCommand("copy");
      area.remove();
      return ok;
    } catch {
      return false;
    }
  }
}

export function JsonView(props: { value: unknown; label?: string }) {
  const text = () => JSON.stringify(props.value ?? null, null, 2) ?? "null";
  const isEmpty = () => {
    const v = props.value;
    return v === null || v === undefined || (typeof v === "object" && Object.keys(v as object).length === 0);
  };
  const copy = async () => {
    if (await copyText(text())) toast(t().json.copied, undefined, "success", 1800);
  };
  return (
    <Show when={!isEmpty()} fallback={<p class="muted xs">{t().json.empty}</p>}>
      <div class="act-json">
        <div class="act-json-bar">
          <span class="kicker">{props.label ?? "JSON"}</span>
          <button type="button" class="btn ghost sm" onClick={copy}>
            <XIcon name="copy" size={13} />
            {t().json.copy}
          </button>
        </div>
        <pre>
          <code>
            <For each={tokens(text())}>{(token) => (token.cls ? <span class={`j-${token.cls}`}>{token.text}</span> : token.text)}</For>
          </code>
        </pre>
      </div>
    </Show>
  );
}
