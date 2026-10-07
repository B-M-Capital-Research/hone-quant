/**
 * Portfolios: every portfolio the user can see as a card (owner, strategy, automation, NAV and
 * returns), with switching, renaming, archiving and creating. Archived portfolios are listed on
 * request.
 */
import { useSearchParams } from "@solidjs/router";
import { For, Show, createResource, createSignal, onCleanup, onMount } from "solid-js";
import { Icon } from "@/components/Icon";
import { NoPortfolio } from "@/components/NoPortfolio";
import { ModeChip, ownerLabel } from "@/components/PortfolioSwitcher";
import { Empty, ErrorState, Loading, Pct, Switch } from "@/components/ui";
import { tpl } from "@/i18n";
import { portfoliosText } from "@/i18n/portfolios";
import { api } from "@/lib/api";
import { onServerEvent } from "@/lib/events";
import { fmtDate, fmtDateTime, fmtMoney } from "@/lib/format";
import { strategyName } from "@/lib/names";
import { canCreate, noPortfolio, selectPortfolio, selectedId } from "@/lib/portfolio";
import type { Portfolio } from "@/lib/types";
import "@/styles/portfolios.css";
import { throttle } from "./overview/util";
import { ArchivePortfolioDialog, EditPortfolioDialog } from "./portfolios/ManageDialogs";
import NewPortfolioDialog from "./portfolios/NewPortfolioDialog";

export default function Portfolios() {
  const t = portfoliosText;
  const [params, setParams] = useSearchParams<{ archived?: string }>();
  const showArchived = () => params.archived === "1";
  const [data, { refetch }] = createResource(
    () => ({ archived: showArchived() }),
    (q) => api.portfolios(q.archived),
  );
  const [creating, setCreating] = createSignal(false);
  const [editing, setEditing] = createSignal<Portfolio | null>(null);
  const [archiving, setArchiving] = createSignal<Portfolio | null>(null);

  onMount(() => {
    const off = onServerEvent(["portfolios", "account", "settings", "strategy"], throttle(() => void refetch(), 1500));
    onCleanup(off);
  });

  /** Active portfolios first (in the server's order), archived ones after them. */
  const list = () => {
    const all = data.latest?.portfolios ?? [];
    return [...all.filter((p) => p.status === "active"), ...all.filter((p) => p.status !== "active")];
  };

  return (
    <div class="stack">
      <div class="page-head">
        <div style={{ flex: 1, "min-width": "280px" }}>
          <h1>{t().title}</h1>
          <p class="lead">{t().lead}</p>
        </div>
        <div class="row wrap">
          <Switch checked={showArchived()} onChange={(v) => setParams({ archived: v ? "1" : undefined }, { replace: true })} label={t().actions.show_archived} />
          <Show when={canCreate()}>
            <button type="button" class="btn primary" onClick={() => setCreating(true)}>
              <Icon name="plus" size={16} /> {t().actions.new}
            </button>
          </Show>
        </div>
      </div>

      <Show when={data.latest} fallback={data.error ? <ErrorState error={data.error} onRetry={() => void refetch()} /> : <Loading />}>
        <Show
          when={list().length}
          fallback={
            <Show when={noPortfolio()} fallback={<Empty title={t().list.empty_archived} icon="briefcase" />}>
              <NoPortfolio onCreate={() => setCreating(true)} />
            </Show>
          }
        >
          <p class="muted xs">{tpl(t().list.count, { n: list().length })}</p>
          <div class="pf-grid" classList={{ refetching: data.loading }}>
            <For each={list()}>{(p) => <PortfolioCard portfolio={p} onEdit={setEditing} onArchive={setArchiving} />}</For>
          </div>
        </Show>
      </Show>

      <Show when={creating()}>
        <NewPortfolioDialog onClose={() => setCreating(false)} onCreated={() => void refetch()} />
      </Show>
      <Show when={editing()}>
        {(p) => (
          <EditPortfolioDialog
            portfolio={p()}
            onClose={() => setEditing(null)}
            onSaved={() => {
              setEditing(null);
              void refetch();
            }}
          />
        )}
      </Show>
      <Show when={archiving()}>
        {(p) => (
          <ArchivePortfolioDialog
            portfolio={p()}
            onClose={() => setArchiving(null)}
            onArchived={() => {
              setArchiving(null);
              void refetch();
            }}
          />
        )}
      </Show>
    </div>
  );
}

function PortfolioCard(props: { portfolio: Portfolio; onEdit: (p: Portfolio) => void; onArchive: (p: Portfolio) => void }) {
  const t = portfoliosText;
  const p = () => props.portfolio;
  const current = () => p().id === selectedId();
  const archived = () => p().status !== "active";
  return (
    <article class="card pf-card" classList={{ current: current() && !archived(), archived: archived() }}>
      <header class="pf-card-head">
        <div class="pf-card-title">
          <h2 title={p().name}>{p().name}</h2>
          <div class="pf-card-owner">
            <Icon name={p().owner === null ? "shield" : "user"} size={12} />
            <span>{ownerLabel(p())}</span>
          </div>
        </div>
        <Show when={current() && !archived()}>
          <span class="chip green">
            <span class="dot" />
            {t().card.current}
          </span>
        </Show>
        <Show when={archived()}>
          <span class="chip">{t().card.archived}</span>
        </Show>
        <Show when={!archived() && !p().can_trade}>
          <span class="chip outline" title={t().card.read_only_hint}>
            <Icon name="lock" size={11} />
            {t().card.read_only}
          </span>
        </Show>
      </header>

      <p class="pf-card-desc" classList={{ muted: !p().description }}>
        {p().description || t().card.no_description}
      </p>

      <Show when={p().summary}>
        {(s) => (
          <>
            <div class="pf-card-kpis">
              <div>
                <span class="label">{t().card.nav}</span>
                <span class="value num">{fmtMoney(s().nav, { dp: 0 })}</span>
              </div>
              <div>
                <span class="label">{t().card.total_return}</span>
                <span class="value">
                  <Pct value={s().total_return} />
                </span>
              </div>
              <div>
                <span class="label">{t().card.day_return}</span>
                <span class="value">
                  <Pct value={s().day_return} />
                </span>
              </div>
            </div>
            <dl class="kv pf-card-kv">
              <dt>{t().card.strategy}</dt>
              <dd>
                <Show when={s().strategy} fallback={<span class="muted">{t().card.no_strategy}</span>}>
                  {(v) => (
                    <>
                      {strategyName(v())} <span class="muted num">#{v().id}</span>
                    </>
                  )}
                </Show>
              </dd>
              <dt>{t().card.automation}</dt>
              <dd>
                <ModeChip mode={p().effective_mode} />
              </dd>
              <dt>{t().card.positions}</dt>
              <dd class="num">{tpl(t().card.positions_value, { n: s().positions })}</dd>
              <dt>{t().card.initial_cash}</dt>
              <dd class="num">{fmtMoney(s().initial_cash, { dp: 0 })}</dd>
              <dt>{t().card.inception}</dt>
              <dd class="num">{fmtDate(s().inception_date)}</dd>
            </dl>
          </>
        )}
      </Show>

      <footer class="pf-card-foot">
        <Show
          when={!archived()}
          fallback={<span class="muted xs">{tpl(t().card.archived_at, { time: fmtDateTime(p().archived_at) })}</span>}
        >
          <Show
            when={!current()}
            fallback={
              <span class="pf-current-note">
                <Icon name="check" size={14} /> {t().card.current}
              </span>
            }
          >
            <button type="button" class="btn sm primary" onClick={() => selectPortfolio(p().id)}>
              {t().actions.switch_to}
            </button>
          </Show>
          <span class="spacer" />
          <Show when={p().can_trade}>
            <button type="button" class="btn sm ghost" onClick={() => props.onEdit(p())}>
              {t().actions.edit}
            </button>
          </Show>
          <button
            type="button"
            class="btn sm danger"
            disabled={!p().can_trade}
            title={p().can_trade ? undefined : t().card.archive_disabled}
            onClick={() => props.onArchive(p())}
          >
            {t().actions.archive}
          </button>
        </Show>
      </footer>
    </article>
  );
}
