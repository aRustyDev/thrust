// The first journey (issue #44's last acceptance criterion; it becomes the first of issue #45's): the stock Plane app, built from
// web/ and served by the Worker, signs a user up, takes them through onboarding, makes a workspace, a project and a work item, and
// lists it, and the browser's network log shows no 404 and no 5xx.
//
// What would make this pass without proving anything, and what stops it:
//   - a target that enforces the CSRF token (the stock app sends it only on its auth posts): /_info must say it does not;
//   - a build that serves no app, or whose assets are the fallback page (a missing hashed file is answered 200 text/html by
//     the single-page fallback, ADR-0011): every /assets/ response must be a file and not HTML, and there must be some;
//   - a journey that reached no API: the creation of the work item and the list that shows it must both be in the log, and
//     so must a minimum of /api/ responses;
//   - a 404 detector that cannot see: with E2E_CONTROL=stickies-404 one route answers 404 and this spec must FAIL, saying so
//     (scripts/e2e.sh runs that as its control and requires exactly that failure).
//
// What is NOT asserted: that the app is pixel-faithful, or anything beyond the work item appearing in the list. Opening the work item
// calls one more route that is not built (`.../work-items/{id}/description-versions/`, G-30); that is outside this journey.
import { expect, test } from "@playwright/test";

type Seen = { method: string; path: string; status: number; type: string };

// Responses this journey may see that are 404 or 5xx. None: the pin answers each route it calls, and Thrust now does as well. A route
// that the pin itself answers 404 (research/0003 section 15.9) would be listed here with the reason, and this list is empty on purpose.
const ALLOWED_FAILURES: { method: string; pathPattern: RegExp; why: string }[] = [];

const control = process.env.E2E_CONTROL ?? "";
const adminToken = process.env.THRUST_ADMIN_TOKEN ?? "";

test("the stock app: sign up, onboard, make a workspace, a project and a work item, and list it", async ({ browser, baseURL }) => {
  const stamp = Date.now().toString(36);
  const email = `journey-${stamp}@example.test`;
  const password = "Journey-Pass-Pa55word-Zebra-81";
  const workspaceSlug = `journey-${stamp}`;
  const title = `First journey work item ${stamp}`;

  // what the target says it is: a CSRF check that is on would refuse the stock app's JSON writes for the wrong reason
  const info = await fetch(`${baseURL}/_info`, { headers: { "x-admin-token": adminToken } });
  expect(info.status, "/_info needs THRUST_ADMIN_TOKEN and a target started with test routes").toBe(200);
  const target = (await info.json()) as { csrf_enforce?: boolean; target?: string };
  expect(target.csrf_enforce, "the journey runs with CSRF enforcement off, the production default (Q-0050.8)").toBe(false);

  const context = await browser.newContext({
    baseURL,
    // the app is a service-worker PWA; the journey is about the server's answers, so the worker stays out of it
    serviceWorkers: "block",
    viewport: { width: 1400, height: 950 },
    // the network log, kept: headers and statuses, not bodies
    recordHar: { path: `${process.env.E2E_OUT ?? "."}/journey.har`, content: "omit" },
  });
  const page = await context.newPage();

  const seen: Seen[] = [];
  page.on("response", (r) => {
    const url = new URL(r.url());
    if (url.origin !== new URL(baseURL ?? "").origin) return;
    seen.push({ method: r.request().method(), path: url.pathname, status: r.status(), type: r.headers()["content-type"] ?? "" });
  });

  if (control === "stickies-404") {
    // the control: one route the journey calls answers 404, as if it were not mounted. The detector below must see it.
    await page.route("**/api/workspaces/*/stickies/**", (route) => route.fulfill({ status: 404, contentType: "application/json", body: "{}" }));
  }

  // sign up
  await page.goto("/");
  await page.fill("input[name=email]", email);
  await page.getByRole("button", { name: "Continue" }).click();
  await expect(page.locator("input[type=password]").first()).toBeVisible();
  for (const input of await page.locator("input[type=password]").all()) await input.fill(password);
  await page.locator("button[type=submit]").click();

  // onboarding: the profile, then the workspace
  await expect(page).toHaveURL(/\/onboarding\//);
  await page.fill("input[name=first_name]", "Journey User");
  await page.getByRole("button", { name: "Continue" }).click();
  await page.fill("input[name=name]", "Journey Workspace");
  await page.fill("input[name=slug]", workspaceSlug);
  await page.getByRole("button", { name: "Just myself" }).click();
  await page.getByRole("button", { name: "Create workspace" }).click();

  // the workspace's home page, and the welcome tour that stands in front of it
  await expect(page).toHaveURL(new RegExp(`/${workspaceSlug}/`));
  await page.getByRole("button", { name: "No thanks, I will explore it myself" }).click();

  // a project
  // (the sidebar is rendered twice, so the entry is found twice: the first is the one on screen)
  await page.getByText("Add project", { exact: true }).first().click();
  await page.fill("input[name=name]", "Journey Project");
  await page.fill("input[name=identifier]", "JRN");
  await page.getByRole("button", { name: "Create project" }).click();
  await page.getByText("Open project", { exact: true }).click();
  await expect(page).toHaveURL(/\/projects\/[0-9a-f-]+\/issues\//);

  // a work item, and the list that shows it
  await page.getByRole("button", { name: "Add work item" }).click();
  await page.fill("input[name=name]", title);
  await page.getByRole("button", { name: "Save", exact: true }).click();
  await expect(page.getByText(title).first()).toBeVisible();
  // The app puts the new work item in its own store from the answer to the creation, so that it is on screen proves little of the
  // server's list. A reload makes the page ask for the list again (S47), and the item must be in what comes back.
  await page.reload();
  await expect(page.getByText(title).first()).toBeVisible();
  // everything the page asks for on the way has been answered
  await page.waitForLoadState("networkidle");

  await context.close(); // writes the HAR

  // --- the network log -----------------------------------------------------------------------------------------------------
  const describe = (s: Seen) => `${s.method} ${s.path} -> ${s.status}`;

  const failures = seen.filter(
    (s) =>
      (s.status === 404 || s.status >= 500) &&
      !ALLOWED_FAILURES.some((a) => a.method === s.method && a.pathPattern.test(s.path)),
  );
  expect(failures.map((s) => `an unexpected ${s.status}: ${describe(s)}`), "the browser's network log must hold no 404 and no 5xx").toEqual([]);

  // the app's own files are files: a missing hashed asset is the fallback page with a 200 (ADR-0011), which is no 404 and so is looked
  // for. (A 304 is the browser asking again for a file it has, and being told it is the same: an answer about a file.)
  const assets = seen.filter((s) => s.path.startsWith("/assets/"));
  expect(assets.length, "the app loaded no files from /assets/: nothing is serving the build").toBeGreaterThan(20);
  expect(
    assets.filter((s) => (s.status !== 200 && s.status !== 304) || s.type.includes("text/html")).map(describe),
    "a file of the app was answered with the fallback page or an error",
  ).toEqual([]);

  // the journey reached the API: a work item made, and a list of work items asked for after it
  const api = seen.filter((s) => s.path.startsWith("/api/"));
  expect(api.length, "too few /api/ responses for a journey of this length").toBeGreaterThan(30);
  const issues = /^\/api\/workspaces\/[^/]+\/projects\/[0-9a-f-]+\/issues\/$/;
  const created = seen.findIndex((s) => s.method === "POST" && issues.test(s.path) && s.status === 201);
  expect(created, "no 201 for the creation of a work item (S48)").toBeGreaterThanOrEqual(0);
  expect(
    seen.slice(created).some((s) => s.method === "GET" && issues.test(s.path) && s.status === 200),
    "no list of work items (S47) answered after the work item was made",
  ).toBe(true);
});
