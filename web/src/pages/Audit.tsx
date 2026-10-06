/**
 * /audit — the append-only audit log (`?tab=log`, default; `actor`, `action`, `entity_type`,
 * `entity_id`) and scheduler job runs (`?tab=jobs&job=plan`).
 */
import "@/styles/activity.css";
import { useSearchParams } from "@solidjs/router";
import { Match, Switch, createMemo } from "solid-js";
import { Icon } from "@/components/Icon";
import { auditText } from "@/i18n/audit";
import { type AuditFilters, AuditLog } from "./activity/audit-log";
import { LiveBadge, TabBar } from "./activity/components";
import { JobsTab } from "./activity/jobs";
import { qstr } from "./activity/util";

type AuditTab = "log" | "jobs";

export default function Audit() {
  const a = auditText;
  const [params, setParams] = useSearchParams();
  const tab = (): AuditTab => (qstr(params.tab) === "jobs" ? "jobs" : "log");
  const filters = createMemo<AuditFilters>(() => ({
    actor: qstr(params.actor),
    action: qstr(params.action),
    entity_type: qstr(params.entity_type),
    entity_id: qstr(params.entity_id),
  }));

  return (
    <div class="act-page">
      <header class="page-head">
        <div class="act-head-text">
          <h1>{a().title}</h1>
          <p class="lead">{a().lead}</p>
        </div>
        <span class="spacer" />
        <div class="act-head-actions">
          <LiveBadge />
        </div>
      </header>

      <div class="callout aud-note">
        <Icon name="shield" size={17} />
        <div>
          <strong>{a().append_only_title}</strong>
          <p>{a().append_only}</p>
        </div>
      </div>

      <TabBar
        label={a().title}
        value={tab()}
        onChange={(id) => setParams({ tab: id === "log" ? "" : id })}
        tabs={[
          { id: "log", label: a().tabs.log },
          { id: "jobs", label: a().tabs.jobs },
        ]}
      />

      <div class="act-tabpanel" role="tabpanel">
        <Switch>
          <Match when={tab() === "log"}>
            <AuditLog filters={filters()} setFilters={(patch) => setParams(patch, { replace: true })} />
          </Match>
          <Match when={tab() === "jobs"}>
            <JobsTab job={qstr(params.job)} setJob={(job) => setParams({ job }, { replace: true })} />
          </Match>
        </Switch>
      </div>
    </div>
  );
}
