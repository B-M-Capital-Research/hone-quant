import { Navigate, Route, Router, useLocation, useNavigate } from "@solidjs/router";
import { type ParentProps, Show, createSignal, lazy, onMount } from "solid-js";
import { Shell } from "@/components/Shell";
import { Loading } from "@/components/ui";
import { onUnauthorized } from "@/lib/api";
import { BASE } from "@/lib/base";
import { loadMe, loadMeta, me, signedOut } from "@/lib/session";
import Login from "@/pages/Login";

const Dashboard = lazy(() => import("@/pages/Dashboard"));
const Plans = lazy(() => import("@/pages/Plans"));
const PlanDetail = lazy(() => import("@/pages/PlanDetail"));
const Trades = lazy(() => import("@/pages/Trades"));
const Strategy = lazy(() => import("@/pages/Strategy"));
const Universe = lazy(() => import("@/pages/Universe"));
const Backtests = lazy(() => import("@/pages/Backtests"));
const BacktestDetail = lazy(() => import("@/pages/BacktestDetail"));
const Performance = lazy(() => import("@/pages/Performance"));
const Notifications = lazy(() => import("@/pages/Notifications"));
const Audit = lazy(() => import("@/pages/Audit"));
const Settings = lazy(() => import("@/pages/Settings"));
const NotFound = lazy(() => import("@/pages/NotFound"));

/** Everything behind the login: waits for the session check, then renders the shell. */
function Protected(props: ParentProps) {
  const location = useLocation();
  return (
    <Show when={me() !== undefined} fallback={<Loading />}>
      <Show when={me()} fallback={<Navigate href={`/login?next=${encodeURIComponent(location.pathname + location.search)}`} />}>
        <Shell>{props.children}</Shell>
      </Show>
    </Show>
  );
}

function Root(props: ParentProps) {
  const navigate = useNavigate();
  const location = useLocation();
  const [ready, setReady] = createSignal(false);
  onMount(async () => {
    onUnauthorized(() => {
      signedOut();
      if (location.pathname !== "/login") {
        navigate(`/login?next=${encodeURIComponent(location.pathname + location.search)}`, { replace: true });
      }
    });
    await Promise.all([loadMeta(), loadMe()]);
    setReady(true);
  });
  return (
    <Show when={ready()} fallback={<Loading />}>
      {props.children}
    </Show>
  );
}

export function App() {
  return (
    <Router root={Root} base={BASE || undefined}>
      <Route path="/login" component={Login} />
      <Route path="/" component={Protected}>
        <Route path="/" component={Dashboard} />
        <Route path="/plans" component={Plans} />
        <Route path="/plans/:id" component={PlanDetail} />
        <Route path="/trades" component={Trades} />
        <Route path="/strategy" component={Strategy} />
        <Route path="/universe" component={Universe} />
        <Route path="/backtests" component={Backtests} />
        <Route path="/backtests/:id" component={BacktestDetail} />
        <Route path="/performance" component={Performance} />
        <Route path="/notifications" component={Notifications} />
        <Route path="/audit" component={Audit} />
        <Route path="/settings/:section?" component={Settings} />
        <Route path="*" component={NotFound} />
      </Route>
    </Router>
  );
}
