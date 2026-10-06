/**
 * Outbound delivery channels.
 *
 * Masking contract (server `notify::channels::secrets`): stored secrets come back masked — a
 * token as `••••` plus its last four characters, a webhook URL as `scheme://host/••••tail`. A
 * field sent back still containing `••••` keeps the stored value (same channel, same kind); any
 * other value replaces it, and `null` clears an optional secret. So an untouched secret is sent
 * back exactly as received, a replaced one is sent in full, and the name and kind of an existing
 * channel are fixed (a renamed or re-kinded channel could not restore its secrets).
 */
import { For, type JSX, Show, createMemo, createSignal, onCleanup, onMount } from "solid-js";
import { createStore, produce } from "solid-js/store";
import { Dialog, Empty, Switch, confirmAction, toast, toastError } from "@/components/ui";
import { tpl } from "@/i18n";
import { common } from "@/i18n/common";
import { settingsText } from "@/i18n/settings";
import { ApiError, api } from "@/lib/api";
import { onServerEvent } from "@/lib/events";
import { fmtDateTime, fmtRelative } from "@/lib/format";
import { isAdmin } from "@/lib/session";
import type { ChannelConfig, ChannelKind, ChannelRow } from "@/lib/types";
import { type Errors, Field, Gate, SIcon, type SIconName, createLoader, focusFirstInvalid } from "./shared";

const MASK = "••••";
const KINDS: ChannelKind[] = ["telegram", "feishu", "wecom", "slack", "discord", "webhook", "email"];
const KIND_ICON: Record<ChannelKind, SIconName> = {
  telegram: "send",
  feishu: "message",
  wecom: "message",
  slack: "hash",
  discord: "message",
  webhook: "link",
  email: "mail",
};

type TestResult = { state: "pending" } | { state: "done"; ok: boolean; error: string | null; at: number };

export function ChannelsCard() {
  const t = settingsText;
  const c = common;
  const loader = createLoader(() => api.channels());
  const [editing, setEditing] = createSignal<{ row: ChannelRow | null } | null>(null);
  const [tests, setTests] = createStore<Record<string, TestResult>>({});
  const [busy, setBusy] = createSignal<string | null>(null);

  onMount(() => {
    const off = onServerEvent(["resync"], () => void loader.reload());
    onCleanup(off);
  });

  const names = () => loader.value()?.channels.map((ch) => ch.name) ?? [];

  const runTest = async (name: string) => {
    setTests(name, { state: "pending" });
    try {
      const result = await api.testChannel(name);
      setTests(name, { state: "done", ok: result.ok, error: result.error, at: Date.now() });
      if (result.ok) toast(t().channels.test_ok, name, "success");
      else toast(t().channels.test_failed, `${name}: ${result.error ?? ""}`, "critical", 7000);
    } catch (error) {
      setTests(name, { state: "done", ok: false, error: error instanceof ApiError ? error.message : String(error), at: Date.now() });
      toastError(error, t().channels.test_failed);
    }
  };

  const toggle = async (row: ChannelRow, enabled: boolean) => {
    if (!row.config) return;
    setBusy(row.name);
    try {
      // Masked secrets go back unchanged; the server restores them.
      await api.putChannel(row.name, { label: row.label, enabled, config: row.config });
      toast(tpl(enabled ? t().channels.enabled_on : t().channels.enabled_off, { name: row.label || row.name }), undefined, "success");
    } catch (error) {
      toastError(error);
    } finally {
      setBusy(null);
      await loader.reload();
    }
  };

  const remove = async (row: ChannelRow) => {
    const ok = await confirmAction({
      title: t().channels.delete_title,
      body: tpl(t().channels.delete_body, { name: row.label || row.name }),
      confirmLabel: c().actions.delete,
      danger: true,
    });
    if (ok === null) return;
    setBusy(row.name);
    try {
      await api.deleteChannel(row.name);
      toast(t().channels.deleted, row.name, "success");
      setTests(produce((s) => delete s[row.name]));
    } catch (error) {
      toastError(error);
    } finally {
      setBusy(null);
      await loader.reload();
    }
  };

  const onSaved = async (name: string, test: boolean) => {
    await loader.reload();
    if (test) await runTest(name);
  };

  return (
    <div class="card">
      <div class="card-head">
        <div class="head-text">
          <h2>{t().channels.title}</h2>
          <div class="sub">{t().channels.sub}</div>
        </div>
        <span class="spacer" />
        <Show when={isAdmin() && (loader.value()?.channels.length ?? 0) > 0}>
          <button type="button" class="btn sm" onClick={() => setEditing({ row: null })}>
            <SIcon name="plus" size={14} />
            {t().channels.add}
          </button>
        </Show>
      </div>
      <Gate loader={loader}>
        {(data) => (
          <Show
            when={data().channels.length > 0}
            fallback={
              <Empty title={t().channels.empty_title} icon="send">
                <p class="empty-body">{t().channels.empty_body}</p>
                <Show when={isAdmin()}>
                  <button type="button" class="btn sm primary" onClick={() => setEditing({ row: null })}>
                    <SIcon name="plus" size={14} />
                    {t().channels.add}
                  </button>
                </Show>
              </Empty>
            }
          >
            <ul class="channel-list">
              <For each={data().channels}>
                {(row) => (
                  <ChannelItem
                    row={row}
                    test={tests[row.name]}
                    busy={busy() === row.name}
                    onToggle={(v) => void toggle(row, v)}
                    onTest={() => void runTest(row.name)}
                    onEdit={() => setEditing({ row })}
                    onDelete={() => void remove(row)}
                  />
                )}
              </For>
            </ul>
            <p class="card-note muted xs">
              <SIcon name="info" size={13} /> {t().channels.test_hint}
            </p>
          </Show>
        )}
      </Gate>
      <Show when={editing()}>
        {(e) => <ChannelDialog row={e().row} names={names()} onClose={() => setEditing(null)} onSaved={(name, test) => void onSaved(name, test)} />}
      </Show>
    </div>
  );
}

/** Where a channel delivers, from its (masked) configuration. */
function destination(config: ChannelConfig | null): string {
  if (!config) return "";
  switch (config.kind) {
    case "telegram":
      return `chat ${config.chat_id} · ${config.bot_token}`;
    case "feishu":
    case "wecom":
    case "slack":
    case "discord":
      return config.webhook_url;
    case "webhook":
      return `${config.url}${config.secret ? " · HMAC" : ""}`;
    case "email":
      return `${config.host}:${config.port} → ${config.to.join(", ")}`;
  }
}

function ChannelItem(props: {
  row: ChannelRow;
  test: TestResult | undefined;
  busy: boolean;
  onToggle: (v: boolean) => void;
  onTest: () => void;
  onEdit: () => void;
  onDelete: () => void;
}) {
  const t = settingsText;
  const c = common;
  const kindName = () => t().channels.kinds[props.row.kind] ?? props.row.kind;
  return (
    <li class="channel-item" classList={{ disabled: !props.row.enabled }}>
      <span class="channel-icon">
        <SIcon name={KIND_ICON[props.row.kind] ?? "send"} size={17} />
      </span>
      <div class="channel-main">
        <div class="channel-title">
          <b>{props.row.label || props.row.name}</b>
          <span class="chip outline">{kindName()}</span>
          <Show when={props.row.label && props.row.label !== props.row.name}>
            <span class="muted xs mono">{props.row.name}</span>
          </Show>
        </div>
        <Show when={props.row.readable} fallback={<div class="channel-dest warn-text">{t().channels.unreadable}</div>}>
          <div class="channel-dest mono" title={destination(props.row.config)}>
            {destination(props.row.config)}
          </div>
        </Show>
        <div class="channel-meta muted xs" title={fmtDateTime(props.row.updated_at)}>
          {t().channels.col_updated} {fmtRelative(props.row.updated_at)}
        </div>
        <Show when={props.test?.state === "pending"}>
          <div class="test-result pending">{t().channels.testing}</div>
        </Show>
        <Show when={props.test?.state === "done" ? (props.test as Extract<TestResult, { state: "done" }>) : null}>
          {(done) => (
            <div class="test-result" classList={{ ok: done().ok, failed: !done().ok }}>
              <SIcon name={done().ok ? "check" : "alert"} size={13} />
              <span>{done().ok ? t().channels.test_ok : `${t().channels.test_failed}: ${done().error ?? ""}`}</span>
            </div>
          )}
        </Show>
      </div>
      <div class="channel-actions">
        <Switch
          checked={props.row.enabled}
          disabled={!isAdmin() || !props.row.config || props.busy}
          onChange={(v) => props.onToggle(v)}
          label={<span class="visually-hidden">{t().channels.col_enabled}</span>}
        />
        <Show when={isAdmin()}>
          <button type="button" class="btn sm" disabled={!props.row.readable || props.test?.state === "pending"} onClick={() => props.onTest()}>
            <SIcon name="send" size={13} />
            {props.test?.state === "pending" ? t().channels.testing : t().channels.test}
          </button>
          <button type="button" class="btn sm" disabled={!props.row.readable} onClick={() => props.onEdit()}>
            {c().actions.edit}
          </button>
          <button type="button" class="btn sm ghost icon danger-ghost" disabled={props.busy} onClick={() => props.onDelete()} aria-label={c().actions.delete} title={c().actions.delete}>
            <SIcon name="trash" size={15} />
          </button>
        </Show>
      </div>
    </li>
  );
}

// ---------------------------------------------------------------------------------------------
// Add / edit dialog
// ---------------------------------------------------------------------------------------------

type SecretKey = "bot_token" | "webhook_url" | "url" | "secret" | "password";

interface Secret {
  /** The masked value from the server, or null when nothing is stored. */
  masked: string | null;
  mode: "keep" | "replace" | "remove";
  value: string;
}

interface DialogForm {
  kind: ChannelKind;
  name: string;
  label: string;
  enabled: boolean;
  chat_id: string;
  host: string;
  port: string;
  username: string;
  from: string;
  to: string;
  tls: "start_tls" | "implicit" | "none";
  secrets: Record<SecretKey, Secret>;
}

const NAME_RE = /^[A-Za-z0-9_-]{1,40}$/;

function secretFrom(value: string | null | undefined): Secret {
  return value ? { masked: value, mode: "keep", value: "" } : { masked: null, mode: "replace", value: "" };
}

function initialForm(row: ChannelRow | null, names: string[]): DialogForm {
  const empty: DialogForm = {
    kind: "telegram",
    name: suggestName("telegram", names),
    label: "",
    enabled: true,
    chat_id: "",
    host: "",
    port: "587",
    username: "",
    from: "",
    to: "",
    tls: "start_tls",
    secrets: {
      bot_token: secretFrom(null),
      webhook_url: secretFrom(null),
      url: secretFrom(null),
      secret: secretFrom(null),
      password: secretFrom(null),
    },
  };
  if (!row || !row.config) return empty;
  const cfg = row.config;
  const form: DialogForm = { ...empty, kind: row.kind, name: row.name, label: row.label, enabled: row.enabled };
  switch (cfg.kind) {
    case "telegram":
      form.secrets.bot_token = secretFrom(cfg.bot_token);
      form.chat_id = cfg.chat_id;
      break;
    case "feishu":
      form.secrets.webhook_url = secretFrom(cfg.webhook_url);
      form.secrets.secret = secretFrom(cfg.secret);
      break;
    case "wecom":
    case "slack":
    case "discord":
      form.secrets.webhook_url = secretFrom(cfg.webhook_url);
      break;
    case "webhook":
      form.secrets.url = secretFrom(cfg.url);
      form.secrets.secret = secretFrom(cfg.secret);
      break;
    case "email":
      form.host = cfg.host;
      form.port = String(cfg.port);
      form.username = cfg.username ?? "";
      form.secrets.password = secretFrom(cfg.password);
      form.from = cfg.from;
      form.to = cfg.to.join("\n");
      form.tls = cfg.tls;
      break;
  }
  return form;
}

function suggestName(kind: ChannelKind, names: string[]): string {
  if (!names.includes(kind)) return kind;
  for (let i = 2; i < 100; i++) if (!names.includes(`${kind}-${i}`)) return `${kind}-${i}`;
  return `${kind}-${Date.now() % 10_000}`;
}

/** Exactly what goes back to the server for one secret field. */
function secretOut(s: Secret, opts: { trim?: boolean } = {}): string {
  if (s.masked !== null && s.mode === "keep") return s.masked;
  return opts.trim === false ? s.value : s.value.trim();
}

function optionalSecretOut(s: Secret, opts: { trim?: boolean } = {}): string | null {
  if (s.masked !== null && s.mode === "keep") return s.masked;
  if (s.mode === "remove") return null;
  const v = opts.trim === false ? s.value : s.value.trim();
  return v ? v : null;
}

function recipients(text: string): string[] {
  return text
    .split(/[,;，；\n]/)
    .map((s) => s.trim())
    .filter(Boolean);
}

function buildConfig(f: DialogForm): ChannelConfig {
  const s = f.secrets;
  switch (f.kind) {
    case "telegram":
      return { kind: "telegram", bot_token: secretOut(s.bot_token), chat_id: f.chat_id.trim() };
    case "feishu":
      return { kind: "feishu", webhook_url: secretOut(s.webhook_url), secret: optionalSecretOut(s.secret) };
    case "wecom":
      return { kind: "wecom", webhook_url: secretOut(s.webhook_url) };
    case "slack":
      return { kind: "slack", webhook_url: secretOut(s.webhook_url) };
    case "discord":
      return { kind: "discord", webhook_url: secretOut(s.webhook_url) };
    case "webhook":
      return { kind: "webhook", url: secretOut(s.url), secret: optionalSecretOut(s.secret) };
    case "email":
      return {
        kind: "email",
        host: f.host.trim(),
        port: Number(f.port),
        username: f.username.trim() || null,
        password: optionalSecretOut(s.password, { trim: false }),
        from: f.from.trim(),
        to: recipients(f.to),
        tls: f.tls,
      };
  }
}

const isLoopback = (host: string) => host === "localhost" || host === "127.0.0.1" || host === "[::1]";
const ADDRESS = /^[^\s@<>()",;:]+@[^\s@<>()",;:]+$/;

function validAddress(value: string): boolean {
  const v = value.trim();
  const m = /^(.*)<([^<>]+)>$/.exec(v);
  return ADDRESS.test((m ? m[2] : v).trim());
}

/** Client-side mirror of `notify::channels::validate`. */
function validate(f: DialogForm, existing: string[], isNew: boolean): Errors {
  const t = settingsText().channels;
  const v = settingsText().v;
  const errors: Errors = {};
  if (isNew) {
    if (!NAME_RE.test(f.name)) errors.name = t.name_invalid;
    else if (existing.includes(f.name)) errors.name = t.name_taken;
  }
  if ([...f.label.trim()].length > 60) errors.label = t.label_too_long;
  const editing = (k: SecretKey) => !(f.secrets[k].masked !== null && f.secrets[k].mode !== "replace");
  const url = (k: SecretKey) => {
    if (!editing(k)) return;
    const value = f.secrets[k].value.trim();
    if (!value) errors[k] = v.required;
    else if (value.includes(MASK)) errors[k] = t.errors.masked;
    else {
      try {
        const parsed = new URL(value);
        const ok = parsed.protocol === "https:" || (parsed.protocol === "http:" && isLoopback(parsed.hostname));
        if (!ok) errors[k] = t.errors.url_https;
      } catch {
        errors[k] = t.errors.url_invalid;
      }
    }
  };
  const optional = (k: SecretKey) => {
    if (editing(k) && f.secrets[k].value.includes(MASK)) errors[k] = t.errors.masked;
  };
  switch (f.kind) {
    case "telegram": {
      if (editing("bot_token")) {
        const token = f.secrets.bot_token.value.trim();
        if (!token) errors.bot_token = v.required;
        else if (token.includes(MASK)) errors.bot_token = t.errors.masked;
        else if (/[\s/?#]/.test(token)) errors.bot_token = t.errors.token_chars;
      }
      if (!f.chat_id.trim()) errors.chat_id = v.required;
      break;
    }
    case "feishu":
      url("webhook_url");
      optional("secret");
      break;
    case "wecom":
    case "slack":
    case "discord":
      url("webhook_url");
      break;
    case "webhook":
      url("url");
      optional("secret");
      break;
    case "email": {
      if (!f.host.trim()) errors.host = v.required;
      const port = Number(f.port);
      if (!/^\d+$/.test(f.port.trim()) || port < 1 || port > 65535) errors.port = t.errors.port;
      optional("password");
      if (!f.from.trim()) errors.from = v.required;
      else if (!validAddress(f.from)) errors.from = tpl(t.errors.address, { value: f.from.trim() });
      const to = recipients(f.to);
      if (!to.length) errors.to = t.errors.recipients;
      else {
        const bad = to.find((a) => !validAddress(a));
        if (bad) errors.to = tpl(t.errors.address, { value: bad });
      }
      break;
    }
  }
  return errors;
}

/** Server message → field. Messages name the field but never echo a secret. */
function fieldFor(kind: ChannelKind, message: string): string | null {
  if (/channel names/i.test(message)) return "name";
  if (/bot token/i.test(message)) return "bot_token";
  if (/chat id/i.test(message)) return "chat_id";
  if (/signing secret/i.test(message)) return "secret";
  if (/SMTP host/i.test(message)) return "host";
  if (/SMTP port/i.test(message)) return "port";
  if (/SMTP password/i.test(message)) return "password";
  if (/^.*Sender/.test(message)) return "from";
  if (/recipient/i.test(message)) return "to";
  if (/URL/.test(message)) return kind === "webhook" ? "url" : "webhook_url";
  return null;
}

function ChannelDialog(props: { row: ChannelRow | null; names: string[]; onClose: () => void; onSaved: (name: string, test: boolean) => void }) {
  const t = settingsText;
  const c = common;
  const isNew = !props.row;
  const [form, setForm] = createStore<DialogForm>(initialForm(props.row, props.names));
  const [submitted, setSubmitted] = createSignal(false);
  const [touched, setTouched] = createStore<Record<string, boolean>>({});
  const [serverErrors, setServerErrors] = createStore<Errors>({});
  const [general, setGeneral] = createSignal<string | null>(null);
  const [saving, setSaving] = createSignal(false);
  const [nameEdited, setNameEdited] = createSignal(false);

  const errors = createMemo(() => validate(form, props.names, isNew));
  const err = (k: string) => serverErrors[k] ?? (submitted() || touched[k] ? errors()[k] : undefined);

  const set = <K extends keyof DialogForm>(k: K, value: DialogForm[K]) => {
    setForm(
      produce((d: DialogForm) => {
        d[k] = value;
      }),
    );
    setServerErrors(k as string, undefined);
    setGeneral(null);
  };
  const setSecret = (k: SecretKey, value: Secret) => {
    setForm("secrets", k, value);
    setServerErrors(k, undefined);
    setGeneral(null);
  };
  const touch = (k: string) => setTouched(k, true);

  const chooseKind = (kind: ChannelKind) => {
    set("kind", kind);
    if (!nameEdited()) set("name", suggestName(kind, props.names));
    if (kind === "email" && !form.port) set("port", "587");
  };

  const save = async (test: boolean) => {
    setSubmitted(true);
    if (Object.values(errors()).some(Boolean)) {
      focusFirstInvalid();
      return;
    }
    setSaving(true);
    setGeneral(null);
    try {
      await api.putChannel(form.name, { label: form.label.trim(), enabled: form.enabled, config: buildConfig(form) });
      toast(t().channels.saved, form.label.trim() || form.name, "success");
      props.onClose();
      props.onSaved(form.name, test);
    } catch (error) {
      if (error instanceof ApiError && error.status === 400) {
        const k = fieldFor(form.kind, error.message);
        const masked = /masked/i.test(error.message);
        if (k) {
          setServerErrors(k, masked ? t().channels.errors.masked : error.message.replace(/^invalid channel configuration:\s*/i, ""));
          focusFirstInvalid();
        } else setGeneral(error.message);
      }
      toastError(error);
    } finally {
      setSaving(false);
    }
  };

  const textField = (k: "chat_id" | "host" | "username" | "from", label: string, opts: { placeholder?: string; hint?: string; mono?: boolean; optional?: boolean } = {}) => (
    <Field id={`ch-${k}`} label={opts.optional ? <>{label} <span class="muted">· {t().form.optional}</span></> : label} hint={opts.hint} error={err(k)}>
      <input
        id={`ch-${k}`}
        class="input"
        classList={{ mono: !!opts.mono, invalid: !!err(k) }}
        aria-invalid={err(k) ? "true" : "false"}
        autocomplete="off"
        spellcheck={false}
        placeholder={opts.placeholder}
        value={form[k]}
        onInput={(e) => set(k, e.currentTarget.value)}
        onBlur={() => touch(k)}
      />
    </Field>
  );

  const secretField = (k: SecretKey, label: string, opts: { hint?: string; optional?: boolean; url?: boolean; placeholder?: string } = {}) => (
    <SecretField
      id={`ch-${k}`}
      label={opts.optional ? <>{label} <span class="muted">· {t().form.optional}</span></> : label}
      hint={opts.hint}
      state={form.secrets[k]}
      optional={opts.optional}
      url={opts.url}
      placeholder={opts.placeholder}
      error={err(k)}
      onChange={(s) => setSecret(k, s)}
      onBlur={() => touch(k)}
    />
  );

  return (
    <Dialog
      title={isNew ? t().channels.dialog_add : t().channels.dialog_edit}
      subtitle={isNew ? undefined : `${t().channels.kinds[form.kind]} · ${form.name}`}
      onClose={() => !saving() && props.onClose()}
      wide={form.kind === "email"}
      footer={
        <>
          <button type="button" class="btn" onClick={() => props.onClose()} disabled={saving()}>
            {c().actions.cancel}
          </button>
          <button type="button" class="btn" onClick={() => void save(true)} disabled={saving()}>
            <SIcon name="send" size={13} />
            {t().channels.save_test}
          </button>
          <button type="button" class="btn primary" onClick={() => void save(false)} disabled={saving()}>
            {saving() ? c().actions.saving : t().channels.save}
          </button>
        </>
      }
    >
      <form
        class="stack channel-form"
        style={{ gap: "16px" }}
        novalidate
        onSubmit={(e) => {
          e.preventDefault();
          void save(false);
        }}
      >
        <Show
          when={isNew}
          fallback={
            <div class="kind-static">
              <SIcon name={KIND_ICON[form.kind]} size={16} />
              <b>{t().channels.kinds[form.kind]}</b>
              <span class="muted mono xs">{form.name}</span>
            </div>
          }
        >
          <div class="field">
            <span class="field-label">{t().channels.kind}</span>
            <div class="kind-grid" role="radiogroup" aria-label={t().channels.kind}>
              <For each={KINDS}>
                {(kind) => (
                  <button type="button" class="kind-btn" role="radio" aria-checked={form.kind === kind} onClick={() => chooseKind(kind)}>
                    <SIcon name={KIND_ICON[kind]} size={16} />
                    <span>{t().channels.kinds[kind]}</span>
                  </button>
                )}
              </For>
            </div>
          </div>
        </Show>

        <details class="setup-hint">
          <summary>
            <SIcon name="info" size={14} />
            {t().channels.setup} · {t().channels.kinds[form.kind]}
          </summary>
          <p>{t().channels.hints[form.kind]}</p>
        </details>

        <div class="dialog-grid">
          <Show when={isNew}>
            <Field id="ch-name" label={t().channels.name} hint={t().channels.name_hint} error={err("name")}>
              <input
                id="ch-name"
                class="input mono"
                classList={{ invalid: !!err("name") }}
                aria-invalid={err("name") ? "true" : "false"}
                autocomplete="off"
                spellcheck={false}
                maxLength={40}
                value={form.name}
                onInput={(e) => {
                  setNameEdited(true);
                  set("name", e.currentTarget.value);
                }}
                onBlur={() => touch("name")}
              />
            </Field>
          </Show>
          <Field
            id="ch-label"
            label={
              <>
                {t().channels.label} <span class="muted">· {t().form.optional}</span>
              </>
            }
            hint={t().channels.label_hint}
            error={err("label")}
          >
            <input
              id="ch-label"
              class="input"
              classList={{ invalid: !!err("label") }}
              aria-invalid={err("label") ? "true" : "false"}
              maxLength={80}
              value={form.label}
              onInput={(e) => set("label", e.currentTarget.value)}
              onBlur={() => touch("label")}
            />
          </Field>
        </div>

        <Show when={!isNew && Object.values(form.secrets).some((s) => s.masked !== null)}>
          <p class="muted xs secret-note">
            <SIcon name="lock" size={13} /> {t().channels.secret_hint}
          </p>
        </Show>

        <Show when={form.kind === "telegram"}>
          {secretField("bot_token", t().channels.fields.bot_token, { placeholder: "123456789:AA…" })}
          {textField("chat_id", t().channels.fields.chat_id, { placeholder: "-1001234567890", mono: true })}
        </Show>
        <Show when={form.kind === "feishu"}>
          {secretField("webhook_url", t().channels.fields.webhook_url, { url: true, placeholder: "https://open.feishu.cn/open-apis/bot/v2/hook/…" })}
          {secretField("secret", t().channels.fields.secret_feishu, { optional: true, hint: t().channels.fields.secret_feishu_hint })}
        </Show>
        <Show when={form.kind === "wecom"}>
          {secretField("webhook_url", t().channels.fields.webhook_url, { url: true, placeholder: "https://qyapi.weixin.qq.com/cgi-bin/webhook/send?key=…" })}
        </Show>
        <Show when={form.kind === "slack"}>
          {secretField("webhook_url", t().channels.fields.webhook_url, { url: true, placeholder: "https://hooks.slack.com/services/…" })}
        </Show>
        <Show when={form.kind === "discord"}>
          {secretField("webhook_url", t().channels.fields.webhook_url, { url: true, placeholder: "https://discord.com/api/webhooks/…" })}
        </Show>
        <Show when={form.kind === "webhook"}>
          {secretField("url", t().channels.fields.url, { url: true, placeholder: "https://example.com/hooks/hone-quant" })}
          {secretField("secret", t().channels.fields.secret_webhook, { optional: true, hint: t().channels.fields.secret_webhook_hint })}
        </Show>
        <Show when={form.kind === "email"}>
          <div class="dialog-grid email-grid">
            {textField("host", t().channels.fields.host, { placeholder: "smtp.example.com", mono: true })}
            <Field id="ch-port" label={t().channels.fields.port} error={err("port")}>
              <input
                id="ch-port"
                class="input num"
                classList={{ invalid: !!err("port") }}
                aria-invalid={err("port") ? "true" : "false"}
                inputmode="numeric"
                value={form.port}
                onInput={(e) => set("port", e.currentTarget.value)}
                onBlur={() => touch("port")}
              />
            </Field>
            <Field id="ch-tls" label={t().channels.fields.tls}>
              <select id="ch-tls" class="select" value={form.tls} onChange={(e) => set("tls", e.currentTarget.value as DialogForm["tls"])}>
                <option value="start_tls">{t().channels.fields.tls_start}</option>
                <option value="implicit">{t().channels.fields.tls_implicit}</option>
                <option value="none">{t().channels.fields.tls_none}</option>
              </select>
            </Field>
            {textField("username", t().channels.fields.username, { optional: true, mono: true })}
            {secretField("password", t().channels.fields.password, { optional: true })}
            {textField("from", t().channels.fields.from, { placeholder: t().channels.fields.from_placeholder })}
            <Field id="ch-to" label={t().channels.fields.to} hint={t().channels.fields.to_hint} error={err("to")} class="span-2">
              <textarea
                id="ch-to"
                class="textarea mono"
                classList={{ invalid: !!err("to") }}
                aria-invalid={err("to") ? "true" : "false"}
                rows={2}
                value={form.to}
                onInput={(e) => set("to", e.currentTarget.value)}
                onBlur={() => touch("to")}
              />
            </Field>
          </div>
        </Show>

        <Switch checked={form.enabled} onChange={(v) => set("enabled", v)} label={<span class="switch-label">{t().channels.enabled}</span>} />

        <Show when={general()}>
          <div class="callout critical" role="alert">
            <SIcon name="alert" size={16} />
            <span>{general()}</span>
          </div>
        </Show>
        <button type="submit" hidden />
      </form>
    </Dialog>
  );
}

/**
 * A stored secret is shown masked with Replace (and Remove when optional); while replacing, the
 * full new value is typed. Untouched secrets are sent back as received.
 */
function SecretField(props: {
  id: string;
  label: JSX.Element;
  hint?: string;
  state: Secret;
  optional?: boolean;
  url?: boolean;
  placeholder?: string;
  error?: string;
  onChange: (s: Secret) => void;
  onBlur: () => void;
}) {
  const t = settingsText;
  const [reveal, setReveal] = createSignal(false);
  const kept = () => props.state.masked !== null && props.state.mode !== "replace";
  return (
    <Field id={props.id} label={props.label} hint={props.hint} error={props.error}>
      <Show
        when={kept()}
        fallback={
          <div class="secret-input">
            <div class="secret-box" classList={{ plain: !!props.url }}>
              <input
                id={props.id}
                class="input mono"
                classList={{ invalid: !!props.error }}
                aria-invalid={props.error ? "true" : "false"}
                type={props.url || reveal() ? "text" : "password"}
                autocomplete="new-password"
                spellcheck={false}
                placeholder={props.placeholder}
                value={props.state.value}
                onInput={(e) => props.onChange({ ...props.state, value: e.currentTarget.value })}
                onBlur={() => props.onBlur()}
              />
              <Show when={!props.url}>
                <button
                  type="button"
                  class="btn ghost icon sm reveal"
                  onClick={() => setReveal((v) => !v)}
                  aria-label={reveal() ? t().form.hide : t().form.show}
                  title={reveal() ? t().form.hide : t().form.show}
                >
                  <SIcon name={reveal() ? "eye_off" : "eye"} size={15} />
                </button>
              </Show>
            </div>
            <Show when={props.state.masked !== null}>
              <button type="button" class="btn sm" onClick={() => props.onChange({ ...props.state, mode: "keep", value: "" })}>
                {t().channels.keep}
              </button>
            </Show>
          </div>
        }
      >
        <div class="secret-kept" classList={{ removed: props.state.mode === "remove" }}>
          <SIcon name="lock" size={14} />
          <span class="mono value">{props.state.mode === "remove" ? t().channels.removed : props.state.masked}</span>
          <Show when={props.state.mode === "keep"}>
            <span class="chip green">{t().channels.configured}</span>
          </Show>
          <span class="spacer" />
          <Show
            when={props.state.mode === "keep"}
            fallback={
              <button type="button" class="btn sm" onClick={() => props.onChange({ ...props.state, mode: "keep" })}>
                {t().channels.keep}
              </button>
            }
          >
            <button type="button" class="btn sm" onClick={() => props.onChange({ ...props.state, mode: "replace", value: "" })}>
              {t().channels.replace}
            </button>
            <Show when={props.optional}>
              <button type="button" class="btn sm danger" onClick={() => props.onChange({ ...props.state, mode: "remove" })}>
                {t().channels.remove}
              </button>
            </Show>
          </Show>
        </div>
      </Show>
    </Field>
  );
}
