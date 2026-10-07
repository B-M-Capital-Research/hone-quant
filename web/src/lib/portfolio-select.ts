/**
 * Pure rules behind the portfolio context (kept free of the API client so they are easy to test):
 * which portfolio to work in, the `?portfolio=<id>` link parameter, how an owner relates to the
 * signed-in user, and which server events concern the portfolio being shown.
 */
import type { Portfolio, ServerEvent } from "@/lib/types";

/** A positive integer id from storage or a URL, or null. */
export function parsePortfolioId(value: string | null | undefined): number | null {
  if (!value || !/^\d{1,15}$/.test(value.trim())) return null;
  const id = Number(value.trim());
  return Number.isSafeInteger(id) && id > 0 ? id : null;
}

/**
 * The portfolio to work in: the preferred one while it is still active and visible, otherwise
 * the server's default, otherwise the first active one; null when there is none. Ids the server
 * has rejected in this session are never picked again, so a stale selection cannot loop.
 */
export function resolveSelection(
  portfolios: readonly Pick<Portfolio, "id" | "status">[],
  preferred: number | null,
  defaultId: number | null,
  rejected: ReadonlySet<number> = new Set(),
): number | null {
  const usable = (id: number | null): id is number =>
    id !== null && !rejected.has(id) && portfolios.some((p) => p.id === id && p.status === "active");
  if (usable(preferred)) return preferred;
  if (usable(defaultId)) return defaultId;
  return portfolios.find((p) => p.status === "active" && !rejected.has(p.id))?.id ?? null;
}

/**
 * Splits the `portfolio` parameter off a location search string. Returns null when the parameter
 * is absent; otherwise the id it names (null when malformed) and the search string without it.
 */
export function takePortfolioParam(search: string): { id: number | null; search: string } | null {
  const params = new URLSearchParams(search);
  if (!params.has("portfolio")) return null;
  const id = parsePortfolioId(params.get("portfolio"));
  params.delete("portfolio");
  const rest = params.toString();
  return { id, search: rest ? `?${rest}` : "" };
}

export type OwnerKind = "shared" | "own" | "other";

/** Shared (managed by administrators), owned by the signed-in user, or owned by someone else. */
export function ownerKind(portfolio: Pick<Portfolio, "owner">, username: string | null | undefined): OwnerKind {
  if (portfolio.owner === null) return "shared";
  return username && portfolio.owner === username ? "own" : "other";
}

/**
 * Whether a server event concerns the portfolio being shown. Events without a portfolio (global
 * settings, the strategy library, resyncs) always do, and so does everything while the selection
 * is unknown.
 */
export function eventConcerns(event: ServerEvent, current: number | null): boolean {
  const id = "portfolio_id" in event ? event.portfolio_id : null;
  return id === null || id === undefined || current === null || id === current;
}
