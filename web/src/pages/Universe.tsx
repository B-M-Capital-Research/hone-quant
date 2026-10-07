import { useSearchParams } from "@solidjs/router";
import { For, Show, createMemo, createResource, createSignal, onCleanup, onMount } from "solid-js";
import "@/styles/universe.css";
import { Icon } from "@/components/Icon";
import { ErrorState, Loading } from "@/components/ui";
import { tpl } from "@/i18n";
import { universeText } from "@/i18n/universe";
import { api } from "@/lib/api";
import { onServerEvent } from "@/lib/events";
import { marketToday } from "@/lib/format";
import { canTrade, onPortfolioEvent } from "@/lib/portfolio";
import { isAdmin, serverNow } from "@/lib/session";
import type { UniverseView } from "@/lib/types";
import { makeWeights, sectorRows } from "./universe/helpers";
import { CompaniesTable, SectorsOverview } from "./universe/Members";
import { ActiveRestrictions, AddRestrictionDialog, ModeExplainer, RestrictionHistory } from "./universe/Restrictions";
import { BenchmarksCard, RemovedCard, SourceCard } from "./universe/Source";
import { CheckPanel, VersionsTable } from "./universe/Updates";

type Tab = "members" | "restrictions" | "updates";
const TABS: Tab[] = ["members", "restrictions", "updates"];

export default function Universe() {
  const t = universeText;
  const [params, setParams] = useSearchParams();
  const [view, { refetch: refetchView }] = createResource(() => api.universe());
  const [restrictions, { refetch: refetchRestrictions }] = createResource(() => api.restrictions().catch(() => null));
  const [dashboard, { refetch: refetchDashboard }] = createResource(() => api.dashboard().catch(() => null));
  const [settings] = createResource(() => api.settings().catch(() => null));
  const [sectorFilter, setSectorFilter] = createSignal("");
  const [addFor, setAddFor] = createSignal<string | null>(null);

  const tab = (): Tab => {
    const value = typeof params.tab === "string" ? params.tab : "";
    return (TABS as string[]).includes(value) ? (value as Tab) : "members";
  };
  const setTab = (next: Tab) => setParams({ tab: next === "members" ? undefined : next });
  const focus = () => (typeof params.symbol === "string" && params.symbol ? params.symbol.toUpperCase() : null);

  let dashTimer: ReturnType<typeof setTimeout> | undefined;
  onMount(() => {
    const offUniverse = onServerEvent(["universe"], () => void refetchView());
    const offSettings = onPortfolioEvent(["settings"], (e) => {
      if (e.type !== "settings" || e.key === "restrictions" || e.key === "benchmarks") {
        void refetchRestrictions();
      }
    });
    const offAccount = onPortfolioEvent(["plan", "account"], () => {
      clearTimeout(dashTimer);
      dashTimer = setTimeout(() => void refetchDashboard(), 1200);
    });
    onCleanup(() => {
      offUniverse();
      offSettings();
      offAccount();
      clearTimeout(dashTimer);
    });
  });

  const data = (): UniverseView | undefined => (view.error ? undefined : view.latest);
  const weights = createMemo(() => makeWeights(dashboard.error ? null : dashboard.latest));
  const today = () => marketToday(new Date(serverNow()));
  const active = () => restrictions.latest?.active ?? data()?.restrictions ?? [];
  const rows = createMemo(() => (data() ? sectorRows(data()!, weights()) : []));

  const restrictionsChanged = () => {
    void refetchRestrictions();
    void refetchView();
  };

  // Deep link /universe?symbol=NVDA: show the members tab focused on that company.
  onMount(() => {
    if (focus() && tab() !== "members") setTab("members");
    if (focus()) setTimeout(() => document.querySelector(".uv-row.focused")?.scrollIntoView({ behavior: "smooth", block: "center" }), 600);
  });

  return (
    <div class="uv-page">
      <div class="page-head">
        <div style={{ flex: "1 1 420px", "min-width": 0 }}>
          <h1>{t().title}</h1>
          <p class="lead">{t().lead}</p>
        </div>
        <div class="row wrap">
          <Show when={isAdmin() && tab() !== "updates"}>
            <button class="btn" onClick={() => setTab("updates")}>
              <Icon name="refresh" size={15} /> {t().actions.check}
            </button>
          </Show>
          <Show when={canTrade()}>
            <button class="btn primary" onClick={() => setAddFor("")} disabled={!data()}>
              <Icon name="plus" size={15} /> {t().actions.add_restriction}
            </button>
          </Show>
        </div>
      </div>

      <div class="tabs uv-tabs" role="tablist" aria-label={t().title}>
        <For each={TABS}>
          {(id) => (
            <button role="tab" aria-selected={tab() === id} onClick={() => setTab(id)}>
              {t().tabs[id]}
              <Show when={id === "restrictions" && active().length}>
                <span class="uv-count">{active().length}</span>
              </Show>
            </button>
          )}
        </For>
      </div>

      <Show when={!view.error} fallback={<ErrorState error={view.error} onRetry={() => void refetchView()} />}>
        <Show when={data()} fallback={<Loading />}>
          {(u) => (
            <>
              <Show when={tab() === "members"}>
                <div class="stack">
                  <div class="grid main-side uv-top">
                    <section class="card">
                      <div class="card-head">
                        <div style={{ flex: 1, "min-width": 0 }}>
                          <h2>{t().sectors.title}</h2>
                          <div class="sub">{t().sectors.sub}</div>
                        </div>
                        <Show when={sectorFilter()}>
                          <button class="btn sm ghost" onClick={() => setSectorFilter("")}>
                            <Icon name="x" size={13} /> {t().actions.clear}
                          </button>
                        </Show>
                      </div>
                      <div class="card-body" classList={{ refetching: dashboard.loading }}>
                        <SectorsOverview rows={rows()} weights={weights()} selected={sectorFilter()} onSelect={setSectorFilter} />
                      </div>
                    </section>
                    <div class="stack">
                      <SourceCard view={u()} />
                      <BenchmarksCard view={u()} settings={settings()?.benchmarks ?? null} />
                    </div>
                  </div>

                  <section class="card">
                    <div class="card-head">
                      <div style={{ flex: 1, "min-width": 0 }}>
                        <h2>{t().companies.title}</h2>
                        <div class="sub">{t().companies.sub}</div>
                      </div>
                    </div>
                    <div class="card-body">
                      <CompaniesTable
                        view={u()}
                        weights={weights()}
                        restrictions={active()}
                        sectorFilter={sectorFilter()}
                        onSectorFilter={setSectorFilter}
                        focus={focus()}
                        onClearFocus={() => setParams({ symbol: undefined })}
                        onRestrict={(symbol) => setAddFor(symbol)}
                      />
                    </div>
                  </section>

                  <Show when={u().removed.length}>
                    <RemovedCard view={u()} weights={weights()} />
                  </Show>
                </div>
              </Show>

              <Show when={tab() === "restrictions"}>
                <div class="stack">
                  <ModeExplainer />
                  <div class="callout info">
                    <Icon name="clock" size={16} />
                    <span>{t().restrictions.timing}</span>
                  </div>
                  <section class="card">
                    <div class="card-head">
                      <div style={{ flex: 1, "min-width": 0 }}>
                        <h2>{t().restrictions.title}</h2>
                        <div class="sub">{tpl(t().restrictions.sub, { n: active().length })}</div>
                      </div>
                    </div>
                    <div class="card-body flush">
                      <ActiveRestrictions view={u()} active={active()} onChanged={restrictionsChanged} onAdd={() => setAddFor("")} />
                    </div>
                  </section>
                  <section class="card">
                    <div class="card-head">
                      <div style={{ flex: 1, "min-width": 0 }}>
                        <h2>{t().restrictions.history}</h2>
                        <div class="sub">{t().restrictions.history_sub}</div>
                      </div>
                    </div>
                    <div class="card-body flush">
                      <RestrictionHistory view={u()} history={restrictions.latest?.history ?? []} today={today()} />
                    </div>
                  </section>
                </div>
              </Show>

              <Show when={tab() === "updates"}>
                <div class="stack">
                  <section class="card">
                    <div class="card-head">
                      <div style={{ flex: 1, "min-width": 0 }}>
                        <h2>{t().updates.check_title}</h2>
                        <div class="sub">{t().updates.check_sub}</div>
                      </div>
                    </div>
                    <div class="card-body">
                      <CheckPanel view={u()} weights={weights()} onApplied={() => void refetchView()} />
                    </div>
                  </section>
                  <section class="card">
                    <div class="card-head">
                      <div style={{ flex: 1, "min-width": 0 }}>
                        <h2>{t().updates.versions_title}</h2>
                        <div class="sub">{t().updates.versions_sub}</div>
                      </div>
                    </div>
                    <div class="card-body flush">
                      <VersionsTable versions={u().versions} />
                    </div>
                  </section>
                </div>
              </Show>

              <Show when={addFor() !== null}>
                <AddRestrictionDialog
                  view={u()}
                  weights={weights()}
                  active={active()}
                  today={today()}
                  initialSymbol={addFor() ?? ""}
                  onClose={() => setAddFor(null)}
                  onAdded={() => {
                    setAddFor(null);
                    restrictionsChanged();
                  }}
                />
              </Show>
            </>
          )}
        </Show>
      </Show>
    </div>
  );
}
