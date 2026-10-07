/**
 * "New portfolio": name, description, initial cash, the strategy version to start with (from the
 * shared library), the automation mode and, for administrators, the owner. The app switches to
 * the new portfolio once it exists.
 */
import { For, Show, createMemo, createResource, createSignal, onMount } from "solid-js";
import { Dialog, Segmented, toast, toastError } from "@/components/ui";
import { tpl } from "@/i18n";
import { common } from "@/i18n/common";
import { portfoliosText } from "@/i18n/portfolios";
import { ApiError, api } from "@/lib/api";
import { strategyName } from "@/lib/names";
import { selectPortfolio, upsertPortfolio } from "@/lib/portfolio";
import { honeclawAuth, isAdmin } from "@/lib/session";
import type { AutomationMode, NewPortfolio, Portfolio, User } from "@/lib/types";
import "@/styles/portfolios.css";
import { DEFAULT_CASH, focusInvalid, formatCash, nameProblem, parseCash } from "./form";

type Key = "name" | "cash" | "strategy" | "owner";
type Errors = Partial<Record<Key, string>>;

const MODES: AutomationMode[] = ["auto", "approval", "paused"];

/** Which field a server validation message or path is about. */
function fieldFor(text: string): Key | null {
  if (/owner|user/i.test(text)) return "owner";
  if (/cash/i.test(text)) return "cash";
  if (/strategy/i.test(text)) return "strategy";
  if (/name/i.test(text)) return "name";
  return null;
}

export default function NewPortfolioDialog(props: { onClose: () => void; onCreated?: (portfolio: Portfolio) => void }) {
  const t = portfoliosText;
  const c = common;
  const [name, setName] = createSignal("");
  const [description, setDescription] = createSignal("");
  const [cashText, setCashText] = createSignal(formatCash(DEFAULT_CASH));
  const [versionId, setVersionId] = createSignal<number | null>(null);
  const [mode, setMode] = createSignal<AutomationMode>("auto");
  const [owner, setOwner] = createSignal("");
  const [touched, setTouched] = createSignal<Partial<Record<Key, boolean>>>({});
  const [submitted, setSubmitted] = createSignal(false);
  const [serverErrors, setServerErrors] = createSignal<Errors>({});
  const [formError, setFormError] = createSignal<string | null>(null);
  const [busy, setBusy] = createSignal(false);
  let nameInput: HTMLInputElement | undefined;
  onMount(() => requestAnimationFrame(() => nameInput?.focus()));

  // The shared library; `active` is the current portfolio's version (the default choice).
  const [strategy] = createResource(() => api.strategy().catch(() => null));
  const pickOwner = () => isAdmin() && !honeclawAuth();
  const [users] = createResource(() => (pickOwner() ? api.users().then((r) => r.users) : Promise.resolve([] as User[])).catch(() => [] as User[]));

  const versions = () => strategy.latest?.versions ?? [];
  const activeId = () => strategy.latest?.active?.id ?? null;
  const chosenVersion = () => versionId() ?? activeId() ?? versions().reduce<number | null>((max, v) => (max === null || v.id > max ? v.id : max), null);

  const errors = createMemo<Errors>(() => {
    const e: Errors = {};
    const problem = nameProblem(name());
    if (problem) e.name = problem === "required" ? t().create.err_name_required : t().create.err_name_long;
    if (parseCash(cashText()) === null) e.cash = t().create.err_cash;
    return e;
  });
  const err = (key: Key) => serverErrors()[key] ?? (submitted() || touched()[key] ? errors()[key] : undefined);
  const touch = (key: Key) => setTouched((prev) => ({ ...prev, [key]: true }));
  const clearServer = (key: Key) => {
    setServerErrors((prev) => ({ ...prev, [key]: undefined }));
    setFormError(null);
  };

  const submit = async () => {
    setSubmitted(true);
    const cash = parseCash(cashText());
    if (Object.values(errors()).some(Boolean) || cash === null) {
      focusInvalid();
      return;
    }
    const body: NewPortfolio = {
      name: name().trim(),
      description: description().trim(),
      initial_cash: cash,
      automation_mode: mode(),
    };
    const version = chosenVersion();
    if (version !== null) body.strategy_version_id = version;
    if (isAdmin()) body.owner = owner() || null;
    setBusy(true);
    setFormError(null);
    try {
      const { portfolio } = await api.createPortfolio(body);
      toast(tpl(t().create.created, { name: portfolio.name }), t().create.created_body, "success");
      props.onCreated?.(portfolio);
      props.onClose();
      upsertPortfolio(portfolio);
      selectPortfolio(portfolio.id);
    } catch (error) {
      if (error instanceof ApiError && error.status === 409) {
        setServerErrors({ name: t().create.err_name_taken });
        focusInvalid();
        return;
      }
      if (error instanceof ApiError && error.status === 400) {
        const mapped: Errors = {};
        const general: string[] = [];
        const problems = error.fields?.length ? error.fields.map((f) => ({ key: fieldFor(f.path) ?? fieldFor(f.message), text: f.message })) : [{ key: fieldFor(error.message), text: error.message }];
        for (const p of problems) {
          if (p.key) mapped[p.key] = p.text;
          else general.push(p.text);
        }
        setServerErrors(mapped);
        setFormError(general.length ? general.join("\n") : null);
        focusInvalid();
        return;
      }
      toastError(error);
    } finally {
      setBusy(false);
    }
  };

  return (
    <Dialog
      title={t().create.title}
      subtitle={t().create.subtitle}
      onClose={() => !busy() && props.onClose()}
      footer={
        <>
          <button type="button" class="btn" onClick={() => props.onClose()} disabled={busy()}>
            {c().actions.cancel}
          </button>
          <button type="button" class="btn primary" onClick={() => void submit()} disabled={busy()}>
            {busy() ? t().create.creating : t().create.submit}
          </button>
        </>
      }
    >
      <form
        class="stack pf-form"
        style={{ gap: "16px" }}
        novalidate
        autocomplete="off"
        onSubmit={(e) => {
          e.preventDefault();
          void submit();
        }}
      >
        <div class="field">
          <label for="pf-new-name">{t().create.name}</label>
          <input
            ref={nameInput}
            id="pf-new-name"
            class="input"
            classList={{ invalid: !!err("name") }}
            aria-invalid={err("name") ? "true" : "false"}
            placeholder={t().create.name_placeholder}
            value={name()}
            onInput={(e) => {
              setName(e.currentTarget.value);
              clearServer("name");
            }}
            onBlur={() => touch("name")}
          />
          <Show when={err("name")} fallback={<span class="hint">{t().create.name_hint}</span>}>
            <span class="error-text" role="alert">
              {err("name")}
            </span>
          </Show>
        </div>

        <div class="field">
          <label for="pf-new-description">{t().create.description}</label>
          <textarea
            id="pf-new-description"
            class="textarea"
            rows={2}
            maxLength={500}
            placeholder={t().create.description_placeholder}
            value={description()}
            onInput={(e) => setDescription(e.currentTarget.value)}
          />
        </div>

        <div class="pf-form-grid">
          <div class="field">
            <label for="pf-new-cash">{t().create.cash}</label>
            <div class="input-group pf-money">
              <span class="addon pre">$</span>
              <input
                id="pf-new-cash"
                class="input num"
                classList={{ invalid: !!err("cash") }}
                aria-invalid={err("cash") ? "true" : "false"}
                inputmode="decimal"
                value={cashText()}
                onInput={(e) => {
                  setCashText(e.currentTarget.value);
                  clearServer("cash");
                }}
                onBlur={() => {
                  touch("cash");
                  const value = parseCash(cashText());
                  if (value !== null) setCashText(formatCash(value));
                }}
              />
              <span class="addon">USD</span>
            </div>
            <Show when={err("cash")} fallback={<span class="hint">{t().create.cash_hint}</span>}>
              <span class="error-text" role="alert">
                {err("cash")}
              </span>
            </Show>
          </div>

          <div class="field">
            <label for="pf-new-strategy">{t().create.strategy}</label>
            <Show
              when={versions().length > 0}
              fallback={
                <span class="hint pf-static">
                  {strategy.loading ? c().states.loading : strategy.latest ? t().create.strategy_none : t().create.strategy_unavailable}
                </span>
              }
            >
              <select
                id="pf-new-strategy"
                class="select"
                classList={{ invalid: !!err("strategy") }}
                aria-invalid={err("strategy") ? "true" : "false"}
                value={String(chosenVersion() ?? "")}
                onChange={(e) => {
                  setVersionId(Number(e.currentTarget.value));
                  clearServer("strategy");
                }}
              >
                <For each={versions()}>
                  {(v) => (
                    <option value={String(v.id)}>
                      {`#${v.id} · ${strategyName(v)}${v.id === activeId() ? ` · ${t().create.strategy_current}` : ""}`}
                    </option>
                  )}
                </For>
              </select>
              <Show when={err("strategy")} fallback={<span class="hint">{t().create.strategy_hint}</span>}>
                <span class="error-text" role="alert">
                  {err("strategy")}
                </span>
              </Show>
            </Show>
          </div>
        </div>

        <div class="field">
          <span class="field-label">{t().create.mode}</span>
          <Segmented
            label={t().create.mode}
            value={mode()}
            onChange={setMode}
            options={MODES.map((m) => ({ value: m, label: c().mode[m], title: c().mode[`${m}_hint` as const] }))}
          />
          <span class="hint">{c().mode[`${mode()}_hint` as const]}</span>
        </div>

        <Show when={isAdmin()}>
          <div class="field">
            <label for="pf-new-owner">{t().create.owner}</label>
            <Show
              when={pickOwner()}
              fallback={
                <>
                  <span class="pf-static small">{t().create.owner_shared}</span>
                  <span class="hint">{t().create.owner_honeclaw}</span>
                </>
              }
            >
              <select
                id="pf-new-owner"
                class="select"
                classList={{ invalid: !!err("owner") }}
                aria-invalid={err("owner") ? "true" : "false"}
                value={owner()}
                onChange={(e) => {
                  setOwner(e.currentTarget.value);
                  clearServer("owner");
                }}
              >
                <option value="">{t().create.owner_shared}</option>
                <For each={users.latest ?? []}>
                  {(u) => <option value={u.username}>{tpl(t().create.owner_role, { name: u.display_name || u.username, role: c().roles[u.role] })}</option>}
                </For>
              </select>
              <Show when={err("owner")} fallback={<span class="hint">{owner() ? t().create.owner_hint_user : t().create.owner_hint_shared}</span>}>
                <span class="error-text" role="alert">
                  {err("owner")}
                </span>
              </Show>
            </Show>
          </div>
        </Show>

        <Show when={formError()}>
          <div class="callout critical" role="alert">
            <span style={{ "white-space": "pre-line" }}>{formError()}</span>
          </div>
        </Show>
        <button type="submit" hidden />
      </form>
    </Dialog>
  );
}
