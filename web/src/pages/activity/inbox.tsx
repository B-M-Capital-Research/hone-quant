/**
 * Notification inbox: day-grouped list with severity, category, delivery status per channel,
 * quiet-hours deferral and links. Paged with the `before` cursor; live through SSE.
 */
import { A } from "@solidjs/router";
import { For, Match, Show, Switch, batch, createEffect, createMemo, createSignal, on, onCleanup, onMount } from "solid-js";
import { createStore, reconcile, unwrap } from "solid-js/store";
import { Icon, type IconName } from "@/components/Icon";
import { Empty, ErrorState, Loading, Segmented, toast, toastError } from "@/components/ui";
import { locale, tpl } from "@/i18n";
import { common } from "@/i18n/common";
import { notificationsText } from "@/i18n/notifications";
import { api } from "@/lib/api";
import { onServerEvent } from "@/lib/events";
import { fmtDual, fmtNum, fmtRelative } from "@/lib/format";
import { displayTz } from "@/lib/prefs";
import { serverNow, setUnread, unread } from "@/lib/session";
import type { NotificationRow, Severity } from "@/lib/types";
import { XIcon } from "./icons";
import { addDays, dateIn } from "./util";

const PAGE = 30;
const CATEGORIES = ["plan", "execution", "risk", "reminder", "report", "data", "system"] as const;

const SEVERITY_TONE: Record<Severity, string> = { info: "blue", success: "green", warning: "yellow", critical: "red" };

export function SeverityIcon(props: { severity: Severity; size?: number }) {
  const icon: Record<Exclude<Severity, "critical">, IconName> = { info: "info", success: "check", warning: "alert" };
  return (
    <Show when={props.severity !== "critical"} fallback={<XIcon name="octagon" size={props.size ?? 16} />}>
      <Icon name={icon[props.severity as Exclude<Severity, "critical">] ?? "info"} size={props.size ?? 16} />
    </Show>
  );
}

export function SeverityChip(props: { severity: Severity }) {
  return (
    <span class={`chip ${SEVERITY_TONE[props.severity] ?? ""}`}>
      <SeverityIcon severity={props.severity} size={11} />
      {common().severity[props.severity] ?? props.severity}
    </span>
  );
}

function categoryLabel(category: string): string {
  return (common().category as Record<string, string>)[category] ?? category;
}

/** "今天 · 10月5日 周一" / "Today · Mon, Oct 5". */
function dayLabel(day: string): string {
  const n = notificationsText().inbox;
  const today = dateIn(serverNow(), displayTz());
  const [y, m, d] = day.split("-").map(Number);
  const date = new Date(Date.UTC(y, m - 1, d, 12));
  const sameYear = day.slice(0, 4) === today.slice(0, 4);
  const formatted =
    locale() === "zh"
      ? `${sameYear ? "" : `${y}年`}${m}月${d}日 ${date.toLocaleDateString("zh-CN", { timeZone: "UTC", weekday: "short" })}`
      : date.toLocaleDateString("en-US", {
          timeZone: "UTC",
          year: sameYear ? undefined : "numeric",
          month: "short",
          day: "numeric",
          weekday: "short",
        });
  if (day === today) return `${n.today} · ${formatted}`;
  if (day === addDays(today, -1)) return `${n.yesterday} · ${formatted}`;
  return formatted;
}

export function Inbox(props: {
  unreadOnly: boolean;
  category: string;
  onFilter: (patch: { unread?: boolean; category?: string }) => void;
}) {
  const n = notificationsText;
  const [state, setState] = createStore({ items: [] as NotificationRow[] });
  const [loading, setLoading] = createSignal(true);
  const [error, setError] = createSignal<unknown>(null);
  const [hasMore, setHasMore] = createSignal(false);
  const [loadingMore, setLoadingMore] = createSignal(false);
  const [busyAll, setBusyAll] = createSignal(false);
  const [fresh, setFresh] = createSignal<Set<number>>(new Set());
  let seq = 0;

  const filter = () => ({ unread: props.unreadOnly || undefined, category: props.category || undefined });

  const load = async () => {
    const id = ++seq;
    setLoading(true);
    setError(null);
    try {
      const res = await api.notifications({ ...filter(), limit: PAGE });
      if (id !== seq) return;
      batch(() => {
        setState("items", reconcile(res.notifications, { key: "id" }));
        setHasMore(res.notifications.length === PAGE);
        setUnread(res.unread);
      });
    } catch (e) {
      if (id === seq) setError(e);
    } finally {
      if (id === seq) setLoading(false);
    }
  };

  /** Merges the newest page into the list (new rows on top, updated deliveries/read state). */
  const refreshTop = async () => {
    const id = seq;
    try {
      const res = await api.notifications({ ...filter(), limit: PAGE });
      if (id !== seq) return;
      const current = unwrap(state.items);
      const maxId = current.length ? current[0].id : 0;
      const incoming = res.notifications.filter((row) => row.id > maxId);
      const byId = new Map(res.notifications.map((row) => [row.id, row]));
      const merged = [...incoming, ...current.map((row) => byId.get(row.id) ?? row)];
      batch(() => {
        setState("items", reconcile(merged, { key: "id" }));
        setUnread(res.unread);
        if (incoming.length) {
          const ids = new Set(incoming.map((row) => row.id));
          setFresh(ids);
          setTimeout(() => setFresh(new Set<number>()), 2400);
        }
      });
    } catch {
      /* the next event or a manual reload will catch up */
    }
  };

  const loadMore = async () => {
    const last = state.items[state.items.length - 1];
    if (!last || loadingMore()) return;
    setLoadingMore(true);
    const id = seq;
    try {
      const res = await api.notifications({ ...filter(), before: last.id, limit: PAGE });
      if (id !== seq) return;
      const known = new Set(state.items.map((row) => row.id));
      const more = res.notifications.filter((row) => !known.has(row.id));
      batch(() => {
        setState("items", (items) => [...items, ...more]);
        setHasMore(res.notifications.length === PAGE);
      });
    } catch (e) {
      toastError(e);
    } finally {
      setLoadingMore(false);
    }
  };

  createEffect(on(() => [props.unreadOnly, props.category], () => load()));

  onMount(() => {
    let timer: ReturnType<typeof setTimeout> | null = null;
    const off = onServerEvent(["notification"], (event) => {
      if (event.type === "resync") {
        load();
        return;
      }
      refreshTop();
      // Outbound deliveries are recorded a moment later; pick them up too.
      if (timer) clearTimeout(timer);
      timer = setTimeout(refreshTop, 5000);
    });
    // Delivery results and relative times age: refresh quietly every minute.
    const interval = setInterval(refreshTop, 60_000);
    onCleanup(() => {
      off();
      clearInterval(interval);
      if (timer) clearTimeout(timer);
    });
  });

  const markRead = async (row: NotificationRow) => {
    if (row.read_at) return;
    const index = state.items.findIndex((item) => item.id === row.id);
    const stamp = new Date(serverNow()).toISOString();
    if (index >= 0) setState("items", index, "read_at", stamp);
    setUnread(Math.max(0, unread() - 1));
    try {
      const res = await api.readNotification(row.id);
      setUnread(res.unread);
    } catch (e) {
      if (index >= 0) setState("items", index, "read_at", null);
      toastError(e);
    }
  };

  const markAll = async () => {
    if (busyAll()) return;
    setBusyAll(true);
    try {
      const res = await api.readAllNotifications();
      const stamp = new Date(serverNow()).toISOString();
      batch(() => {
        setState("items", (row) => row.read_at === null, "read_at", stamp);
        setUnread(res.unread);
      });
      toast(n().inbox.marked_all, undefined, "success", 2400);
    } catch (e) {
      toastError(e);
    } finally {
      setBusyAll(false);
    }
  };

  const groups = createMemo(() => {
    const map = new Map<string, NotificationRow[]>();
    for (const row of state.items) {
      const day = dateIn(Date.parse(row.ts), displayTz());
      const list = map.get(day);
      if (list) list.push(row);
      else map.set(day, [row]);
    }
    return map;
  });
  const days = createMemo(() => [...groups().keys()]);

  return (
    <section class="card ntf-card">
      <div class="card-head ntf-head">
        <Segmented
          label={n().inbox.show}
          value={props.unreadOnly ? "unread" : "all"}
          onChange={(value) => props.onFilter({ unread: value === "unread" })}
          options={[
            { value: "all", label: n().inbox.all },
            { value: "unread", label: unread() > 0 ? `${n().inbox.unread} ${fmtNum(unread(), 0)}` : n().inbox.unread },
          ]}
        />
        <div class="ntf-cats" role="group" aria-label={n().inbox.category}>
          <button type="button" class="ntf-cat" aria-pressed={!props.category} onClick={() => props.onFilter({ category: "" })}>
            {n().inbox.all_categories}
          </button>
          <For each={CATEGORIES}>
            {(category) => (
              <button
                type="button"
                class="ntf-cat"
                aria-pressed={props.category === category}
                onClick={() => props.onFilter({ category: props.category === category ? "" : category })}
              >
                {categoryLabel(category)}
              </button>
            )}
          </For>
        </div>
        <span class="spacer" />
        <button type="button" class="btn sm" disabled={unread() === 0 || busyAll()} onClick={markAll}>
          <XIcon name="check_all" size={15} />
          {n().inbox.mark_all}
        </button>
      </div>

      <div class="card-body flush" classList={{ refetching: loading() && state.items.length > 0 }}>
        <Switch>
          <Match when={error() && !state.items.length}>
            <ErrorState error={error()} onRetry={load} />
          </Match>
          <Match when={loading() && !state.items.length}>
            <Loading />
          </Match>
          <Match when={!state.items.length}>
            <Show
              when={props.unreadOnly}
              fallback={
                <Empty title={props.category ? n().inbox.empty_category : n().inbox.empty} icon="bell">
                  <span>{n().inbox.empty_hint}</span>
                </Empty>
              }
            >
              <Empty title={n().inbox.empty_unread} icon="check">
                <span>{n().inbox.empty_unread_hint}</span>
              </Empty>
            </Show>
          </Match>
          <Match when={true}>
            <For each={days()}>
              {(day) => (
                <div class="ntf-day">
                  <h3 class="ntf-day-head">{dayLabel(day)}</h3>
                  <ul class="ntf-list">
                    <For each={groups().get(day) ?? []}>
                      {(row) => <Item row={row} fresh={fresh().has(row.id)} onRead={markRead} />}
                    </For>
                  </ul>
                </div>
              )}
            </For>
          </Match>
        </Switch>
      </div>

      <Show when={state.items.length > 0}>
        <div class="card-foot">
          <span>{tpl(n().inbox.loaded, { n: fmtNum(state.items.length, 0) })}</span>
          <span class="spacer" />
          <Show when={hasMore()} fallback={<span>{n().inbox.end}</span>}>
            <button type="button" class="btn sm" disabled={loadingMore()} onClick={loadMore}>
              {loadingMore() ? common().states.loading : n().inbox.load_more}
            </button>
          </Show>
        </div>
      </Show>
    </section>
  );
}

function Item(props: { row: NotificationRow; fresh: boolean; onRead: (row: NotificationRow) => void }) {
  const n = notificationsText;
  const row = props.row;
  const title = () => (locale() === "zh" ? row.title_zh || row.title_en : row.title_en || row.title_zh);
  const body = () => (locale() === "zh" ? row.body_zh || row.body_en : row.body_en || row.body_zh);
  return (
    <li class={`ntf-item sev-${row.severity}`} classList={{ unread: !row.read_at, fresh: props.fresh }}>
      <div class="ntf-icon" aria-hidden="true">
        <SeverityIcon severity={row.severity} size={17} />
      </div>
      <div class="ntf-main">
        <div class="ntf-top">
          <h4 class="ntf-title">
            <Show when={!row.read_at}>
              <span class="ntf-dot" title={n().inbox.unread_dot}>
                <span class="visually-hidden">{n().inbox.unread_dot}</span>
              </span>
            </Show>
            {title()}
          </h4>
          <time class="ntf-time" datetime={row.ts} title={fmtDual(row.ts, true)}>
            {fmtRelative(row.ts, serverNow())}
          </time>
        </div>
        <Show when={body()}>
          <p class="ntf-body">{body()}</p>
        </Show>
        <div class="ntf-meta">
          <SeverityChip severity={row.severity} />
          <span class="chip outline">{categoryLabel(row.category)}</span>
          <Deliveries row={row} />
          <span class="spacer" />
          <Show when={row.read_at}>
            <span class="ntf-read" title={fmtDual(row.read_at, true)}>
              <Icon name="check" size={12} />
              {tpl(n().inbox.read_at, { time: fmtRelative(row.read_at, serverNow()) })}
            </span>
          </Show>
          <Show when={!row.read_at}>
            <button type="button" class="btn ghost sm" onClick={() => props.onRead(row)}>
              <Icon name="check" size={13} />
              {n().inbox.mark_read}
            </button>
          </Show>
          <Show when={row.link}>
            <A href={row.link!} class="btn sm ntf-open" title={n().inbox.open_hint} onClick={() => props.onRead(row)}>
              {n().inbox.open}
              <XIcon name="arrow_right" size={13} />
            </A>
          </Show>
        </div>
      </div>
    </li>
  );
}

function Deliveries(props: { row: NotificationRow }) {
  const d = () => notificationsText().inbox.delivery;
  return (
    <Switch>
      <Match when={props.row.deferred}>
        <span class="chip yellow" title={d().deferred_hint}>
          <Icon name="moon" size={11} />
          {d().deferred}
        </span>
      </Match>
      <Match when={!props.row.deliveries?.length}>
        <span class="chip outline ntf-inapp" title={d().in_app_hint}>
          <XIcon name="inbox" size={11} />
          {d().in_app}
        </span>
      </Match>
      <Match when={true}>
        <For each={props.row.deliveries}>
          {(delivery) => (
            <span
              class={`chip ${delivery.ok ? "green" : "red"}`}
              title={
                delivery.ok
                  ? tpl(d().sent_hint, { channel: delivery.channel, time: fmtDual(delivery.at, true) })
                  : tpl(d().failed_hint, { channel: delivery.channel, time: fmtDual(delivery.at, true), error: delivery.error ?? "" })
              }
            >
              <Icon name={delivery.ok ? "check" : "x"} size={11} />
              {tpl(delivery.ok ? d().sent : d().failed, { channel: delivery.channel })}
              <Show when={delivery.digest}>
                <span class="ntf-digest" title={d().digest_hint}>
                  · {d().digest}
                </span>
              </Show>
            </span>
          )}
        </For>
      </Match>
    </Switch>
  );
}
