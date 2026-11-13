// The browser's network log for a journey, and the verdict on it. Every journey ends by judging its log the same way, so it lives here.
//
// The log is judged BEFORE the screen's own outcome: a route that is not mounted can break the page as well as show in the log, and
// then the 404 is the finding, not the click that timed out after it.
import { expect, type Page, type Request } from "@playwright/test";

export type Seen = { method: string; path: string; status: number; type: string };

// A response a journey may see that is a 404 or a 5xx, with the reason. Empty on purpose until the pin itself answers one
// (research/0003 section 15.9): then it is listed here, and Thrust is held to the same answer.
export type AllowedFailure = { method: string; pathPattern: RegExp; why: string };

export const describe = (s: Seen) => `${s.method} ${s.path} -> ${s.status}`;

// What the target answered, in order, for the target's own origin only.
export function watchResponses(page: Page, baseURL: string): Seen[] {
  const seen: Seen[] = [];
  const origin = new URL(baseURL).origin;
  page.on("response", (r) => {
    const url = new URL(r.url());
    if (url.origin !== origin) return;
    seen.push({ method: r.request().method(), path: url.pathname, status: r.status(), type: r.headers()["content-type"] ?? "" });
  });
  return seen;
}

// What the page has asked for and not yet had answered, from the moment this is called (call it before the first navigation). A
// `waitForLoadState("networkidle")` cannot stand in for it: it resolves at once on a page that is a single-page app, past its first
// load, because that document reached the state long ago. `quiet` resolves when nothing has been in flight for `ms`, and says what
// was when it is not so within `timeout`.
export function watchPending(page: Page): { quiet: (ms?: number, timeout?: number) => Promise<void> } {
  const pending = new Map<Request, string>();
  let lastChange = Date.now();
  page.on("request", (r) => {
    pending.set(r, `${r.method()} ${new URL(r.url()).pathname}`);
    lastChange = Date.now();
  });
  const done = (r: Request) => {
    pending.delete(r);
    lastChange = Date.now();
  };
  page.on("requestfinished", done);
  page.on("requestfailed", done);
  return {
    async quiet(ms = 500, timeout = 15_000) {
      const deadline = Date.now() + timeout;
      while (Date.now() < deadline) {
        if (pending.size === 0 && Date.now() - lastChange >= ms) return;
        await new Promise((resolve) => setTimeout(resolve, 50));
      }
      throw new Error(`the page was not quiet for ${ms} ms within ${timeout} ms; in flight: ${[...pending.values()].slice(0, 5).join(", ") || "nothing, but it kept asking"}`);
    },
  };
}

const firstLine = (e: unknown) => String(e instanceof Error ? e.message : e).split("\n")[0];

// `walked` is what stopped the screen, if anything. Throws, in this order:
//   1. the screen failed and the log has nothing to say about why: that failure is the finding;
//   2. a 404 or 5xx that is not allowed (naming the screen's failure too, when there was one);
//   3. the app's own files are not files: a missing hashed asset is the fallback page with a 200 (ADR-0011), which is no 404 and so is
//      looked for; a 304 is the browser asking again for a file it has, an answer about a file;
//   4. too few /api/ responses for the journey to have reached the API at all.
export function judgeLog(
  seen: Seen[],
  walked: unknown,
  allowed: AllowedFailure[],
  floors: { assets: number; api: number },
): void {
  const failures = seen.filter(
    (s) => (s.status === 404 || s.status >= 500) && !allowed.some((a) => a.method === s.method && a.pathPattern.test(s.path)),
  );
  if (walked !== undefined && failures.length === 0) throw walked;
  expect(
    failures.map((s) => `an unexpected ${s.status}: ${describe(s)}`),
    `the browser's network log must hold no 404 and no 5xx${walked !== undefined ? ` (the screen also failed: ${firstLine(walked)})` : ""}`,
  ).toEqual([]);

  const assets = seen.filter((s) => s.path.startsWith("/assets/"));
  expect(assets.length, "the app loaded too few files from /assets/: nothing is serving the build").toBeGreaterThan(floors.assets);
  expect(
    assets.filter((s) => (s.status !== 200 && s.status !== 304) || s.type.includes("text/html")).map(describe),
    "a file of the app was answered with the fallback page or an error",
  ).toEqual([]);

  const api = seen.filter((s) => s.path.startsWith("/api/"));
  expect(api.length, "too few /api/ responses for a journey of this length").toBeGreaterThan(floors.api);
}
