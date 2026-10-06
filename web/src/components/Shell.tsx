import { A, useLocation, useNavigate } from "@solidjs/router";
import { For, type ParentProps, Show, createMemo, createSignal, onCleanup, onMount } from "solid-js";
import { locale, setLocale, tpl } from "@/i18n";
import { common } from "@/i18n/common";
import { shellText } from "@/i18n/shell";
import { api } from "@/lib/api";
import { withBase } from "@/lib/base";
import { connectEvents, connected, onServerEvent } from "@/lib/events";
import { fmtCountdown, fmtDual, fmtTime, MARKET_TZ, zoneLabel } from "@/lib/format";
import { displayTz, setThemePref, setUpDown, themePref, upDown, type ThemePref, type UpDown } from "@/lib/prefs";
import { isAdmin, market, me, meta, refreshMarket, refreshUnread, serverNow, signedOut, startClock, unread } from "@/lib/session";
import type { AutomationMode } from "@/lib/types";
import { Icon, type IconName } from "./Icon";
import { ConfirmHost, Popover, Segmented, Toasts, confirmAction, toast, toastError } from "./ui";

interface NavItem {
  href: string;
  icon: IconName;
  label: () => string;
  badge?: () => number;
}

function navGroups(): { label: () => string; items: NavItem[] }[] {
  const c = common;
  return [
    {
      label: () => c().nav.trading,
      items: [
        { href: "/", icon: "overview", label: () => c().nav.overview },
        { href: "/plans", icon: "plans", label: () => c().nav.plans },
        { href: "/trades", icon: "trades", label: () => c().nav.trades },
      ],
    },
    {
      label: () => c().nav.strategy_group,
      items: [
        { href: "/strategy", icon: "sliders", label: () => c().nav.strategy },
        { href: "/universe", icon: "universe", label: () => c().nav.universe },
      ],
    },
    {
      label: () => c().nav.research,
      items: [
        { href: "/backtests", icon: "backtest", label: () => c().nav.backtests },
        { href: "/performance", icon: "performance", label: () => c().nav.performance },
      ],
    },
    {
      label: () => c().nav.system,
      items: [
        { href: "/notifications", icon: "bell", label: () => c().nav.notifications, badge: unread },
        { href: "/audit", icon: "audit", label: () => c().nav.audit },
        { href: "/settings", icon: "settings", label: () => c().nav.settings },
      ],
    },
  ];
}

function isActive(path: string, href: string) {
  return href === "/" ? path === "/" : path === href || path.startsWith(`${href}/`);
}

/** The most relevant upcoming event for the top bar. */
function useNextEvent() {
  return createMemo(() => {
    const m = market();
    if (!m) return null;
    const now = serverNow();
    const s = shellText();
    const c = common();
    for (const slot of m.schedule) {
      const plan = slot.plan;
      const slotName = c.slot[slot.slot];
      if (plan?.status === "pending") {
        if (plan.execute_after && new Date(plan.execute_after).getTime() > now) {
          return { label: tpl(s.next.executes_in, { slot: slotName }), at: plan.execute_after, tone: "orange" };
        }
        return { label: tpl(s.next.approval_due, { slot: slotName }), at: plan.deadline, tone: "orange" };
      }
      if (!plan && !slot.cancelled && new Date(slot.generate_at).getTime() > now && m.effective_mode !== "paused") {
        return { label: tpl(s.next.plan_in, { slot: slotName }), at: slot.generate_at, tone: "" };
      }
    }
    if (m.phase === "open" && m.today) return { label: s.next.closes_in, at: m.today.close, tone: "" };
    return { label: s.next.opens_in, at: m.next_session.open, tone: "" };
  });
}

function MarketPill() {
  const c = common;
  const phaseClass = () => {
    const p = market()?.phase;
    return p === "open" ? "open" : p === "pre_open" ? "pre" : "";
  };
  const next = useNextEvent();
  return (
    <div class="market-pill" title={market()?.holiday ? tpl(shellText().next.holiday, { name: locale() === "zh" ? market()!.holiday!.name_zh : market()!.holiday!.name_en }) : undefined}>
      <span class={`phase ${phaseClass()}`}>
        <span class="dot" />
        {market() ? c().phase[market()!.phase] : "…"}
      </span>
      <span class="sep" />
      <span class="clock">
        {fmtTime(serverNow(), MARKET_TZ)} ET
        <Show when={displayTz() !== MARKET_TZ}>
          <span class="muted"> · {fmtTime(serverNow(), displayTz())} {zoneLabel(displayTz())}</span>
        </Show>
      </span>
      <Show when={next()}>
        {(n) => (
          <>
            <span class="sep hide-md" />
            <span class="hide-md" title={fmtDual(n().at, true)}>
              <span class="muted">{n().label} </span>
              <b class="num">{fmtCountdown(new Date(n().at).getTime() - serverNow())}</b>
            </span>
          </>
        )}
      </Show>
    </div>
  );
}

function AutomationControl() {
  const c = common;
  const s = shellText;
  const mode = () => market()?.effective_mode ?? "auto";
  const tone = () => (mode() === "auto" ? "green" : mode() === "approval" ? "blue" : "yellow");
  const change = async (next: AutomationMode, pausedUntil: string | null = null) => {
    const body = next === "auto" ? s().automation.confirm_auto : next === "approval" ? s().automation.confirm_approval : s().automation.confirm_paused;
    const note = await confirmAction({ title: s().automation.change_title, body, askReason: true, reasonLabel: c().words.note });
    if (note === null) return;
    try {
      await api.setAutomation({ mode: next === "paused" && pausedUntil ? market()?.automation.mode ?? "auto" : next, paused_until: pausedUntil, note });
      toast(c().states.saved, `${s().automation.label}: ${c().mode[next]}`, "success");
      refreshMarket();
    } catch (error) {
      toastError(error);
    }
  };
  /** Skips the rest of the current (or next) session; automation resumes after its close. */
  const pauseSession = () => {
    const m = market();
    if (m) change("paused", m.next_session.close);
  };
  return (
    <Popover
      width={300}
      trigger={(toggle) => (
        <button class={`chip ${tone()}`} style={{ height: "30px", padding: "0 12px", cursor: "pointer", border: 0 }} onClick={toggle} disabled={!isAdmin()} title={c().mode[`${mode()}_hint` as const]}>
          <Icon name={mode() === "paused" ? "pause" : "zap"} size={13} />
          <span class="hide-sm">{c().mode[mode()]}</span>
        </button>
      )}
    >
      <div class="stack" style={{ gap: "10px", padding: "8px" }}>
        <div class="kicker">{s().automation.label}</div>
        <Segmented
          value={market()?.automation.mode ?? "auto"}
          onChange={(v) => change(v as AutomationMode)}
          options={(["auto", "approval", "paused"] as AutomationMode[]).map((m) => ({ value: m, label: c().mode[m], title: c().mode[`${m}_hint` as const] }))}
        />
        <p class="muted xs">{c().mode[`${mode()}_hint` as const]}</p>
        <Show when={market()?.automation.paused_until}>
          <p class="xs">{tpl(s().automation.pause_until, { time: fmtDual(market()!.automation.paused_until, true) })}</p>
        </Show>
        <div class="menu-sep" />
        <button class="menu-item" data-close onClick={pauseSession} disabled={!market()}>
          <Icon name="pause" size={15} /> {tpl(s().automation.pause_session, { date: market()?.next_session.date ?? "" })}
        </button>
      </div>
    </Popover>
  );
}

function PrefsMenu() {
  const c = common;
  return (
    <Popover
      width={300}
      trigger={(toggle) => (
        <button class="btn ghost icon" onClick={toggle} aria-label={c().prefs.title} title={c().prefs.title}>
          <Icon name="globe" />
        </button>
      )}
    >
      <div class="stack" style={{ gap: "12px", padding: "8px" }}>
        <div class="field">
          <span class="field-label">{c().prefs.language}</span>
          <Segmented value={locale()} onChange={(v) => setLocale(v)} options={[{ value: "zh", label: "中文" }, { value: "en", label: "English" }]} />
        </div>
        <div class="field">
          <span class="field-label">{c().prefs.theme}</span>
          <Segmented
            value={themePref()}
            onChange={(v) => setThemePref(v as ThemePref)}
            options={[
              { value: "auto", label: c().prefs.theme_auto },
              { value: "light", label: c().prefs.theme_light },
              { value: "dark", label: c().prefs.theme_dark },
            ]}
          />
        </div>
        <div class="field">
          <label class="field-label" for="pref-updown">{c().prefs.updown}</label>
          <select id="pref-updown" class="select" value={upDown()} onChange={(e) => setUpDown(e.currentTarget.value as UpDown)}>
            <option value="green-up">{c().prefs.green_up}</option>
            <option value="red-up">{c().prefs.red_up}</option>
            <option value="blue-orange">{c().prefs.blue_orange}</option>
          </select>
        </div>
      </div>
    </Popover>
  );
}

function UserMenu() {
  const c = common;
  const navigate = useNavigate();
  const logout = async () => {
    try {
      await api.logout();
    } catch {
      /* already signed out */
    }
    signedOut();
    navigate("/login", { replace: true });
  };
  return (
    <Popover
      width={240}
      trigger={(toggle) => (
        <button class="btn ghost icon" onClick={toggle} aria-label={me()?.username}>
          <Icon name="user" />
        </button>
      )}
    >
      <div style={{ padding: "8px 10px 6px" }}>
        <div class="muted xs">{shellText().user.signed_in_as}</div>
        <div style={{ "font-weight": 650 }}>{me()?.display_name ?? me()?.username}</div>
        <span class="chip outline" style={{ "margin-top": "6px" }}>
          {me()?.role === "admin" ? shellText().user.role_admin : shellText().user.role_viewer}
        </span>
        <Show when={me()?.external}>
          <div class="muted xs" style={{ "margin-top": "6px" }}>
            {shellText().user.via_honeclaw}
          </div>
        </Show>
      </div>
      <div class="menu-sep" />
      <Show
        when={!me()?.external}
        fallback={
          <a class="menu-item" href={meta()?.auth?.login_url ?? "https://hone-claw.com/"} data-close>
            <Icon name="user" size={15} /> {shellText().user.honeclaw_account}
          </a>
        }
      >
        <A class="menu-item" href="/settings/security" data-close>
          <Icon name="lock" size={15} /> {c().actions.change_password}
        </A>
        <button class="menu-item" onClick={logout} data-close>
          <Icon name="logout" size={15} /> {c().actions.sign_out}
        </button>
      </Show>
    </Popover>
  );
}

export function Shell(props: ParentProps) {
  const location = useLocation();
  const c = common;
  const groups = navGroups();
  const allItems = groups.flatMap((g) => g.items);
  const title = createMemo(() => {
    const path = location.pathname;
    const match = [...allItems].sort((a, b) => b.href.length - a.href.length).find((item) => isActive(path, item.href));
    return match?.label() ?? "";
  });

  onMount(() => {
    startClock();
    connectEvents();
    refreshMarket();
    refreshUnread();
    const timer = setInterval(refreshMarket, 30_000);
    const off = onServerEvent(["plan", "settings", "strategy", "account"], () => refreshMarket());
    const offNotify = onServerEvent(["notification"], (event) => {
      refreshUnread();
      if (event.type === "notification") {
        const titleText = locale() === "zh" ? event.title_zh : event.title_en;
        toast(titleText, undefined, event.severity === "critical" ? "critical" : event.severity === "warning" ? "warning" : "info");
        maybeBrowserNotify(titleText);
      }
    });
    onCleanup(() => {
      clearInterval(timer);
      off();
      offNotify();
    });
  });

  const mobileItems = () => [allItems[0], allItems[1], allItems[3], allItems[6], allItems[9]];

  return (
    <div class="shell">
      <aside class="sidebar" aria-label="Primary">
        <A class="brand" href="/">
          <img src={withBase("/hone-mark.svg")} alt="" />
          <span class="word">
            <b>HONE</b>
            <span>QUANT</span>
          </span>
        </A>
        <nav>
          <For each={groups}>
            {(group) => (
              <>
                <div class="nav-section kicker">{group.label()}</div>
                <For each={group.items}>
                  {(item) => (
                    <A href={item.href} class="nav-link" classList={{ active: isActive(location.pathname, item.href) }} end={item.href === "/"} title={item.label()}>
                      <Icon name={item.icon} />
                      <span class="label">{item.label()}</span>
                      <Show when={item.badge && item.badge() > 0}>
                        <span class="badge">{item.badge!() > 99 ? "99+" : item.badge!()}</span>
                      </Show>
                    </A>
                  )}
                </For>
              </>
            )}
          </For>
        </nav>
        <div class="sidebar-foot">
          <span class="chip orange" title={c().app.paper_long}>
            <Icon name="shield" size={12} />
            <span class="label">{c().app.paper}</span>
          </span>
          <span class="label">
            {meta()?.data_source === "demo" ? "DEMO" : "FMP"} · v{meta()?.version}
          </span>
          <span class="label" style={{ display: "inline-flex", "align-items": "center", gap: "6px" }}>
            <span style={{ width: "6px", height: "6px", "border-radius": "50%", background: connected() ? "var(--hone-signal-green)" : "var(--hone-signal-yellow)" }} />
            {connected() ? shellText().connection.live : shellText().connection.reconnecting}
          </span>
        </div>
      </aside>
      <div class="main">
        <Show when={meta()?.demo}>
          <div class="demo-banner">
            <Icon name="alert" size={14} />
            {c().app.demo_banner}
          </div>
        </Show>
        <header class="topbar">
          <div class="title">{title()}</div>
          <div class="spacer" />
          <MarketPill />
          <AutomationControl />
          <A href="/notifications" class="btn ghost icon" aria-label={c().nav.notifications} style={{ position: "relative" }}>
            <Icon name="bell" />
            <Show when={unread() > 0}>
              <span
                style={{
                  position: "absolute",
                  top: "3px",
                  right: "3px",
                  "min-width": "16px",
                  height: "16px",
                  padding: "0 4px",
                  "border-radius": "999px",
                  background: "var(--hone-coral-500)",
                  color: "#fff",
                  "font-size": "10px",
                  "font-weight": 700,
                  "line-height": "16px",
                  "text-align": "center",
                }}
              >
                {unread() > 99 ? "99+" : unread()}
              </span>
            </Show>
          </A>
          <PrefsMenu />
          <UserMenu />
        </header>
        <main class="content">{props.children}</main>
      </div>
      <nav class="mobile-nav" aria-label="Mobile">
        <For each={mobileItems()}>
          {(item) => (
            <A href={item.href} classList={{ active: isActive(location.pathname, item.href) }} end={item.href === "/"}>
              <Icon name={item.icon} size={20} />
              {item.label()}
            </A>
          )}
        </For>
      </nav>
      <Toasts />
      <ConfirmHost />
    </div>
  );
}

function maybeBrowserNotify(title: string) {
  try {
    if (typeof Notification === "undefined" || document.visibilityState === "visible") return;
    if (Notification.permission === "granted") new Notification("hone-quant", { body: title, icon: withBase("/hone-mark.svg") });
  } catch {
    /* unsupported */
  }
}

export function requestBrowserNotifications() {
  try {
    if (typeof Notification !== "undefined" && Notification.permission === "default") void Notification.requestPermission();
  } catch {
    /* unsupported */
  }
}

export { createSignal };
