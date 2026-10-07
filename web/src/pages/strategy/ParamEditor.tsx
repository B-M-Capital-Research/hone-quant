import { For, type JSX, Show, createEffect, createSignal, on } from "solid-js";
import { Segmented, Switch } from "@/components/ui";
import { tpl } from "@/i18n";
import { strategyText } from "@/i18n/strategy";
import type { Sector, StrategyParams } from "@/lib/types";
import { type FieldDef, type FieldKind, GROUPS, fieldForIssue, fieldsOf, fromInputText, getPath, toInputText } from "./params";
import { fieldText, fmtBound, fmtParam, issueText, pickText } from "./format";
import type { Workbench } from "./workbench-state";

export function fieldDomId(path: string): string {
  return `st-f-${path.replace(/\./g, "-")}`;
}

/** Text input bound to a number, keeping the user's text while it is being typed. */
function NumInput(props: {
  id: string;
  kind: FieldKind;
  value: number | null | undefined;
  onValue: (value: number, text: string) => void;
  invalid?: boolean;
  disabled?: boolean;
  ariaLabel?: string;
}) {
  const [text, setText] = createSignal(toInputText(props.kind, props.value));
  createEffect(
    on(
      () => props.value,
      (value) => {
        const parsed = fromInputText(props.kind, text());
        const same =
          (typeof value === "number" && Number.isFinite(value) && Math.abs(parsed - value) < 1e-12) ||
          ((value === null || value === undefined || Number.isNaN(value)) && Number.isNaN(parsed));
        if (!same) setText(toInputText(props.kind, value));
      },
      { defer: true },
    ),
  );
  return (
    <input
      id={props.id}
      class="input num"
      classList={{ invalid: !!props.invalid }}
      type="text"
      inputmode={props.kind === "int" ? "numeric" : "decimal"}
      autocomplete="off"
      spellcheck={false}
      aria-label={props.ariaLabel}
      aria-invalid={props.invalid ? "true" : undefined}
      disabled={props.disabled}
      value={text()}
      onInput={(e) => {
        setText(e.currentTarget.value);
        props.onValue(fromInputText(props.kind, e.currentTarget.value), e.currentTarget.value);
      }}
    />
  );
}

function unitFor(kind: FieldKind): { pre?: string; post?: string } {
  const u = strategyText().units;
  switch (kind) {
    case "int":
      return { post: u.days };
    case "pct":
    case "optpct":
      return { post: "%" };
    case "pp":
      return { post: u.pp_short };
    case "money":
      return { pre: "$" };
    case "mult":
      return { pre: "×" };
    default:
      return {};
  }
}

function Addon(props: { kind: FieldKind; children: JSX.Element }) {
  const unit = () => unitFor(props.kind);
  return (
    <div class="st-input-group">
      <Show when={unit().pre}>
        <span class="st-addon pre">{unit().pre}</span>
      </Show>
      {props.children}
      <Show when={unit().post}>
        <span class="st-addon post">{unit().post}</span>
      </Show>
    </div>
  );
}

function FieldRow(props: { def: FieldDef; wb: Workbench; defaults: StrategyParams; sectors: Sector[]; memberCounts: Record<string, number> }) {
  const t = strategyText;
  const path = props.def.path;
  const id = fieldDomId(path);
  const draft = () => props.wb.draft() as StrategyParams;
  const value = () => getPath(draft(), path);
  const startValue = () => {
    const s = props.wb.source();
    return s ? getPath(s.params, path) : undefined;
  };
  const isChanged = () => props.wb.changed().includes(path);
  const issues = () => props.wb.issues().filter((i) => fieldForIssue(i.path)?.path === path);
  const invalid = () => issues().length > 0;
  const meta = () => {
    const def = fmtParam(path, getPath(props.defaults, path));
    const { min, max } = props.def;
    if (min === undefined || max === undefined || props.def.kind === "budgets") return tpl(t().workbench.default_only, { value: def });
    return tpl(t().workbench.default_range, {
      value: def,
      min: `${props.def.zeroOff ? "0, " : ""}${fmtBound(props.def, min)}`,
      max: fmtBound(props.def, max),
    });
  };
  const [lastVol, setLastVol] = createSignal<number>((getPath(props.defaults, path) as number | null) ?? 0.2);

  const control = (): JSX.Element => {
    const kind = props.def.kind;
    if (kind === "bool") {
      return <Switch checked={!!value()} onChange={(v) => props.wb.update(path, v)} label={value() ? t().values.on : t().values.off} />;
    }
    if (kind === "method") {
      return (
        <Segmented
          value={String(value())}
          onChange={(v) => props.wb.update(path, v)}
          label={fieldText(path).label}
          options={(["risk", "member_count", "custom"] as const).map((m) => ({ value: m, label: t().methods[m] }))}
        />
      );
    }
    if (kind === "optpct") {
      const enabled = () => value() !== null && value() !== undefined;
      return (
        <div class="st-optpct">
          <Switch
            checked={enabled()}
            onChange={(on) => {
              if (on) props.wb.update(path, lastVol() || 0.2);
              else {
                const current = value();
                if (typeof current === "number" && Number.isFinite(current)) setLastVol(current);
                props.wb.update(path, null);
              }
            }}
            label={enabled() ? undefined : t().values.not_set}
          />
          <Show when={enabled()}>
            <Addon kind={kind}>
              <NumInput id={id} kind={kind} value={value() as number} invalid={invalid()} onValue={(n) => props.wb.update(path, n)} />
            </Addon>
          </Show>
        </div>
      );
    }
    if (kind === "budgets") return <span />;
    return (
      <Addon kind={kind}>
        <NumInput id={id} kind={kind} value={value() as number} invalid={invalid()} onValue={(n) => props.wb.update(path, n)} />
      </Addon>
    );
  };

  return (
    <div class="st-field" classList={{ "st-field-changed": isChanged(), "st-field-invalid": invalid(), wide: props.def.kind === "budgets" || props.def.kind === "method" }} data-path={path}>
      <div class="st-field-text">
        <label class="st-field-label" for={props.def.kind === "bool" || props.def.kind === "method" ? undefined : id}>
          {fieldText(path).label}
        </label>
        <div class="st-phint">{fieldText(path).hint}</div>
        <div class="st-field-meta">
          <span>{meta()}</span>
          <Show when={isChanged()}>
            <span class="st-was">{tpl(t().workbench.was, { value: fmtParam(path, startValue()) })}</span>
          </Show>
        </div>
      </div>
      <Show when={props.def.kind !== "budgets"}>
        <div class="st-field-control">{control()}</div>
      </Show>
      <Show when={props.def.kind === "budgets"}>
        <BudgetEditor wb={props.wb} sectors={props.sectors} memberCounts={props.memberCounts} />
      </Show>
      <Show when={invalid()}>
        <div class="st-field-error" role="alert">
          {[...new Set(issues().map(issueText))].join(" · ")}
        </div>
      </Show>
    </div>
  );
}

function BudgetEditor(props: { wb: Workbench; sectors: Sector[]; memberCounts: Record<string, number> }) {
  const t = strategyText;
  const draft = () => props.wb.draft() as StrategyParams;
  const custom = () => draft().sector.method === "custom";
  const budgets = () => draft().sector.custom_budgets ?? {};
  const total = () => Object.values(budgets()).reduce((a, b) => a + (Number.isFinite(b) ? b : 0), 0);
  const setOne = (sector: string, value: number, blank: boolean) => {
    const next = { ...budgets() };
    if (blank) delete next[sector];
    else next[sector] = value;
    props.wb.update("sector.custom_budgets", next);
  };
  const fill = () => {
    const next: Record<string, number> = {};
    for (const sector of props.sectors) next[sector.id] = props.memberCounts[sector.id] ?? 0;
    props.wb.update("sector.custom_budgets", next);
  };
  const invalidFor = (sector: string) => props.wb.issues().some((i) => i.path === `sector.custom_budgets.${sector}`);
  return (
    <div class="st-budgets" classList={{ disabled: !custom() }}>
      <div class="row wrap st-budgets-head">
        <span class="xs muted">{t().workbench.budgets_title}</span>
        <span class="spacer" />
        <span class="xs muted num">{tpl(t().workbench.budgets_total, { n: Math.round(total() * 100) / 100 })}</span>
        <button type="button" class="btn sm ghost" onClick={fill} disabled={!custom()}>
          {t().workbench.budgets_fill}
        </button>
      </div>
      <div class="st-budget-grid">
        <For each={props.sectors}>
          {(sector) => (
            <label class="st-budget">
              <span class="truncate" title={pickText(sector, "name")}>
                {pickText(sector, "name")}
              </span>
              <NumInput
                id={`st-budget-${sector.id}`}
                kind="num"
                value={budgets()[sector.id] ?? null}
                invalid={invalidFor(sector.id)}
                disabled={!custom()}
                ariaLabel={pickText(sector, "name")}
                onValue={(n, text) => setOne(sector.id, n, !text.trim())}
              />
            </label>
          )}
        </For>
      </div>
      <p class="xs muted">{t().workbench.budgets_hint}</p>
    </div>
  );
}

/** The parameter form, grouped like the read-only view. */
export function ParamEditor(props: { wb: Workbench; defaults: StrategyParams; sectors: Sector[]; memberCounts: Record<string, number> }) {
  const t = strategyText;
  return (
    <div class="st-editor">
      <For each={GROUPS}>
        {(group) => (
          <section class="st-edit-group">
            <header class="st-group-head">
              <h3>{t().groups[group]}</h3>
              <span class="muted xs">{t().group_hints[group]}</span>
            </header>
            <For each={fieldsOf(group)}>
              {(def) => <FieldRow def={def} wb={props.wb} defaults={props.defaults} sectors={props.sectors} memberCounts={props.memberCounts} />}
            </For>
          </section>
        )}
      </For>
    </div>
  );
}
