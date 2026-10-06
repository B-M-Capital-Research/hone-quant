// hone-quant-proxy: serves hone-claw.com/quant from hone-quant on honeclaw's origin.
//
// Route: `hone-claw.com/quant*`, one pattern. Cloudflare matches the query string as part of the
// URL, so only a trailing wildcard also covers `/quant?lang=en`; paths that merely share the
// prefix (`/quantum`) are handed back to the zone untouched.
//
// The origin's Caddy sends /quant and /quant/* to hone-quant, which answers 404 to every request
// without X-Hone-Quant-Origin-Token. Signing in is hone-quant's job: it checks the browser's
// hone_web_session cookie against honeclaw's /api/public/auth/me and admits hone-claw.com
// administrators only. This Worker adds the origin token and forwards that one cookie; no other
// cookie of the hone-claw.com site reaches hone-quant.
//
// Configuration: QUANT_ORIGIN_URL (var, https://<origin-host>) and QUANT_ORIGIN_TOKEN
// (secret, HONE_QUANT_ORIGIN_TOKEN from /etc/hone-quant/runtime.env on the origin).

// Module-private on purpose: workerd treats every named export as an entrypoint and refuses to
// start on exports that are not functions or handlers. Only functions are exported (for tests).
const BASE_PATH = "/quant";
const ORIGIN_TOKEN_HEADER = "X-Hone-Quant-Origin-Token";
const FORWARDED_COOKIES = new Set(["hone_web_session"]);
const ASSET_PREFIX = `${BASE_PATH}/assets/`;
const MIN_TOKEN_LENGTH = 32;
// Never cache anything but the content-hashed bundles (any negative TTL means "do not cache").
const NO_EDGE_CACHE = { cacheTtlByStatus: { "100-599": -1 } };

export function isQuantPath(pathname) {
  return pathname === BASE_PATH || pathname.startsWith(`${BASE_PATH}/`);
}

/** The origin as a bare https URL (no credentials, path, query or fragment), or null. */
export function originUrl(raw) {
  let url;
  try {
    url = new URL(String(raw ?? "").trim());
  } catch {
    return null;
  }
  const bare =
    url.protocol === "https:" &&
    url.username === "" &&
    url.password === "" &&
    url.pathname === "/" &&
    url.search === "" &&
    url.hash === "";
  return bare ? url : null;
}

export function validToken(raw) {
  const token = String(raw ?? "").trim();
  return token.length >= MIN_TOKEN_LENGTH && /^[\x21-\x7e]+$/.test(token) ? token : null;
}

/** Only the cookies hone-quant needs, as sent by the browser; null when there are none. */
export function forwardedCookies(header) {
  if (!header) return null;
  const kept = [];
  for (const part of header.split(";")) {
    const pair = part.trim();
    const separator = pair.indexOf("=");
    if (separator <= 0) continue;
    if (FORWARDED_COOKIES.has(pair.slice(0, separator).trim())) kept.push(pair);
  }
  return kept.length > 0 ? kept.join("; ") : null;
}

export function buildOriginRequest(request, origin, token) {
  const incoming = new URL(request.url);
  // Assigning pathname/search (rather than resolving a string) can never change the host.
  const target = new URL(origin);
  target.pathname = incoming.pathname;
  target.search = incoming.search;

  const headers = new Headers(request.headers);
  headers.delete("host");
  headers.delete("cookie");
  headers.delete(ORIGIN_TOKEN_HEADER);
  const cookies = forwardedCookies(request.headers.get("cookie"));
  if (cookies) headers.set("cookie", cookies);
  headers.set(ORIGIN_TOKEN_HEADER, token);

  const init = { method: request.method, headers, redirect: "manual" };
  if (request.method !== "GET" && request.method !== "HEAD") init.body = request.body;
  return new Request(target.toString(), init);
}

function failure(status, code) {
  return new Response(JSON.stringify({ error: code }), {
    status,
    headers: {
      "content-type": "application/json; charset=utf-8",
      "cache-control": "no-store",
      "x-content-type-options": "nosniff",
    },
  });
}

export async function handleRequest(request, env) {
  const url = new URL(request.url);
  if (!isQuantPath(url.pathname)) return fetch(request);
  if (url.protocol === "http:") {
    url.protocol = "https:";
    return Response.redirect(url.toString(), 301);
  }

  const origin = originUrl(env.QUANT_ORIGIN_URL);
  const token = validToken(env.QUANT_ORIGIN_TOKEN);
  if (!origin || !token) return failure(503, "hone_quant_proxy_not_configured");

  const cacheable = url.pathname.startsWith(ASSET_PREFIX);
  try {
    return await fetch(
      buildOriginRequest(request, origin, token),
      cacheable ? undefined : { cf: NO_EDGE_CACHE },
    );
  } catch {
    return failure(502, "hone_quant_origin_unreachable");
  }
}

export default {
  fetch(request, env) {
    return handleRequest(request, env);
  },
};
