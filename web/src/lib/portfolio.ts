/**
 * The portfolio context: the portfolios the signed-in user can see, the one the app works in
 * (remembered in this browser and sent with every API request), and whether the user has none
 * yet. The shell remounts the routed page whenever the selection changes, so pages simply read
 * `currentPortfolio()` / `canTrade()` and fetch as usual.
 */
import { batch, createSignal } from "solid-js";
import { api, setPortfolioContext } from "@/lib/api";
import { onServerEvent } from "@/lib/events";
import { eventConcerns, parsePortfolioId, resolveSelection } from "@/lib/portfolio-select";
import type { Portfolio, ServerEvent } from "@/lib/types";

const STORAGE_KEY = "hone-quant.portfolio";

function readStored(): number | null {
  try {
    return parsePortfolioId(localStorage.getItem(STORAGE_KEY));
  } catch {
    return null;
  }
}

function writeStored(id: number | null) {
  try {
    if (id === null) localStorage.removeItem(STORAGE_KEY);
    else localStorage.setItem(STORAGE_KEY, String(id));
  } catch {
    /* storage unavailable */
  }
}

/** Visible active portfolios, in the server's order (shared first, then by id). */
const [portfolios, setPortfolios] = createSignal<Portfolio[]>([]);
const [canCreate, setCanCreate] = createSignal(false);
const [defaultId, setDefaultId] = createSignal<number | null>(null);
const [selectedId, setSelectedId] = createSignal<number | null>(readStored());
/** The first load finished (successfully or not); pages wait for it so their first request carries the header. */
const [loaded, setLoaded] = createSignal(false);
const [listError, setListError] = createSignal<unknown>(null);

export { portfolios, canCreate, selectedId, loaded as portfoliosLoaded, listError as portfoliosError };

/** Ids the server refused as the context during this session; never selected automatically again. */
const rejected = new Set<number>();
let inflight: Promise<void> | null = null;
let generation = 0;
/** Bumped by local changes to the list, so an answer requested before one of them is not applied. */
let edits = 0;
let reloadTimer: ReturnType<typeof setTimeout> | null = null;

export const currentPortfolio = () => {
  const id = selectedId();
  return portfolios().find((p) => p.id === id && p.status === "active") ?? null;
};

/** The signed-in user may trade the current portfolio (plans, automation, restrictions, strategy, reset). */
export const canTrade = () => currentPortfolio()?.can_trade ?? false;

/** The list loaded and there is no active portfolio this user can see. */
export const noPortfolio = () => loaded() && listError() === null && !portfolios().some((p) => p.status === "active");

function adoptSelection(next: number | null) {
  if (next !== selectedId()) setSelectedId(next);
  writeStored(next);
}

/** Loads the visible portfolios and settles the selection. Concurrent calls share one request. */
export function loadPortfolios(): Promise<void> {
  if (inflight) return inflight;
  const gen = generation;
  const seen = edits;
  const run: Promise<void> = api
    .portfolios()
    .then(
      (res) => {
        if (gen !== generation) return;
        if (seen !== edits) {
          scheduleReload(0);
          return;
        }
        batch(() => {
          setPortfolios(res.portfolios.filter((p) => p.status === "active"));
          setCanCreate(res.can_create);
          setDefaultId(res.default_id);
          setListError(null);
          adoptSelection(resolveSelection(res.portfolios, selectedId(), res.default_id, rejected));
          setLoaded(true);
        });
      },
      (error) => {
        if (gen !== generation) return;
        batch(() => {
          setListError(error);
          // A selection the server refused is dropped, so requests fall back to the server's default.
          const current = selectedId();
          if (current !== null && rejected.has(current)) adoptSelection(null);
          setLoaded(true);
        });
      },
    )
    .finally(() => {
      if (inflight === run) inflight = null;
    });
  inflight = run;
  return run;
}

function scheduleReload(delay: number) {
  if (reloadTimer) return;
  reloadTimer = setTimeout(() => {
    reloadTimer = null;
    void loadPortfolios();
  }, delay);
}

/** Switches to a visible active portfolio. Returns false (and changes nothing) otherwise. */
export function selectPortfolio(id: number): boolean {
  if (!portfolios().some((p) => p.id === id && p.status === "active")) return false;
  rejected.delete(id);
  adoptSelection(id);
  return true;
}

/** Applies a portfolio returned by a create, rename or archive before the list reloads. */
export function upsertPortfolio(portfolio: Portfolio) {
  const list = portfolios();
  const next =
    portfolio.status !== "active"
      ? list.filter((p) => p.id !== portfolio.id)
      : list.some((p) => p.id === portfolio.id)
        ? list.map((p) => (p.id === portfolio.id ? portfolio : p))
        : [...list, portfolio];
  edits++;
  batch(() => {
    setPortfolios(next);
    if (selectedId() === portfolio.id && portfolio.status !== "active") {
      adoptSelection(resolveSelection(next, null, defaultId(), rejected));
    }
  });
  scheduleReload(800);
}

/** Forgets everything about the previous user (sign-out). */
export function resetPortfolios() {
  generation++;
  inflight = null;
  rejected.clear();
  if (reloadTimer) clearTimeout(reloadTimer);
  reloadTimer = null;
  batch(() => {
    setPortfolios([]);
    setCanCreate(false);
    setDefaultId(null);
    setListError(null);
    setLoaded(false);
    setSelectedId(readStored());
  });
}

/** Keeps names, modes and summaries fresh; returns an unsubscribe function. */
export function watchPortfolios(): () => void {
  const off = onServerEvent(["portfolios", "account", "settings", "strategy"], (event) => {
    if (event.type === "settings" && event.key !== "automation") return;
    scheduleReload(event.type === "portfolios" ? 0 : 1500);
  });
  return () => {
    off();
    if (reloadTimer) clearTimeout(reloadTimer);
    reloadTimer = null;
  };
}

/** `onServerEvent` for views of the current portfolio: events about other portfolios are ignored. */
export function onPortfolioEvent(types: ServerEvent["type"][], handler: (event: ServerEvent) => void): () => void {
  return onServerEvent(types, (event) => {
    if (eventConcerns(event, selectedId())) handler(event);
  });
}

setPortfolioContext({
  current: selectedId,
  notFound(id) {
    rejected.add(id);
    if (selectedId() !== id) return;
    writeStored(null);
    void loadPortfolios();
  },
  none() {
    // The list still shows portfolios: refresh it; the empty state follows once it is empty.
    if (portfolios().length) scheduleReload(0);
  },
});
