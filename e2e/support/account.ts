// The first minute of a journey that needs an account: a person signs up at the stock app's own form, goes through its onboarding and makes a
// workspace. The same steps as the first lines of e2e/journeys/m2-change-password.spec.ts, kept here for the journeys that come after it (the
// older ones keep their own copy: moving them is a change of its own, and the pinned Plane, which only they run on, cannot be run in CI).
import { expect, type Page } from "@playwright/test";

export type NewAccount = { email: string; password: string; name: string; workspaceSlug: string };

// `quiet` is the page's `watchPending(...).quiet` (support/network.ts): what the page asked for on its way in is answered before the next
// navigation, because a `goto` made while the app is still redirecting itself is aborted by the browser (G-31).
export async function signUpAndMakeWorkspace(page: Page, who: NewAccount, quiet: () => Promise<void>): Promise<void> {
  await page.goto("/");
  await page.fill("input[name=email]", who.email);
  await page.getByRole("button", { name: "Continue" }).click();
  await expect(page.locator("input[type=password]").first()).toBeVisible();
  for (const input of await page.locator("input[type=password]").all()) await input.fill(who.password);
  await page.locator("button[type=submit]").click();
  await expect(page).toHaveURL(/\/onboarding\//);
  await page.fill("input[name=first_name]", who.name);
  await page.getByRole("button", { name: "Continue" }).click();
  await page.fill("input[name=name]", `${who.name} Workspace`);
  await page.fill("input[name=slug]", who.workspaceSlug);
  await page.getByRole("button", { name: "Just myself" }).click();
  await page.getByRole("button", { name: "Create workspace" }).click();
  // by the path alone: a sign-in page that carries the workspace's path in its query is not the workspace
  await expect.poll(() => new URL(page.url()).pathname).toMatch(new RegExp(`^/${who.workspaceSlug}/`));
  await quiet();
}
