import { afterEach, beforeEach, describe, expect, mock, test } from "bun:test";
import * as module from "../src/index.js";
import worker, { forwardedCookies, isQuantPath, originUrl, validToken } from "../src/index.js";

const ORIGIN_TOKEN_HEADER = "X-Hone-Quant-Origin-Token";
const TOKEN = "a".repeat(64);
const ENV = { QUANT_ORIGIN_URL: "https://origin.example.com", QUANT_ORIGIN_TOKEN: TOKEN };

let calls;
const realFetch = globalThis.fetch;
function upstream(respond = () => new Response("upstream", { status: 200 })) {
  calls = [];
  globalThis.fetch = mock(async (input, init) => {
    calls.push({ input, init });
    return respond(input, init);
  });
}
beforeEach(() => upstream());
afterEach(() => {
  globalThis.fetch = realFetch;
});

const call = (url, init = {}, env = ENV) => worker.fetch(new Request(url, init), env);

describe("path matching", () => {
  test("only /quant and /quant/... belong to hone-quant", () => {
    for (const p of ["/quant", "/quant/", "/quant/api/health", "/quant/assets/index-x.js"]) {
      expect(isQuantPath(p)).toBe(true);
    }
    for (const p of ["/", "/quantum", "/quant-desk", "/QUANT", "/api/quant", "/quan"]) {
      expect(isQuantPath(p)).toBe(false);
    }
  });

  test("other paths under the route go back to the zone untouched, without the token", async () => {
    const request = new Request("https://hone-claw.com/quantum?x=1", {
      headers: { cookie: "hone_web_session=s; other=1" },
    });
    await worker.fetch(request, ENV);
    expect(calls).toHaveLength(1);
    expect(calls[0].input).toBe(request);
    expect(calls[0].init).toBeUndefined();
    expect(request.headers.get(ORIGIN_TOKEN_HEADER)).toBeNull();
  });
});

describe("configuration", () => {
  test("the origin must be a bare https URL", () => {
    expect(originUrl("https://origin.example.com")?.host).toBe("origin.example.com");
    expect(originUrl("https://origin.example.com/")?.host).toBe("origin.example.com");
    for (const bad of [
      undefined,
      "",
      "not a url",
      "http://origin.example.com",
      "https://origin.example.com/quant",
      "https://user:pw@origin.example.com",
      "https://origin.example.com/?x=1",
    ]) {
      expect(originUrl(bad)).toBeNull();
    }
  });

  test("the token must be at least 32 printable characters", () => {
    expect(validToken(TOKEN)).toBe(TOKEN);
    expect(validToken(` ${TOKEN}\n`)).toBe(TOKEN);
    expect(validToken("short")).toBeNull();
    expect(validToken(`${"a".repeat(40)} b`)).toBeNull();
    expect(validToken(undefined)).toBeNull();
  });

  test("a missing or weak configuration fails closed without contacting the origin", async () => {
    for (const env of [
      {},
      { ...ENV, QUANT_ORIGIN_TOKEN: "short" },
      { ...ENV, QUANT_ORIGIN_URL: "http://origin.example.com" },
    ]) {
      const response = await call("https://hone-claw.com/quant/api/health", {}, env);
      expect(response.status).toBe(503);
      expect(response.headers.get("cache-control")).toBe("no-store");
      expect(await response.json()).toEqual({ error: "hone_quant_proxy_not_configured" });
    }
    expect(calls).toHaveLength(0);
  });
});

describe("proxying", () => {
  test("GET keeps path and query, adds the token and forwards only the honeclaw session", async () => {
    await call("https://hone-claw.com/quant/api/auth/me?lang=en&x=%2F", {
      headers: {
        cookie: "_ga=1; hone_web_session=abc.def; hone_web_session_x=no; __cf_bm=z",
        [ORIGIN_TOKEN_HEADER]: "forged-by-the-client",
        accept: "application/json",
      },
    });
    expect(calls).toHaveLength(1);
    const sent = calls[0].input;
    expect(sent.url).toBe("https://origin.example.com/quant/api/auth/me?lang=en&x=%2F");
    expect(sent.method).toBe("GET");
    expect(sent.redirect).toBe("manual");
    expect(sent.headers.get(ORIGIN_TOKEN_HEADER)).toBe(TOKEN);
    expect(sent.headers.get("cookie")).toBe("hone_web_session=abc.def");
    expect(sent.headers.get("accept")).toBe("application/json");
    expect(calls[0].init).toEqual({ cf: { cacheTtlByStatus: { "100-599": -1 } } });
  });

  test("no cookie header is sent when the browser has no honeclaw session", async () => {
    await call("https://hone-claw.com/quant/", { headers: { cookie: "_ga=1" } });
    expect(calls[0].input.headers.get("cookie")).toBeNull();
  });

  test("POST keeps method, body and the CSRF headers hone-quant checks", async () => {
    await call("https://hone-claw.com/quant/api/plans/7/approve", {
      method: "POST",
      headers: {
        "content-type": "application/json",
        origin: "https://hone-claw.com",
        "sec-fetch-site": "same-origin",
        "x-hone-quant-action": "1",
        cookie: "hone_web_session=abc",
      },
      body: JSON.stringify({ note: "ok" }),
    });
    const sent = calls[0].input;
    expect(sent.method).toBe("POST");
    expect(await sent.text()).toBe('{"note":"ok"}');
    expect(sent.headers.get("origin")).toBe("https://hone-claw.com");
    expect(sent.headers.get("sec-fetch-site")).toBe("same-origin");
    expect(sent.headers.get("x-hone-quant-action")).toBe("1");
    expect(sent.headers.get("content-type")).toBe("application/json");
  });

  test("the bare /quant is proxied too and hone-quant's relative redirect passes through", async () => {
    upstream(() => new Response(null, { status: 308, headers: { location: "/quant/?lang=en" } }));
    const response = await call("https://hone-claw.com/quant?lang=en");
    expect(calls[0].input.url).toBe("https://origin.example.com/quant?lang=en");
    expect(response.status).toBe(308);
    expect(response.headers.get("location")).toBe("/quant/?lang=en");
  });

  test("responses (status, headers, body) are passed through unchanged", async () => {
    upstream(
      () =>
        new Response('{"error":"unauthorized"}', {
          status: 401,
          headers: { "content-type": "application/json", "x-request-id": "r1" },
        }),
    );
    const response = await call("https://hone-claw.com/quant/api/dashboard");
    expect(response.status).toBe(401);
    expect(response.headers.get("x-request-id")).toBe("r1");
    expect(await response.text()).toBe('{"error":"unauthorized"}');
  });

  test("only the content-hashed bundles may be cached at the edge", async () => {
    await call("https://hone-claw.com/quant/assets/index-CmXNMzML.js");
    expect(calls[0].init).toBeUndefined();
    await call("https://hone-claw.com/quant/");
    expect(calls[1].init).toEqual({ cf: { cacheTtlByStatus: { "100-599": -1 } } });
  });

  test("an unreachable origin is a 502 that is never cached", async () => {
    upstream(() => {
      throw new TypeError("network down");
    });
    const response = await call("https://hone-claw.com/quant/api/health");
    expect(response.status).toBe(502);
    expect(response.headers.get("cache-control")).toBe("no-store");
    expect(await response.json()).toEqual({ error: "hone_quant_origin_unreachable" });
  });

  test("plain http is redirected to https before anything is proxied", async () => {
    const response = await call("http://hone-claw.com/quant/plans?x=1");
    expect(response.status).toBe(301);
    expect(response.headers.get("location")).toBe("https://hone-claw.com/quant/plans?x=1");
    expect(calls).toHaveLength(0);
  });
});

describe("module shape", () => {
  test("named exports are functions only (workerd rejects any other export at start-up)", () => {
    for (const [name, value] of Object.entries(module)) {
      if (name === "default") continue;
      expect(typeof value).toBe("function");
    }
    expect(typeof worker.fetch).toBe("function");
  });
});

describe("cookie filtering", () => {
  test("keeps exact names only, in the browser's order", () => {
    expect(forwardedCookies(null)).toBeNull();
    expect(forwardedCookies("")).toBeNull();
    expect(forwardedCookies("a=1; b=2")).toBeNull();
    expect(forwardedCookies("hone_web_session=x")).toBe("hone_web_session=x");
    expect(forwardedCookies(" a=1 ;hone_web_session=x;hone_web_session=y ")).toBe(
      "hone_web_session=x; hone_web_session=y",
    );
    expect(forwardedCookies("hone_web_sessionx=1; xhone_web_session=2; =3")).toBeNull();
  });
});
