/**
 * Building blocks shared by every settings section: a small data loader, the settings bundle
 * context, a form-state hook (dirty tracking, client validation, server error mapping, reset to
 * defaults, unsaved-change guards) and the field / footer components that render it.
 */
import { A, useBeforeLeave } from "@solidjs/router";
import {
  type Accessor,
  type JSX,
  type ParentProps,
  Show,
  createContext,
  createEffect,
  createMemo,
  createSignal,
  on,
  onCleanup,
  useContext,
} from "solid-js";
import { createStore, produce, reconcile } from "solid-js/store";
import { Icon, type IconName } from "@/components/Icon";
import { Dialog, ErrorState, Loading, toast, toastError } from "@/components/ui";
import { locale, tpl } from "@/i18n";
import { common } from "@/i18n/common";
import { settingsText } from "@/i18n/settings";
import { ApiError } from "@/lib/api";
import { isAdmin, meta } from "@/lib/session";
import type { SettingsBundle } from "@/lib/types";
import { sameNumText } from "./num";

// ---------------------------------------------------------------------------------------------
// Icons not in the shared set
// ---------------------------------------------------------------------------------------------

const EXTRA = {
  wallet:
    "M19 7V5a2 2 0 0 0-2-2H5a2 2 0 0 0 0 4h14a2 2 0 0 1 2 2v3h-4a2 2 0 0 0 0 4h4v3a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2V5",
  users:
    "M16 21v-2a4 4 0 0 0-4-4H6a4 4 0 0 0-4 4v2M9 11a4 4 0 1 0 0-8 4 4 0 0 0 0 8zM22 21v-2a4 4 0 0 0-3-3.9M16 3.1a4 4 0 0 1 0 7.8",
  monitor: "M4 3h16a2 2 0 0 1 2 2v10a2 2 0 0 1-2 2H4a2 2 0 0 1-2-2V5a2 2 0 0 1 2-2zM8 21h8M12 17v4",
  eye: "M2 12s3.6-7 10-7 10 7 10 7-3.6 7-10 7S2 12 2 12zM12 15a3 3 0 1 0 0-6 3 3 0 0 0 0 6z",
  eye_off:
    "M9.9 4.2A9.7 9.7 0 0 1 12 4c6.4 0 10 8 10 8a17.6 17.6 0 0 1-2.2 3.2M6.6 6.6C3.9 8.4 2 12 2 12s3.6 8 10 8a9.7 9.7 0 0 0 5.4-1.6M14.1 14.1a3 3 0 1 1-4.2-4.2M2 2l20 20",
  copy: "M9 9h11a2 2 0 0 1 2 2v9a2 2 0 0 1-2 2H11a2 2 0 0 1-2-2zM5 15H4a2 2 0 0 1-2-2V4a2 2 0 0 1 2-2h9a2 2 0 0 1 2 2v1",
  mail: "M4 4h16a2 2 0 0 1 2 2v12a2 2 0 0 1-2 2H4a2 2 0 0 1-2-2V6a2 2 0 0 1 2-2zM22 6l-10 7L2 6",
  link: "M10 13a5 5 0 0 0 7.5.5l3-3a5 5 0 0 0-7-7l-1.7 1.7M14 11a5 5 0 0 0-7.5-.5l-3 3a5 5 0 0 0 7 7l1.7-1.7",
  message: "M21 15a2 2 0 0 1-2 2H7l-4 4V5a2 2 0 0 1 2-2h14a2 2 0 0 1 2 2z",
  hash: "M4 9h16M4 15h16M10 3 8 21M16 3l-2 18",
  key: "M21 2l-2 2M15.5 7.5l3 3L22 7l-3-3M15.5 7.5 11.4 11.6M7.5 22a5.5 5.5 0 1 0 0-11 5.5 5.5 0 0 0 0 11z",
} as const;

type ExtraName = keyof typeof EXTRA;
export type SIconName = IconName | ExtraName;

export function SIcon(props: { name: SIconName; size?: number; class?: string; style?: JSX.CSSProperties }) {
  const isExtra = () => (props.name as string) in EXTRA;
  return (
    <Show when={isExtra()} fallback={<Icon name={props.name as IconName} size={props.size} class={props.class} style={props.style} />}>
      <svg
        class={props.class}
        style={props.style}
        width={props.size ?? 18}
        height={props.size ?? 18}
        viewBox="0 0 24 24"
        fill="none"
        stroke="currentColor"
        stroke-width="1.7"
        stroke-linecap="round"
        stroke-linejoin="round"
        aria-hidden="true"
      >
        <path d={EXTRA[props.name as ExtraName]} />
      </svg>
    </Show>
  );
}

/**
 * Localised name of an API row. (The shared `pick` only accepts index-signature types, so it
 * rejects interfaces such as `Asset`.)
 */
export function localName(row: { name_zh: string; name_en: string } | null | undefined): string {
  if (!row) return "";
  return locale() === "zh" ? row.name_zh || row.name_en : row.name_en || row.name_zh;
}

// ---------------------------------------------------------------------------------------------
// Loading
// ---------------------------------------------------------------------------------------------

export interface Loader<T> {
  value: Accessor<T | undefined>;
  error: Accessor<unknown>;
  loading: Accessor<boolean>;
  reload: () => Promise<void>;
  set: (value: T) => void;
}

/** Fetches once on creation; `reload` refetches and keeps the previous value while it runs. */
export function createLoader<T>(fetcher: () => Promise<T>): Loader<T> {
  const [value, setValue] = createSignal<T | undefined>(undefined);
  const [error, setError] = createSignal<unknown>(undefined);
  const [loading, setLoading] = createSignal(false);
  let seq = 0;
  const reload = async () => {
    const id = ++seq;
    setLoading(true);
    try {
      const next = await fetcher();
      if (id !== seq) return;
      setValue(() => next);
      setError(undefined);
    } catch (e) {
      if (id !== seq) return;
      setError(() => e);
      if (value() !== undefined) toastError(e);
    } finally {
      if (id === seq) setLoading(false);
    }
  };
  void reload();
  return { value, error, loading, reload, set: (v: T) => setValue(() => v) };
}

/** Renders children once a loader has a value; loading and error states otherwise. */
export function Gate<T>(props: { loader: Loader<T>; children: (value: Accessor<NonNullable<T>>) => JSX.Element }) {
  return (
    <Show
      when={props.loader.value()}
      fallback={
        <Show when={props.loader.error()} fallback={<Loading />}>
          <div class="card">
            <ErrorState error={props.loader.error()} onRetry={() => void props.loader.reload()} />
          </div>
        </Show>
      }
    >
      {(value) => props.children(value as Accessor<NonNullable<T>>)}
    </Show>
  );
}

// ---------------------------------------------------------------------------------------------
// Settings bundle context
// ---------------------------------------------------------------------------------------------

export interface SettingsContext {
  bundle: Loader<SettingsBundle>;
}

export const SettingsCtx = createContext<SettingsContext>();

export function useSettings(): SettingsContext {
  const ctx = useContext(SettingsCtx);
  if (!ctx) throw new Error("settings context missing");
  return ctx;
}

// ---------------------------------------------------------------------------------------------
// Value comparison
// ---------------------------------------------------------------------------------------------

/** JSON with sorted keys and rounded numbers, for order-insensitive equality. */
export function stable(value: unknown): string {
  return JSON.stringify(value, (_key, v) => {
    if (typeof v === "number") return Number.isFinite(v) ? Number(v.toPrecision(12)) : v;
    if (v && typeof v === "object" && !Array.isArray(v)) {
      return Object.fromEntries(Object.entries(v as Record<string, unknown>).sort(([a], [b]) => (a < b ? -1 : a > b ? 1 : 0)));
    }
    return v;
  });
}

function sameField(a: unknown, b: unknown): boolean {
  if (typeof a === "string" && typeof b === "string") return sameNumText(a, b);
  if (typeof a === "object" || typeof b === "object") return stable(a) === stable(b);
  return a === b;
}

// ---------------------------------------------------------------------------------------------
// Local confirmation (for choices that are not audited actions, e.g. leaving with unsaved edits)
// ---------------------------------------------------------------------------------------------

interface LocalConfirmState {
  title: string;
  body: string;
  confirmLabel: string;
  cancelLabel: string;
  danger: boolean;
  resolve: (ok: boolean) => void;
}

const [localConfirm, setLocalConfirm] = createSignal<LocalConfirmState | null>(null);

export function confirmLocal(opts: { title: string; body: string; confirmLabel: string; cancelLabel?: string; danger?: boolean }): Promise<boolean> {
  return new Promise((resolve) => {
    localConfirm()?.resolve(false);
    setLocalConfirm({
      title: opts.title,
      body: opts.body,
      confirmLabel: opts.confirmLabel,
      cancelLabel: opts.cancelLabel ?? common().actions.cancel,
      danger: opts.danger ?? false,
      resolve,
    });
  });
}

export function LocalConfirmHost() {
  const close = (ok: boolean) => {
    localConfirm()?.resolve(ok);
    setLocalConfirm(null);
  };
  onCleanup(() => close(false));
  return (
    <Show when={localConfirm()}>
      {(state) => (
        <Dialog
          title={state().title}
          onClose={() => close(false)}
          footer={
            <>
              <button type="button" class="btn" onClick={() => close(false)}>
                {state().cancelLabel}
              </button>
              <button type="button" class={`btn ${state().danger ? "danger" : "primary"}`} onClick={() => close(true)}>
                {state().confirmLabel}
              </button>
            </>
          }
        >
          <p style={{ "white-space": "pre-line" }}>{state().body}</p>
        </Dialog>
      )}
    </Show>
  );
}

// ---------------------------------------------------------------------------------------------
// Form state
// ---------------------------------------------------------------------------------------------

export type Errors = Partial<Record<string, string>>;

export interface FormOptions<T, F extends object> {
  /** The value currently stored on the server. */
  source: () => T;
  defaults?: () => T | undefined;
  toForm: (value: T) => F;
  /** Validates the form and builds the value to save (`base` carries fields the form does not edit). */
  parse: (form: F, base: T) => { value: T | null; errors: Errors };
  submit: (value: T) => Promise<unknown>;
  /** Runs after a successful save (typically a refetch). */
  after?: () => unknown;
  /** Maps a server error message or field path to a form key. */
  fieldFor?: (messageOrPath: string) => string | null;
  /** Localised constraint for a key, shown instead of an English server message. */
  describe?: (key: string) => string | undefined;
  /** What dirty tracking compares (defaults to the whole value). */
  compareKey?: (value: T) => unknown;
  /** Asked before saving; resolve false to abort. */
  confirm?: (value: T) => Promise<boolean>;
  savedLabel: () => string;
  editable?: () => boolean;
}

export interface SectionForm<T, F extends object> {
  form: F;
  set: <K extends keyof F & string>(key: K, value: F[K]) => void;
  touch: (key: string) => void;
  parsed: Accessor<{ value: T | null; errors: Errors }>;
  baseline: Accessor<T>;
  dirty: Accessor<boolean>;
  changed: (key: keyof F & string) => boolean;
  error: (key: string) => string | undefined;
  formError: Accessor<string | null>;
  saving: Accessor<boolean>;
  conflict: Accessor<boolean>;
  canEdit: Accessor<boolean>;
  hasDefaults: boolean;
  atDefaults: Accessor<boolean>;
  save: () => Promise<boolean>;
  discard: () => void;
  resetToDefaults: () => void;
  loadLatest: () => void;
}

export function focusFirstInvalid() {
  requestAnimationFrame(() => {
    const el = document.querySelector<HTMLElement>("[aria-invalid='true']");
    if (!el) return;
    el.focus({ preventScroll: true });
    el.scrollIntoView({ block: "center", behavior: "smooth" });
  });
}

export function createSectionForm<T, F extends object>(opts: FormOptions<T, F>): SectionForm<T, F> {
  const t = settingsText;
  const [baseline, setBaseline] = createSignal<T>(opts.source());
  const [form, setForm] = createStore<F>(opts.toForm(opts.source()));
  const [touched, setTouched] = createSignal<ReadonlySet<string>>(new Set());
  const [submitted, setSubmitted] = createSignal(false);
  const [serverErrors, setServerErrors] = createSignal<Errors>({});
  const [formError, setFormError] = createSignal<string | null>(null);
  const [saving, setSaving] = createSignal(false);
  const [conflict, setConflict] = createSignal(false);
  const canEdit = () => (opts.editable ? opts.editable() : isAdmin());

  const key = (value: T) => stable(opts.compareKey ? opts.compareKey(value) : value);
  const parsed = createMemo(() => opts.parse(form, baseline()));
  const baseForm = createMemo(() => opts.toForm(baseline()));
  const dirty = createMemo(() => {
    const p = parsed();
    return !p.value || key(p.value) !== key(baseline());
  });

  const load = (value: T) => {
    setBaseline(() => value);
    setForm(reconcile(opts.toForm(value)));
    setTouched(new Set<string>());
    setSubmitted(false);
    setServerErrors({});
    setFormError(null);
    setConflict(false);
  };

  // Adopt server changes made elsewhere unless the user is editing; then flag the conflict.
  createEffect(
    on(
      () => key(opts.source()),
      (next) => {
        if (next === key(baseline())) {
          setConflict(false);
          return;
        }
        if (!dirty()) load(opts.source());
        else setConflict(true);
      },
      { defer: true },
    ),
  );

  const set = <K extends keyof F & string>(k: K, value: F[K]) => {
    setForm(
      produce((draft: F) => {
        draft[k] = value;
      }),
    );
    if (serverErrors()[k]) setServerErrors((prev) => ({ ...prev, [k]: undefined }));
    if (formError()) setFormError(null);
  };

  const touch = (k: string) => {
    if (touched().has(k)) return;
    setTouched((prev) => new Set([...prev, k]));
  };

  const error = (k: string): string | undefined => {
    const server = serverErrors()[k];
    if (server) return server;
    if (!submitted() && !touched().has(k)) return undefined;
    return parsed().errors[k] || undefined;
  };

  const applyServerError = (e: unknown) => {
    if (e instanceof ApiError && e.code !== "network" && e.status !== 401 && e.status !== 403) {
      const errs: Errors = {};
      const general: string[] = [];
      const known = (k: string | null): k is string => !!k && k in (form as object);
      if (e.fields?.length) {
        for (const f of e.fields) {
          const k = opts.fieldFor?.(f.path) ?? f.path;
          if (known(k)) errs[k] = opts.describe?.(k) ?? f.message;
          else general.push(`${f.path}: ${f.message}`);
        }
      } else {
        const k = opts.fieldFor?.(e.message) ?? null;
        if (known(k)) errs[k] = opts.describe?.(k) ?? e.message;
        else general.push(e.message);
      }
      setServerErrors(errs);
      setFormError(general.length ? general.join("\n") : null);
      if (Object.keys(errs).length) focusFirstInvalid();
    }
    toastError(e, t().form.save_failed);
  };

  const save = async (): Promise<boolean> => {
    if (!canEdit() || saving()) return false;
    setSubmitted(true);
    const p = parsed();
    if (!p.value || Object.values(p.errors).some(Boolean)) {
      toast(t().form.fix_errors, undefined, "warning");
      focusFirstInvalid();
      return false;
    }
    const value = p.value;
    if (opts.confirm && !(await opts.confirm(value))) return false;
    setSaving(true);
    setServerErrors({});
    setFormError(null);
    try {
      await opts.submit(value);
      load(value);
      toast(common().states.saved, tpl(t().form.saved_section, { section: opts.savedLabel() }), "success");
      await opts.after?.();
      return true;
    } catch (e) {
      applyServerError(e);
      return false;
    } finally {
      setSaving(false);
    }
  };

  const discard = () => load(baseline());

  const resetToDefaults = () => {
    const d = opts.defaults?.();
    if (!d) return;
    setForm(reconcile(opts.toForm(d)));
    setServerErrors({});
    setFormError(null);
    setSubmitted(true);
  };

  const atDefaults = createMemo(() => {
    const d = opts.defaults?.();
    const p = parsed();
    return !!d && !!p.value && key(p.value) === key(d);
  });

  // Unsaved edits: confirm in-app navigation and warn before the tab closes.
  useBeforeLeave((e) => {
    if (e.defaultPrevented || !canEdit() || !dirty()) return;
    e.preventDefault();
    void confirmLocal({
      title: t().form.leave_title,
      body: t().form.leave_body,
      confirmLabel: t().form.leave_confirm,
      cancelLabel: t().form.stay,
      danger: true,
    }).then((ok) => {
      if (ok) e.retry(true);
    });
  });
  const onUnload = (event: BeforeUnloadEvent) => {
    if (canEdit() && dirty()) {
      event.preventDefault();
      event.returnValue = "";
    }
  };
  window.addEventListener("beforeunload", onUnload);
  onCleanup(() => window.removeEventListener("beforeunload", onUnload));

  return {
    form,
    set,
    touch,
    parsed,
    baseline,
    dirty,
    changed: (k) => !sameField(form[k], baseForm()[k]),
    error,
    formError,
    saving,
    conflict,
    canEdit,
    hasDefaults: !!opts.defaults,
    atDefaults,
    save,
    discard,
    resetToDefaults,
    loadLatest: () => load(opts.source()),
  };
}

// ---------------------------------------------------------------------------------------------
// Form chrome
// ---------------------------------------------------------------------------------------------

export function FormCard<T, F extends object>(
  props: ParentProps<{ f: SectionForm<T, F>; title: string; sub?: string; saveLabel?: string; class?: string; headExtra?: JSX.Element }>,
) {
  return (
    <form
      class={`card settings-form ${props.class ?? ""}`}
      novalidate
      onSubmit={(e) => {
        e.preventDefault();
        void props.f.save();
      }}
    >
      <FormHead f={props.f} title={props.title} sub={props.sub} extra={props.headExtra} />
      <div class="card-body">
        <FormAlerts f={props.f} />
        {props.children}
      </div>
      <FormFoot f={props.f} saveLabel={props.saveLabel} />
    </form>
  );
}

export function FormHead<T, F extends object>(props: { f?: SectionForm<T, F>; title: string; sub?: string; extra?: JSX.Element }) {
  const t = settingsText;
  return (
    <div class="card-head">
      <div class="head-text">
        <h2>{props.title}</h2>
        <Show when={props.sub}>
          <div class="sub">{props.sub}</div>
        </Show>
      </div>
      <span class="spacer" />
      {props.extra}
      <Show when={props.f && props.f.hasDefaults && props.f.canEdit()}>
        <button
          type="button"
          class="btn ghost sm"
          onClick={() => props.f!.resetToDefaults()}
          disabled={props.f!.atDefaults() || props.f!.saving()}
          title={props.f!.atDefaults() ? t().form.at_defaults : t().form.reset_defaults_hint}
        >
          <SIcon name="history" size={14} />
          {t().form.reset_defaults}
        </button>
      </Show>
    </div>
  );
}

export function FormAlerts<T, F extends object>(props: { f: SectionForm<T, F> }) {
  const t = settingsText;
  return (
    <>
      <Show when={props.f.conflict()}>
        <div class="callout warn settings-alert">
          <SIcon name="alert" size={16} />
          <span class="grow">{t().form.conflict}</span>
          <button type="button" class="btn sm" onClick={() => props.f.loadLatest()}>
            {t().form.load_latest}
          </button>
        </div>
      </Show>
      <Show when={props.f.formError()}>
        <div class="callout critical settings-alert" role="alert">
          <SIcon name="alert" size={16} />
          <span class="grow" style={{ "white-space": "pre-line" }}>
            {t().form.server_rejected} {props.f.formError()}
          </span>
        </div>
      </Show>
    </>
  );
}

export function FormFoot<T, F extends object>(props: { f: SectionForm<T, F>; saveLabel?: string }) {
  const t = settingsText;
  const c = common;
  return (
    <div class="card-foot settings-foot" classList={{ sticky: props.f.canEdit() && props.f.dirty() }}>
      <Show
        when={props.f.canEdit()}
        fallback={
          <span class="foot-status">
            <SIcon name="lock" size={14} />
            {t().form.read_only_short}
          </span>
        }
      >
        <span class="foot-status" classList={{ dirty: props.f.dirty() }} aria-live="polite">
          <Show
            when={props.f.dirty()}
            fallback={
              <>
                <SIcon name="check" size={14} />
                {t().form.no_changes}
              </>
            }
          >
            <span class="dirty-dot" />
            {t().form.unsaved}
          </Show>
        </span>
        <A class="foot-audit" href="/audit" title={t().form.audit_note}>
          <SIcon name="audit" size={13} />
          <span>{t().form.audit_short}</span>
        </A>
        <span class="spacer" />
        <button type="button" class="btn" disabled={!props.f.dirty() || props.f.saving()} onClick={() => props.f.discard()}>
          {t().form.discard}
        </button>
        <button type="submit" class="btn primary" disabled={!props.f.dirty() || props.f.saving()}>
          {props.f.saving() ? c().actions.saving : props.saveLabel ?? c().actions.save}
        </button>
      </Show>
    </div>
  );
}

/** Label + control + hint/error, with a dot when the value differs from what is saved. */
export function Field(
  props: ParentProps<{
    id?: string;
    label: JSX.Element;
    hint?: JSX.Element;
    error?: string;
    changed?: boolean;
    class?: string;
    aside?: JSX.Element;
  }>,
) {
  return (
    <div class={`field ${props.class ?? ""}`} classList={{ "is-invalid": !!props.error }}>
      <div class="field-top">
        <label for={props.id}>{props.label}</label>
        <Show when={props.changed}>
          <span class="changed-dot" title={settingsText().form.changed} />
        </Show>
        <Show when={props.aside}>
          <span class="field-aside">{props.aside}</span>
        </Show>
      </div>
      {props.children}
      <Show
        when={props.error}
        fallback={
          <Show when={props.hint}>
            <div class="hint">{props.hint}</div>
          </Show>
        }
      >
        <div class="error-text" role="alert">
          {props.error}
        </div>
      </Show>
    </div>
  );
}

export function NumInput(props: {
  id: string;
  value: string;
  onInput: (value: string) => void;
  onBlur?: () => void;
  unit?: string;
  prefix?: string;
  invalid?: boolean;
  disabled?: boolean;
  int?: boolean;
  placeholder?: string;
  wide?: boolean;
}) {
  return (
    <div class="input-group num-group" classList={{ "has-prefix": !!props.prefix, "no-unit": !props.unit, wide: !!props.wide }}>
      <Show when={props.prefix}>
        <span class="addon pre">{props.prefix}</span>
      </Show>
      <input
        id={props.id}
        class="input num"
        classList={{ invalid: !!props.invalid }}
        inputmode={props.int ? "numeric" : "decimal"}
        autocomplete="off"
        spellcheck={false}
        placeholder={props.placeholder}
        value={props.value}
        disabled={props.disabled}
        aria-invalid={props.invalid ? "true" : "false"}
        onInput={(e) => props.onInput(e.currentTarget.value)}
        onBlur={() => props.onBlur?.()}
      />
      <Show when={props.unit}>
        <span class="addon">{props.unit}</span>
      </Show>
    </div>
  );
}

/** Small "default: …" suffix appended to a hint. */
export function withDefault(hint: string, defaultText: string | null | undefined): string {
  if (!defaultText) return hint;
  const sep = !hint || /[。！？；）]$/.test(hint) ? "" : " ";
  return `${hint}${sep}${tpl(settingsText().form.default_value, { value: defaultText })}`;
}

export function GroupTitle(props: ParentProps<{ sub?: string }>) {
  return (
    <div class="group-title">
      <h3>{props.children}</h3>
      <Show when={props.sub}>
        <p>{props.sub}</p>
      </Show>
    </div>
  );
}

/** Shown instead of local account forms when operators sign in through hone-claw.com. */
export function HoneclawAccountsNote() {
  const t = settingsText;
  return (
    <div class="card">
      <div class="card-body">
        <div class="callout info">
          <Icon name="shield" size={16} />
          <div>
            <strong>{t().users.honeclaw_title}</strong>
            <p style={{ margin: "4px 0 8px" }}>{t().users.honeclaw_body}</p>
            <a class="btn sm" href={meta()?.auth?.login_url ?? "https://hone-claw.com/"}>
              {t().users.honeclaw_link}
            </a>
          </div>
        </div>
      </div>
    </div>
  );
}
