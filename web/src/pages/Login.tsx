import { useNavigate, useSearchParams } from "@solidjs/router";
import { Show, createSignal, onMount } from "solid-js";
import { Icon } from "@/components/Icon";
import { Segmented } from "@/components/ui";
import { locale, setLocale } from "@/i18n";
import { common } from "@/i18n/common";
import { loginText } from "@/i18n/login";
import { ApiError, api } from "@/lib/api";
import { withBase } from "@/lib/base";
import { authProblem, honeclawAuth, loadMe, me, meta } from "@/lib/session";

export default function Login() {
  const t = loginText;
  const navigate = useNavigate();
  const [params] = useSearchParams();
  const [username, setUsername] = createSignal("");
  const [password, setPassword] = createSignal("");
  const [busy, setBusy] = createSignal(false);
  const [error, setError] = createSignal<string | null>(null);

  const target = () => {
    const next = typeof params.next === "string" ? params.next : "/";
    // Only same-app paths: never follow an absolute or protocol-relative URL.
    return next.startsWith("/") && !next.startsWith("//") ? next : "/";
  };

  onMount(() => {
    if (me()) navigate(target(), { replace: true });
  });

  // honeclaw mode: the session lives on hone-claw.com; re-check it after signing in there.
  const [checked, setChecked] = createSignal(false);
  const recheck = async () => {
    if (busy()) return;
    setBusy(true);
    try {
      const user = await loadMe();
      setChecked(true);
      if (user) navigate(target(), { replace: true });
    } finally {
      setBusy(false);
    }
  };
  const honeclawMessage = () =>
    authProblem() === "not_admin" ? t().hc_not_admin : authProblem() === "unavailable" ? t().hc_unavailable : checked() ? t().hc_signed_out : null;

  const submit = async (event: Event) => {
    event.preventDefault();
    if (busy()) return;
    setBusy(true);
    setError(null);
    try {
      await api.login(username().trim(), password());
      await loadMe();
      navigate(target(), { replace: true });
    } catch (e) {
      if (e instanceof ApiError) {
        if (e.status === 429) setError(t().rate_limited);
        else if (e.status === 403 || e.status === 401) setError(t().invalid);
        else if (e.code === "network") setError(common().states.network);
        else setError(e.message);
      } else {
        setError(String(e));
      }
      setPassword("");
    } finally {
      setBusy(false);
    }
  };

  return (
    <div class="login-page">
      <div class="login-top">
        <Segmented
          value={locale()}
          onChange={(v) => setLocale(v)}
          options={[
            { value: "zh", label: "中文" },
            { value: "en", label: "English" },
          ]}
          label={common().prefs.language}
        />
      </div>
      <main class="login-card">
        <div class="login-brand">
          <img src={withBase("/hone-mark.svg")} alt="" width="40" height="40" />
          <div class="word">
            <b>HONE</b>
            <span>QUANT</span>
          </div>
        </div>
        <h1>{t().title}</h1>
        <p class="muted small">{t().subtitle}</p>

        <Show when={meta()?.demo}>
          <div class="callout warn" style={{ "margin-top": "16px" }}>
            <Icon name="alert" size={16} />
            <span>{common().app.demo_banner}</span>
          </div>
        </Show>

        <Show when={honeclawAuth()}>
          <div class="stack" style={{ gap: "14px", "margin-top": "22px" }}>
            <div class="callout info">
              <Icon name="shield" size={16} />
              <div>
                <strong>{t().hc_title}</strong>
                <p style={{ margin: "4px 0 0" }}>{t().hc_body}</p>
              </div>
            </div>
            <Show when={honeclawMessage()}>
              <div class={`callout ${authProblem() === "unavailable" ? "warn" : "critical"}`} role="alert">
                <Icon name="alert" size={16} />
                <span>{honeclawMessage()}</span>
              </div>
            </Show>
            <a class="btn primary" href={meta()?.auth?.login_url ?? "https://hone-claw.com/"} style={{ "min-height": "40px", "justify-content": "center" }}>
              {t().hc_sign_in}
            </a>
            <button class="btn" type="button" onClick={recheck} disabled={busy()}>
              {busy() ? t().hc_checking : t().hc_retry}
            </button>
          </div>
        </Show>

        <Show
          when={!honeclawAuth() && meta()?.has_users !== false}
          fallback={
            <Show when={!honeclawAuth()}>
              <div class="callout info" style={{ "margin-top": "18px" }}>
                <Icon name="info" size={16} />
                <div>
                  <strong>{t().no_users_title}</strong>
                  <p style={{ margin: "4px 0 6px" }}>{t().no_users_body}</p>
                  <code class="mono xs">hone-quant user add admin --role admin</code>
                </div>
              </div>
            </Show>
          }
        >
          <form class="stack" style={{ gap: "14px", "margin-top": "22px" }} onSubmit={submit} novalidate>
            <div class="field">
              <label for="login-user">{t().username}</label>
              <input
                id="login-user"
                class="input"
                autocomplete="username"
                autocapitalize="off"
                spellcheck={false}
                required
                value={username()}
                onInput={(e) => setUsername(e.currentTarget.value)}
              />
            </div>
            <div class="field">
              <label for="login-pass">{t().password}</label>
              <input
                id="login-pass"
                class="input"
                type="password"
                autocomplete="current-password"
                required
                value={password()}
                onInput={(e) => setPassword(e.currentTarget.value)}
              />
            </div>
            <Show when={error()}>
              <div class="callout critical" role="alert">
                <Icon name="alert" size={16} />
                <span>{error()}</span>
              </div>
            </Show>
            <button class="btn primary" type="submit" disabled={busy() || !username().trim() || !password()} style={{ "min-height": "40px" }}>
              {busy() ? t().submitting : t().submit}
            </button>
          </form>
        </Show>

        <div class="login-paper">
          <Icon name="shield" size={14} />
          {t().paper_note}
        </div>
      </main>
      <p class="muted xs login-foot">
        {t().footer}
        <Show when={meta()}> · v{meta()!.version}</Show>
      </p>
    </div>
  );
}
