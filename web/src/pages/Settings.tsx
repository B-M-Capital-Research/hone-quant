/**
 * Settings: a sub-navigation of sections (a select on narrow screens) and the active section.
 * Sections that edit the settings bundle share one loader through context, refetched whenever
 * the server announces a settings change.
 */
import { A, Navigate, useNavigate, useParams } from "@solidjs/router";
import { For, Match, Show, Switch, createEffect, createMemo, onCleanup, onMount } from "solid-js";
import { settingsText } from "@/i18n/settings";
import { api } from "@/lib/api";
import { onServerEvent } from "@/lib/events";
import { isAdmin } from "@/lib/session";
import AccountSection from "@/pages/settings/Account";
import AutomationSection from "@/pages/settings/Automation";
import BenchmarksSection from "@/pages/settings/Benchmarks";
import DataSection from "@/pages/settings/Data";
import DisplaySection from "@/pages/settings/Display";
import ExecutionSection from "@/pages/settings/Execution";
import NotificationsSection from "@/pages/settings/Notifications";
import RiskSection from "@/pages/settings/Risk";
import ScheduleSection from "@/pages/settings/Schedule";
import SecuritySection from "@/pages/settings/Security";
import UsersSection from "@/pages/settings/Users";
import { LocalConfirmHost, SIcon, type SIconName, SettingsCtx, createLoader } from "@/pages/settings/shared";
import "@/styles/settings.css";

type SectionKey =
  | "schedule"
  | "automation"
  | "execution"
  | "risk"
  | "notifications"
  | "data"
  | "display"
  | "benchmarks"
  | "account"
  | "users"
  | "security";

const GROUPS: { key: "trading" | "notifications" | "data" | "access"; items: SectionKey[] }[] = [
  { key: "trading", items: ["schedule", "automation", "execution", "risk"] },
  { key: "notifications", items: ["notifications"] },
  { key: "data", items: ["data", "display", "benchmarks"] },
  { key: "access", items: ["account", "users", "security"] },
];

const SECTIONS = GROUPS.flatMap((g) => g.items);

const ICONS: Record<SectionKey, SIconName> = {
  schedule: "clock",
  automation: "zap",
  execution: "sliders",
  risk: "shield",
  notifications: "bell",
  data: "database",
  display: "monitor",
  benchmarks: "target",
  account: "wallet",
  users: "users",
  security: "lock",
};

/** Sections whose content (not just editing) is admin-only. */
const ADMIN_ONLY: SectionKey[] = ["users"];

export default function Settings() {
  const t = settingsText;
  const params = useParams<{ section?: string }>();
  const navigate = useNavigate();
  const known = () => !params.section || (SECTIONS as string[]).includes(params.section);
  const section = createMemo<SectionKey>(() => (known() && params.section ? (params.section as SectionKey) : "schedule"));
  const meta = () => t().sections[section()];

  const bundle = createLoader(() => api.settings());
  onMount(() => {
    const off = onServerEvent(["settings"], () => void bundle.reload());
    onCleanup(off);
  });

  const previousTitle = document.title;
  createEffect(() => {
    document.title = `${meta().title} · ${t().page_title} · hone-quant`;
  });
  onCleanup(() => {
    document.title = previousTitle;
  });

  return (
    <SettingsCtx.Provider value={{ bundle }}>
      <Show when={known()} fallback={<Navigate href="/settings/schedule" />}>
        <div class="settings">
          <nav class="settings-nav" aria-label={t().nav_label}>
            <For each={GROUPS}>
              {(group) => (
                <div class="settings-nav-group">
                  <div class="settings-nav-label kicker">{t().groups[group.key]}</div>
                  <For each={group.items}>
                    {(key) => (
                      <A
                        href={`/settings/${key}`}
                        class="settings-nav-link"
                        classList={{ active: section() === key }}
                        aria-current={section() === key ? "page" : undefined}
                      >
                        <SIcon name={ICONS[key]} size={16} />
                        <span class="label">{t().sections[key].title}</span>
                        <Show when={!isAdmin() && ADMIN_ONLY.includes(key)}>
                          <SIcon name="lock" size={12} class="nav-lock" />
                        </Show>
                      </A>
                    )}
                  </For>
                </div>
              )}
            </For>
          </nav>

          <div class="settings-main">
            <div class="settings-mobile-nav">
              <label class="visually-hidden" for="settings-section-select">
                {t().nav_label}
              </label>
              <span class="mobile-nav-icon" aria-hidden="true">
                <SIcon name={ICONS[section()]} size={16} />
              </span>
              <select id="settings-section-select" class="select" value={section()} onChange={(e) => navigate(`/settings/${e.currentTarget.value}`)}>
                <For each={GROUPS}>
                  {(group) => (
                    <optgroup label={t().groups[group.key]}>
                      <For each={group.items}>{(key) => <option value={key}>{t().sections[key].title}</option>}</For>
                    </optgroup>
                  )}
                </For>
              </select>
            </div>

            <header class="page-head settings-head">
              <div>
                <h1>{meta().title}</h1>
                <p class="lead">{meta().lead}</p>
              </div>
            </header>

            <Show when={!isAdmin() && section() !== "security"}>
              <div class="callout info">
                <SIcon name="lock" size={16} />
                <span>{t().read_only_banner}</span>
              </div>
            </Show>

            <Switch>
              <Match when={section() === "schedule"}>
                <ScheduleSection />
              </Match>
              <Match when={section() === "automation"}>
                <AutomationSection />
              </Match>
              <Match when={section() === "execution"}>
                <ExecutionSection />
              </Match>
              <Match when={section() === "risk"}>
                <RiskSection />
              </Match>
              <Match when={section() === "notifications"}>
                <NotificationsSection />
              </Match>
              <Match when={section() === "data"}>
                <DataSection />
              </Match>
              <Match when={section() === "display"}>
                <DisplaySection />
              </Match>
              <Match when={section() === "benchmarks"}>
                <BenchmarksSection />
              </Match>
              <Match when={section() === "account"}>
                <AccountSection />
              </Match>
              <Match when={section() === "users"}>
                <UsersSection />
              </Match>
              <Match when={section() === "security"}>
                <SecuritySection />
              </Match>
            </Switch>
          </div>
        </div>
      </Show>
      <LocalConfirmHost />
    </SettingsCtx.Provider>
  );
}
