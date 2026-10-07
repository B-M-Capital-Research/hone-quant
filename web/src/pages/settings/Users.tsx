import { For, Show, createMemo, createSignal } from "solid-js";
import { Dialog, Empty, confirmAction, toast, toastError } from "@/components/ui";
import { tpl } from "@/i18n";
import { common } from "@/i18n/common";
import { settingsText } from "@/i18n/settings";
import { ApiError, api } from "@/lib/api";
import { fmtDateTime, fmtRelative } from "@/lib/format";
import { honeclawAuth, isAdmin, me } from "@/lib/session";
import type { Role, User } from "@/lib/types";
import { type Errors, Field, Gate, HoneclawAccountsNote, SIcon, createLoader, focusFirstInvalid } from "./shared";
import { PasswordInput, passwordProblems } from "./Security";

const ROLES: Role[] = ["viewer", "member", "admin"];
const ROLE_TONE: Record<Role, string> = { admin: "orange", member: "blue", viewer: "" };

export default function UsersSection() {
  const t = settingsText;
  return (
    <Show when={!honeclawAuth()} fallback={<HoneclawAccountsNote />}>
      <Show
        when={isAdmin()}
        fallback={
          <div class="card">
            <Empty title={t().users.admin_only} icon="lock" />
          </div>
        }
      >
        <UsersCard />
      </Show>
    </Show>
  );
}

function UsersCard() {
  const t = settingsText;
  const c = common;
  const users = createLoader(() => api.users());
  const [adding, setAdding] = createSignal(false);
  const [busy, setBusy] = createSignal<number | null>(null);

  const isSelf = (u: User) => (me()?.id !== undefined && u.id === me()?.id) || u.username === me()?.username;

  const remove = async (u: User) => {
    if (u.id === undefined) return;
    const ok = await confirmAction({
      title: t().users.delete_title,
      body: tpl(t().users.delete_body, { name: u.username }),
      confirmLabel: c().actions.delete,
      danger: true,
    });
    if (ok === null) return;
    setBusy(u.id);
    try {
      await api.deleteUser(u.id);
      toast(tpl(t().users.deleted, { name: u.username }), undefined, "success");
    } catch (error) {
      toastError(error);
    } finally {
      setBusy(null);
      await users.reload();
    }
  };

  return (
    <>
      <div class="card">
        <div class="card-head">
          <div class="head-text">
            <h2>{t().users.title}</h2>
            <div class="sub">{tpl(t().users.sub, { count: users.value()?.users.length ?? "…" })}</div>
          </div>
          <span class="spacer" />
          <button type="button" class="btn sm" onClick={() => setAdding(true)}>
            <SIcon name="plus" size={14} />
            {t().users.add}
          </button>
        </div>
        <Gate loader={users}>
          {(data) => (
            <div class="table-wrap">
              <table class="table users-table">
                <thead>
                  <tr>
                    <th>{t().users.col_user}</th>
                    <th>{t().users.col_role}</th>
                    <th class="col-created">{t().users.col_created}</th>
                    <th>{t().users.col_login}</th>
                    <th class="r" />
                  </tr>
                </thead>
                <tbody>
                  <For each={data().users}>
                    {(u) => (
                      <tr>
                        <td>
                          <span class="user-cell">
                            <span class="avatar" aria-hidden="true">
                              {u.username.slice(0, 1).toUpperCase()}
                            </span>
                            <b>{u.username}</b>
                            <Show when={isSelf(u)}>
                              <span class="chip outline">{t().users.you}</span>
                            </Show>
                          </span>
                        </td>
                        <td>
                          <span class={`chip ${ROLE_TONE[u.role] ?? ""}`}>{c().roles[u.role] ?? u.role}</span>
                        </td>
                        <td class="num small nowrap col-created">{fmtDateTime(u.created_at)}</td>
                        <td class="small nowrap" title={u.last_login_at ? fmtDateTime(u.last_login_at) : undefined}>
                          {u.last_login_at ? fmtRelative(u.last_login_at) : <span class="muted">{t().users.never}</span>}
                        </td>
                        <td class="r">
                          <button
                            type="button"
                            class="btn ghost icon sm danger-ghost"
                            disabled={isSelf(u) || busy() === u.id}
                            title={isSelf(u) ? t().users.cannot_delete_self : c().actions.delete}
                            aria-label={`${c().actions.delete} ${u.username}`}
                            onClick={() => void remove(u)}
                          >
                            <SIcon name="trash" size={15} />
                          </button>
                        </td>
                      </tr>
                    )}
                  </For>
                </tbody>
              </table>
            </div>
          )}
        </Gate>
      </div>
      <div class="card">
        <div class="card-body roles-explainer">
          <For each={[...ROLES].reverse()}>
            {(r) => (
              <div class="role-line">
                <span class={`chip ${ROLE_TONE[r]}`}>{c().roles[r]}</span>
                <span class="small subtle">{roleBody(r)}</span>
              </div>
            )}
          </For>
        </div>
      </div>
      <Show when={adding()}>
        <AddUserDialog
          existing={users.value()?.users.map((u) => u.username) ?? []}
          onClose={() => setAdding(false)}
          onCreated={() => void users.reload()}
        />
      </Show>
    </>
  );
}

/** One line on what a role may do. */
function roleBody(role: Role): string {
  const u = settingsText().users;
  return role === "admin" ? u.role_admin_body : role === "member" ? u.role_member_body : u.role_viewer_body;
}

const USERNAME = /^[\p{L}\p{N}._-]+$/u;

function randomPassword(): string {
  const alphabet = "ABCDEFGHJKLMNPQRSTUVWXYZabcdefghijkmnopqrstuvwxyz23456789-_!@#%";
  const bytes = new Uint32Array(18);
  crypto.getRandomValues(bytes);
  return Array.from(bytes, (n) => alphabet[n % alphabet.length]).join("");
}

function AddUserDialog(props: { existing: string[]; onClose: () => void; onCreated: () => void }) {
  const t = settingsText;
  const c = common;
  const [username, setUsername] = createSignal("");
  const [role, setRole] = createSignal<Role>("viewer");
  const [password, setPassword] = createSignal("");
  const [password2, setPassword2] = createSignal("");
  const [reveal, setReveal] = createSignal(false);
  const [submitted, setSubmitted] = createSignal(false);
  const [touched, setTouched] = createSignal<Record<string, boolean>>({});
  const [serverErrors, setServerErrors] = createSignal<Errors>({});
  const [busy, setBusy] = createSignal(false);

  const errors = createMemo<Errors>(() => {
    const e: Errors = {};
    const u = username().trim();
    if (!u) e.username = settingsText().v.required;
    else if (new TextEncoder().encode(u).length > 64 || !USERNAME.test(u)) e.username = t().users.username_invalid;
    else if (props.existing.includes(u)) e.username = t().users.username_taken;
    const problems = passwordProblems(password());
    if (!password()) e.password = settingsText().v.required;
    else if (problems.length) e.password = problems[0];
    if (password2() !== password()) e.password2 = t().users.mismatch;
    return e;
  });
  const err = (k: string) => serverErrors()[k] ?? (submitted() || touched()[k] ? errors()[k] : undefined);
  const touch = (k: string) => setTouched((prev) => ({ ...prev, [k]: true }));
  const clearServer = (k: string) => setServerErrors((prev) => ({ ...prev, [k]: undefined }));

  const generate = async () => {
    const pw = randomPassword();
    setPassword(pw);
    setPassword2(pw);
    setReveal(true);
    clearServer("password");
    try {
      await navigator.clipboard.writeText(pw);
      toast(t().users.copied, undefined, "success");
    } catch {
      /* clipboard unavailable: the password stays visible */
    }
  };

  const submit = async () => {
    setSubmitted(true);
    if (Object.values(errors()).some(Boolean)) {
      focusFirstInvalid();
      return;
    }
    setBusy(true);
    try {
      const name = username().trim();
      await api.createUser(name, password(), role());
      toast(tpl(t().users.created, { name }), undefined, "success");
      props.onClose();
      props.onCreated();
    } catch (error) {
      if (error instanceof ApiError && (error.status === 400 || error.status === 409)) {
        const field = /username/i.test(error.message) ? "username" : /password/i.test(error.message) ? "password" : null;
        if (field) {
          setServerErrors({ [field]: error.status === 409 ? t().users.username_taken : error.message });
          focusFirstInvalid();
        }
      }
      toastError(error);
    } finally {
      setBusy(false);
    }
  };

  return (
    <Dialog
      title={t().users.dialog_title}
      onClose={() => !busy() && props.onClose()}
      footer={
        <>
          <button type="button" class="btn" onClick={() => props.onClose()} disabled={busy()}>
            {c().actions.cancel}
          </button>
          <button type="button" class="btn primary" onClick={() => void submit()} disabled={busy()}>
            {busy() ? c().actions.saving : t().users.create}
          </button>
        </>
      }
    >
      <form
        class="stack"
        style={{ gap: "16px" }}
        novalidate
        autocomplete="off"
        onSubmit={(e) => {
          e.preventDefault();
          void submit();
        }}
      >
        <Field id="user-name" label={t().users.username} hint={t().users.username_hint} error={err("username")}>
          <input
            id="user-name"
            class="input"
            classList={{ invalid: !!err("username") }}
            aria-invalid={err("username") ? "true" : "false"}
            autocomplete="off"
            autocapitalize="off"
            spellcheck={false}
            value={username()}
            onInput={(e) => {
              setUsername(e.currentTarget.value);
              clearServer("username");
            }}
            onBlur={() => touch("username")}
          />
        </Field>
        <div class="field">
          <span class="field-label">{t().users.role}</span>
          <div class="role-cards" role="radiogroup" aria-label={t().users.role}>
            <For each={ROLES}>
              {(r) => (
                <label class="role-card" classList={{ selected: role() === r }}>
                  <input type="radio" name="new-user-role" value={r} checked={role() === r} onChange={() => setRole(r)} />
                  <span>
                    <b>{c().roles[r]}</b>
                    <span class="muted xs">{roleBody(r)}</span>
                  </span>
                </label>
              )}
            </For>
          </div>
        </div>
        <Field
          id="user-pass"
          label={t().users.password}
          hint={t().users.password_hint}
          error={err("password")}
          aside={
            <button type="button" class="link-btn" onClick={() => void generate()}>
              <SIcon name="key" size={13} />
              {t().users.generate}
            </button>
          }
        >
          <PasswordInput
            id="user-pass"
            value={password()}
            reveal={reveal()}
            onReveal={setReveal}
            invalid={!!err("password")}
            autocomplete="new-password"
            onInput={(v) => {
              setPassword(v);
              clearServer("password");
            }}
            onBlur={() => touch("password")}
          />
        </Field>
        <Field id="user-pass2" label={t().users.password2} error={err("password2")}>
          <PasswordInput
            id="user-pass2"
            value={password2()}
            reveal={reveal()}
            onReveal={setReveal}
            invalid={!!err("password2")}
            autocomplete="new-password"
            onInput={setPassword2}
            onBlur={() => touch("password2")}
          />
        </Field>
        <button type="submit" hidden />
      </form>
    </Dialog>
  );
}
