// Journey M1 (issue #44's last acceptance criterion, the first journey of issue #45's suite): the stock Plane app, built from web/ and
// served by the Worker, signs a user up, takes them through onboarding, makes a workspace, a project and a work item, and lists it, and
// the browser's network log shows no 404 and no 5xx. It covers the screens of M1's exit: sign up, onboarding, workspace, project, work
// item, list (TRACEABILITY, "Parity matrix").
//
// What would make this pass without proving anything, and what stops it:
//   - a run aimed at the wrong target (the pinned Plane for a Thrust run, or the reverse): support/target.ts fails first, naming
//     TARGET GUARD; for a Thrust it also requires that CSRF enforcement is off (the stock app sends the token on its auth posts only);
//   - a build that serves no app, or whose assets are the fallback page (a missing hashed file is answered 200 text/html by
//     the single-page fallback, ADR-0011): every /assets/ response must be a file and not HTML, and there must be some
//     (support/network.ts);
//   - a journey that reached no API: the creation of the work item and the list that shows it must both be in the log, and
//     so must a minimum of /api/ responses;
//   - a 404 detector that cannot see, a journey that never looks at the server's list, and a journey that skips itself: the controls
//     in support/controls.ts, each of which scripts/e2e.sh runs and requires to FAIL, and to fail by saying why; and its count guard
//     (the report must say that every journey in e2e/journeys/ passed, none skipped).
//
// What is NOT asserted: that the app is pixel-faithful, or anything beyond the work item appearing in the list. Opening the work item
// calls one more route that is not built (`.../work-items/{id}/description-versions/`, G-30); that is outside this journey.
import { expect, test } from "@playwright/test";
import { control, installControl, STEP_LIST } from "../support/controls";
import { openJourney } from "../support/context";
import { judgeLog, watchResponses, type AllowedFailure } from "../support/network";
import { assertTarget, wantedTarget } from "../support/target";

// None: the pin answers each route this journey calls, and Thrust does as well. The first run on the pinned Plane decides whether
// this stays empty (issue #45); a route that the pin itself answers 404 or 5xx would be listed here with the reason.
const ALLOWED_FAILURES: AllowedFailure[] = [];

test("the stock app: sign up, onboard, make a workspace, a project and a work item, and list it", async ({ browser, baseURL }) => {
  // the control that the count guard of scripts/e2e.sh must catch: Playwright reports a skipped test as a pass
  test.skip(control === "skip", "E2E_CONTROL=skip");
  if (!baseURL) throw new Error("no baseURL: scripts/e2e.sh sets THRUST_BASE_URL");

  const stamp = Date.now().toString(36);
  const email = `journey-${stamp}@example.test`;
  const password = "Journey-Pass-Pa55word-Zebra-81";
  const workspaceSlug = `journey-${stamp}`;
  const title = `First journey work item ${stamp}`;

  // is this the target that was asked for? Before any click: a journey on the wrong target says nothing about either
  await assertTarget(baseURL, wantedTarget());

  const { context, page } = await openJourney(browser, baseURL, "m1-sign-up-to-work-item");
  const seen = watchResponses(page, baseURL);
  const controlled = await installControl(page);

  // The screen is walked inside a `try`, and the network log is judged before its outcome is: a route that is not mounted can break
  // the page as well as show in the log, and then the 404 is the finding, not the click that timed out after it.
  let walked: unknown;
  try {
    await test.step("sign up", async () => {
      await page.goto("/");
      await page.fill("input[name=email]", email);
      await page.getByRole("button", { name: "Continue" }).click();
      await expect(page.locator("input[type=password]").first()).toBeVisible();
      for (const input of await page.locator("input[type=password]").all()) await input.fill(password);
      await page.locator("button[type=submit]").click();
    });

    await test.step("onboard: the profile, then the workspace", async () => {
      await expect(page).toHaveURL(/\/onboarding\//);
      await page.fill("input[name=first_name]", "Journey User");
      await page.getByRole("button", { name: "Continue" }).click();
      await page.fill("input[name=name]", "Journey Workspace");
      await page.fill("input[name=slug]", workspaceSlug);
      await page.getByRole("button", { name: "Just myself" }).click();
      await page.getByRole("button", { name: "Create workspace" }).click();
    });

    await test.step("the workspace's home page, and the welcome tour that stands in front of it", async () => {
      await expect(page).toHaveURL(new RegExp(`/${workspaceSlug}/`));
      await page.getByRole("button", { name: "No thanks, I will explore it myself" }).click();
    });

    await test.step("create a project", async () => {
      // (the sidebar is rendered twice, so the entry is found twice: the first is the one on screen)
      await page.getByText("Add project", { exact: true }).first().click();
      await page.fill("input[name=name]", "Journey Project");
      await page.fill("input[name=identifier]", "JRN");
      await page.getByRole("button", { name: "Create project" }).click();
      await page.getByText("Open project", { exact: true }).click();
      await expect(page).toHaveURL(/\/projects\/[0-9a-f-]+\/issues\//);
    });

    await test.step("create a work item", async () => {
      await page.getByRole("button", { name: "Add work item" }).click();
      await page.fill("input[name=name]", title);
      await page.getByRole("button", { name: "Save", exact: true }).click();
      await expect(page.getByText(title).first(), "create a work item: the new work item is not on screen").toBeVisible();
    });

    await test.step(STEP_LIST, async () => {
      // The app puts the new work item in its own store from the answer to the creation, so that it is on screen proves little of the
      // server's list. A reload makes the page ask for the list again (S47), and the item must be in what comes back.
      await page.reload();
      await expect(page.getByText(title).first(), `${STEP_LIST}: the work item is not in the list the server answers after a reload`).toBeVisible();
      // everything the page asks for on the way has been answered
      await page.waitForLoadState("networkidle");
    });
  } catch (error) {
    walked = error;
  }

  await context.close(); // writes the HAR

  // --- the network log -----------------------------------------------------------------------------------------------------
  judgeLog(seen, walked, ALLOWED_FAILURES, { assets: 20, api: 30 });

  // the journey reached the API: a work item made, and a list of work items asked for after it
  const issues = /^\/api\/workspaces\/[^/]+\/projects\/[0-9a-f-]+\/issues\/$/;
  const created = seen.findIndex((s) => s.method === "POST" && issues.test(s.path) && s.status === 201);
  expect(created, "no 201 for the creation of a work item (S48)").toBeGreaterThanOrEqual(0);
  expect(
    seen.slice(created).some((s) => s.method === "GET" && issues.test(s.path) && s.status === 200),
    "no list of work items (S47) answered after the work item was made",
  ).toBe(true);

  // a control that did nothing is no control: with `empty-list` the browser must have emptied a list that was not empty
  if (control === "empty-list") expect(controlled.emptied(), "CONTROL INERT: the empty-list control emptied no work-item list").toBeGreaterThan(0);
});
