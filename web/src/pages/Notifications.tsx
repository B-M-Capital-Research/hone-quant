/**
 * /notifications — the inbox (`?tab=inbox`, default; `unread=1`, `category=risk`) and the
 * reminders (`?tab=reminders`).
 */
import "@/styles/activity.css";
import { useSearchParams } from "@solidjs/router";
import { Match, Switch } from "solid-js";
import { notificationsText } from "@/i18n/notifications";
import { unread } from "@/lib/session";
import { LiveBadge, TabBar } from "./activity/components";
import { Inbox } from "./activity/inbox";
import { Reminders } from "./activity/reminders";
import { qstr } from "./activity/util";

type NotificationsTab = "inbox" | "reminders";

export default function Notifications() {
  const n = notificationsText;
  const [params, setParams] = useSearchParams();
  const tab = (): NotificationsTab => (qstr(params.tab) === "reminders" ? "reminders" : "inbox");
  const unreadOnly = () => qstr(params.unread) === "1";
  const category = () => qstr(params.category);

  return (
    <div class="act-page">
      <header class="page-head">
        <div class="act-head-text">
          <h1>{n().title}</h1>
          <p class="lead">{n().lead}</p>
        </div>
        <span class="spacer" />
        <div class="act-head-actions">
          <LiveBadge />
        </div>
      </header>

      <TabBar
        label={n().title}
        value={tab()}
        onChange={(id) => setParams({ tab: id === "inbox" ? "" : id })}
        tabs={[
          { id: "inbox", label: n().tabs.inbox, count: unread() > 0 ? unread() : null, attention: true },
          { id: "reminders", label: n().tabs.reminders },
        ]}
      />

      <div class="act-tabpanel" role="tabpanel">
        <Switch>
          <Match when={tab() === "inbox"}>
            <Inbox
              unreadOnly={unreadOnly()}
              category={category()}
              onFilter={(patch) =>
                setParams(
                  {
                    unread: patch.unread === undefined ? params.unread : patch.unread ? "1" : "",
                    category: patch.category === undefined ? params.category : patch.category,
                  },
                  { replace: true },
                )
              }
            />
          </Match>
          <Match when={tab() === "reminders"}>
            <Reminders />
          </Match>
        </Switch>
      </div>
    </div>
  );
}
