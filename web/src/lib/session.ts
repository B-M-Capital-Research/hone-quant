/**
 * Global session state: who is signed in, server metadata, the market status shown in the
 * top bar, unread notifications, and a clock synchronised to the server (which may run a demo
 * clock offset), ticking once a second for countdowns.
 */
import { createSignal } from "solid-js";
import { api, ApiError } from "@/lib/api";
import type { MarketView, Meta, User } from "@/lib/types";
import { adoptServerDisplay } from "@/lib/prefs";

const [meta, setMeta] = createSignal<Meta | null>(null);
const [me, setMe] = createSignal<User | null | undefined>(undefined);
const [market, setMarket] = createSignal<MarketView | null>(null);
const [unread, setUnread] = createSignal(0);
const [offsetMs, setOffsetMs] = createSignal(0);
const [tick, setTick] = createSignal(Date.now());

/** Why the last sign-in check failed: not an administrator (403) or not verifiable (503). */
const [authProblem, setAuthProblem] = createSignal<"not_admin" | "unavailable" | null>(null);

export { meta, me, market, unread, setUnread, authProblem };

export const isAdmin = () => me()?.role === "admin";

/** Sign-in goes through hone-claw.com (administrators only). */
export const honeclawAuth = () => meta()?.auth?.mode === "honeclaw";

/** Milliseconds since epoch on the server's clock. */
export const serverNow = () => tick() + offsetMs();

let ticking = false;
export function startClock() {
  if (ticking) return;
  ticking = true;
  setInterval(() => setTick(Date.now()), 1000);
}

export async function loadMeta() {
  try {
    const value = await api.meta();
    setMeta(value);
    setOffsetMs(new Date(value.server_time).getTime() - Date.now());
    adoptServerDisplay(value.display?.timezone, value.display?.up_color);
  } catch {
    /* the shell shows a connection problem through other requests */
  }
}

export async function loadMe(): Promise<User | null> {
  try {
    const { user } = await api.me();
    setAuthProblem(null);
    setMe(user);
    return user;
  } catch (error) {
    if (error instanceof ApiError) {
      setAuthProblem(error.status === 403 ? "not_admin" : error.status === 503 ? "unavailable" : null);
    }
    setMe(null);
    return null;
  }
}

export function signedOut() {
  setMe(null);
}

export async function refreshMarket() {
  try {
    const value = await api.market();
    setMarket(value);
    setOffsetMs(new Date(value.now).getTime() - Date.now());
  } catch {
    /* keep the last known state */
  }
}

export async function refreshUnread() {
  try {
    const { unread: count } = await api.notifications({ unread: true, limit: 1 });
    setUnread(count);
  } catch {
    /* ignore */
  }
}
