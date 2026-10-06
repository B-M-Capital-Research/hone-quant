/**
 * The URL prefix the app is served under: "" at the root, "/quant" behind hone-claw.com.
 * Set at build time (`HONE_QUANT_BASE_PATH`, see vite.config.ts); the server checks that it
 * serves the same prefix.
 */
export const BASE = import.meta.env.BASE_URL.replace(/\/+$/, "");

/** An absolute app path ("/api/x", "/hone-mark.svg") under the base path. */
export function withBase(path: string): string {
  return `${BASE}${path.startsWith("/") ? path : `/${path}`}`;
}
