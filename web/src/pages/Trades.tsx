/**
 * /trades — orders, fills and the cash ledger of the paper account.
 *
 * Deep-linkable: `?tab=orders|fills|ledger&symbol=NVDA&from=2026-10-01&to=2026-10-05&status=filled`
 * (`kind=trade|dividend|deposit|adjustment` on the ledger tab). Refreshes on `plan` and `account`
 * server events.
 */
import "@/styles/activity.css";
import { useSearchParams } from "@solidjs/router";
import { Match, Switch, createMemo, createSignal, onCleanup, onMount } from "solid-js";
import { Icon } from "@/components/Icon";
import { common } from "@/i18n/common";
import { tradesText } from "@/i18n/trades";
import { onServerEvent } from "@/lib/events";
import { LiveBadge, TabBar } from "./activity/components";
import { FillsTab } from "./activity/fills";
import { LedgerTab } from "./activity/ledger";
import { useNames } from "./activity/names";
import { OrdersTab } from "./activity/orders";
import type { TradeFilters } from "./activity/trades-common";
import { debounce, isDate, qstr } from "./activity/util";

type TradesTab = "orders" | "fills" | "ledger";

export default function Trades() {
  const t = tradesText;
  const [params, setParams] = useSearchParams();

  const tab = (): TradesTab => {
    const value = qstr(params.tab);
    return value === "fills" || value === "ledger" ? value : "orders";
  };
  const filters = createMemo<TradeFilters>(() => ({
    symbol: qstr(params.symbol).trim().toUpperCase(),
    from: isDate(qstr(params.from)) ? qstr(params.from) : "",
    to: isDate(qstr(params.to)) ? qstr(params.to) : "",
    status: qstr(params.status),
    kind: qstr(params.kind),
  }));
  const setFilters = (patch: Partial<TradeFilters>) => setParams(patch, { replace: true });
  const setTab = (id: TradesTab) =>
    setParams({
      tab: id,
      // Status only applies to orders and the entry type only to the ledger.
      status: id === "orders" ? filters().status : "",
      kind: id === "ledger" ? filters().kind : "",
    });

  const names = useNames();
  const [tick, setTick] = createSignal(0);
  onMount(() => {
    const bump = debounce(() => setTick((n) => n + 1), 700);
    const off = onServerEvent(["plan", "account"], () => bump());
    onCleanup(() => {
      off();
      bump.cancel();
    });
  });

  return (
    <div class="act-page">
      <header class="page-head">
        <div class="act-head-text">
          <h1>{t().title}</h1>
          <p class="lead">{t().lead}</p>
        </div>
        <span class="spacer" />
        <div class="act-head-actions">
          <LiveBadge />
          <button type="button" class="btn sm" onClick={() => setTick((n) => n + 1)}>
            <Icon name="refresh" size={14} />
            {common().actions.refresh}
          </button>
        </div>
      </header>

      <TabBar
        label={t().title}
        value={tab()}
        onChange={setTab}
        tabs={[
          { id: "orders", label: t().tabs.orders },
          { id: "fills", label: t().tabs.fills },
          { id: "ledger", label: t().tabs.ledger },
        ]}
      />

      <div class="act-tabpanel" role="tabpanel">
        <Switch>
          <Match when={tab() === "orders"}>
            <OrdersTab filters={filters()} setFilters={setFilters} names={names} tick={tick} />
          </Match>
          <Match when={tab() === "fills"}>
            <FillsTab filters={filters()} setFilters={setFilters} names={names} tick={tick} />
          </Match>
          <Match when={tab() === "ledger"}>
            <LedgerTab filters={filters()} setFilters={setFilters} names={names} tick={tick} />
          </Match>
        </Switch>
      </div>
    </div>
  );
}
