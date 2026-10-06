import { For, Show, createMemo, createSignal } from "solid-js";
import { toast, toastError } from "@/components/ui";
import { tpl } from "@/i18n";
import { common } from "@/i18n/common";
import { settingsText } from "@/i18n/settings";
import { ApiError, api } from "@/lib/api";
import { honeclawAuth, me } from "@/lib/session";
import { type Errors, Field, HoneclawAccountsNote, SIcon, focusFirstInvalid } from "./shared";

const MIN_LENGTH = 10;

/** Mirrors `auth::check_password_strength`: at least 10 characters and not only digits. */
export function passwordProblems(password: string): string[] {
  const t = settingsText().security;
  const out: string[] = [];
  if ([...password].length < MIN_LENGTH) out.push(t.rule_length);
  if (password && /^[0-9]+$/.test(password)) out.push(t.rule_digits);
  return out;
}

function strength(password: string): 0 | 1 | 2 | 3 {
  if (!password) return 0;
  if (passwordProblems(password).length) return 1;
  const classes = [/[a-z]/, /[A-Z]/, /\d/, /[^A-Za-z0-9]/].filter((r) => r.test(password)).length;
  const length = [...password].length;
  return length >= 16 || (length >= 12 && classes >= 3) ? 3 : 2;
}

export function PasswordInput(props: {
  id: string;
  value: string;
  onInput: (value: string) => void;
  onBlur?: () => void;
  reveal?: boolean;
  onReveal?: (value: boolean) => void;
  invalid?: boolean;
  autocomplete?: string;
}) {
  const t = settingsText;
  const [local, setLocal] = createSignal(false);
  const shown = () => props.reveal ?? local();
  const toggle = () => (props.onReveal ? props.onReveal(!shown()) : setLocal(!shown()));
  return (
    <div class="secret-box">
      <input
        id={props.id}
        class="input"
        classList={{ invalid: !!props.invalid, mono: shown() }}
        aria-invalid={props.invalid ? "true" : "false"}
        type={shown() ? "text" : "password"}
        autocomplete={props.autocomplete ?? "off"}
        spellcheck={false}
        value={props.value}
        onInput={(e) => props.onInput(e.currentTarget.value)}
        onBlur={() => props.onBlur?.()}
      />
      <button
        type="button"
        class="btn ghost icon sm reveal"
        onClick={toggle}
        aria-label={shown() ? t().form.hide : t().form.show}
        title={shown() ? t().form.hide : t().form.show}
      >
        <SIcon name={shown() ? "eye_off" : "eye"} size={15} />
      </button>
    </div>
  );
}

export function StrengthMeter(props: { password: string; current?: string }) {
  const t = settingsText;
  const level = () => strength(props.password);
  const rules = createMemo(() => {
    const s = t().security;
    const pw = props.password;
    const list = [
      { ok: [...pw].length >= MIN_LENGTH, text: s.rule_length },
      { ok: !!pw && !/^[0-9]+$/.test(pw), text: s.rule_digits },
    ];
    if (props.current !== undefined) list.push({ ok: !!pw && pw !== props.current, text: s.rule_differs });
    return list;
  });
  const label = () => {
    const s = t().security;
    return level() === 3 ? s.strong : level() === 2 ? s.fair : s.weak;
  };
  return (
    <div class="strength">
      <div class="strength-bar" data-level={level()}>
        <span />
        <span />
        <span />
      </div>
      <Show when={props.password}>
        <span class="xs muted">{tpl(t().security.strength, { level: label() })}</span>
      </Show>
      <ul class="rules">
        <For each={rules()}>
          {(r) => (
            <li classList={{ ok: r.ok }}>
              <SIcon name={r.ok ? "check" : "x"} size={12} />
              {r.text}
            </li>
          )}
        </For>
      </ul>
    </div>
  );
}

export default function SecuritySection() {
  return (
    <Show when={!honeclawAuth()} fallback={<HoneclawAccountsNote />}>
      <PasswordSection />
    </Show>
  );
}

function PasswordSection() {
  const t = settingsText;
  const c = common;
  const [current, setCurrent] = createSignal("");
  const [next, setNext] = createSignal("");
  const [confirm, setConfirm] = createSignal("");
  const [submitted, setSubmitted] = createSignal(false);
  const [serverErrors, setServerErrors] = createSignal<Errors>({});
  const [busy, setBusy] = createSignal(false);

  const errors = createMemo<Errors>(() => {
    const e: Errors = {};
    const v = settingsText().v;
    if (!current()) e.current = v.required;
    const problems = passwordProblems(next());
    if (!next()) e.next = v.required;
    else if (problems.length) e.next = problems[0];
    else if (next() === current()) e.next = t().security.rule_differs;
    if (confirm() !== next()) e.confirm = t().security.mismatch;
    return e;
  });
  const err = (k: string) => serverErrors()[k] ?? (submitted() ? errors()[k] : undefined);
  const clear = (k: string) => setServerErrors((prev) => ({ ...prev, [k]: undefined }));
  const dirty = () => !!(current() || next() || confirm());

  const submit = async () => {
    setSubmitted(true);
    if (Object.values(errors()).some(Boolean)) {
      focusFirstInvalid();
      return;
    }
    setBusy(true);
    try {
      await api.changePassword(current(), next());
      toast(t().security.done, t().security.done_body, "success");
      setCurrent("");
      setNext("");
      setConfirm("");
      setSubmitted(false);
      setServerErrors({});
    } catch (error) {
      if (error instanceof ApiError && error.status === 403 && /current password/i.test(error.message)) {
        setServerErrors({ current: t().security.wrong_current });
        focusFirstInvalid();
        toast(t().security.wrong_current, undefined, "critical");
      } else {
        if (error instanceof ApiError && error.status === 400) {
          setServerErrors({ next: error.message });
          focusFirstInvalid();
        }
        toastError(error);
      }
    } finally {
      setBusy(false);
    }
  };

  const reset = () => {
    setCurrent("");
    setNext("");
    setConfirm("");
    setSubmitted(false);
    setServerErrors({});
  };

  return (
    <form
      class="card settings-form"
      novalidate
      onSubmit={(e) => {
        e.preventDefault();
        void submit();
      }}
    >
      <div class="card-head">
        <div class="head-text">
          <h2>{t().security.title}</h2>
          <div class="sub">
            {tpl(t().security.sub, { user: me()?.username ?? "" })}
            {" · "}
            {me()?.role === "admin" ? t().users.role_admin : t().users.role_viewer}
          </div>
        </div>
      </div>
      <div class="card-body">
        <div class="stack password-form" style={{ gap: "16px" }}>
          {/* Lets password managers associate the form with the account. */}
          <input type="text" name="username" autocomplete="username" value={me()?.username ?? ""} hidden readOnly />
          <Field id="pw-current" label={t().security.current} error={err("current")}>
            <PasswordInput
              id="pw-current"
              value={current()}
              autocomplete="current-password"
              invalid={!!err("current")}
              onInput={(v) => {
                setCurrent(v);
                clear("current");
              }}
            />
          </Field>
          <Field id="pw-next" label={t().security.next} error={err("next")}>
            <PasswordInput
              id="pw-next"
              value={next()}
              autocomplete="new-password"
              invalid={!!err("next")}
              onInput={(v) => {
                setNext(v);
                clear("next");
              }}
            />
          </Field>
          <StrengthMeter password={next()} current={current()} />
          <Field id="pw-confirm" label={t().security.confirm} error={err("confirm")}>
            <PasswordInput id="pw-confirm" value={confirm()} autocomplete="new-password" invalid={!!err("confirm")} onInput={setConfirm} />
          </Field>
          <div class="callout info">
            <SIcon name="info" size={16} />
            <span>{t().security.sessions_note}</span>
          </div>
        </div>
      </div>
      <div class="card-foot settings-foot">
        <span class="foot-status">
          <SIcon name="audit" size={13} />
          {settingsText().form.audit_change}
        </span>
        <span class="spacer" />
        <button type="button" class="btn" disabled={!dirty() || busy()} onClick={reset}>
          {c().actions.cancel}
        </button>
        <button type="submit" class="btn primary" disabled={!dirty() || busy()}>
          {busy() ? c().actions.saving : t().security.submit}
        </button>
      </div>
    </form>
  );
}
