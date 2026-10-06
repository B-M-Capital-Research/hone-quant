/**
 * Server-sent events. One EventSource for the whole app, reconnecting with backoff; pages
 * subscribe to the event types they care about and refetch from the API (the single source of
 * truth) when something changes.
 */
import { createSignal } from "solid-js";
import { withBase } from "@/lib/base";
import type { ServerEvent } from "@/lib/types";

type Handler = (event: ServerEvent) => void;

const handlers = new Set<Handler>();
const [connected, setConnected] = createSignal(false);
export { connected };

let source: EventSource | null = null;
let retry = 1000;
let timer: ReturnType<typeof setTimeout> | null = null;

const TYPES = ["hello", "quotes", "plan", "account", "notification", "backtest", "settings", "strategy", "universe", "resync"];

function dispatch(event: ServerEvent) {
  for (const handler of handlers) {
    try {
      handler(event);
    } catch (error) {
      console.error("event handler failed", error);
    }
  }
}

export function connectEvents() {
  if (source || typeof EventSource === "undefined") return;
  source = new EventSource(withBase("/api/events"), { withCredentials: true });
  source.onopen = () => {
    setConnected(true);
    retry = 1000;
  };
  for (const type of TYPES) {
    source.addEventListener(type, (message) => {
      let payload: any = { type };
      try {
        payload = JSON.parse((message as MessageEvent).data || "{}");
        if (!payload.type) payload.type = type;
      } catch {
        /* keep the bare type */
      }
      dispatch(payload as ServerEvent);
    });
  }
  source.onerror = () => {
    setConnected(false);
    source?.close();
    source = null;
    if (timer) clearTimeout(timer);
    timer = setTimeout(connectEvents, retry);
    retry = Math.min(retry * 2, 30_000);
  };
}

export function disconnectEvents() {
  source?.close();
  source = null;
  if (timer) clearTimeout(timer);
  setConnected(false);
}

/** Subscribes to server events; returns an unsubscribe function. */
export function onServerEvent(types: ServerEvent["type"][] | "all", handler: Handler): () => void {
  const wrapped: Handler = (event) => {
    if (types === "all" || types.includes(event.type) || event.type === "resync") handler(event);
  };
  handlers.add(wrapped);
  return () => handlers.delete(wrapped);
}
