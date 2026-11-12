// The browser context every journey runs in, and the one place its recording is configured.
import type { Browser, BrowserContext, Page } from "@playwright/test";

export async function openJourney(browser: Browser, baseURL: string, name: string): Promise<{ context: BrowserContext; page: Page }> {
  const context = await browser.newContext({
    baseURL,
    // the app is a service-worker PWA; the journey is about the server's answers, so the worker stays out of it
    serviceWorkers: "block",
    viewport: { width: 1400, height: 950 },
    // the network log, kept: headers and statuses, not bodies. It is written when the context closes
    recordHar: { path: `${process.env.E2E_OUT ?? "."}/${name}.har`, content: "omit" },
  });
  const page = await context.newPage();
  page.setDefaultTimeout(30_000);
  return { context, page };
}
