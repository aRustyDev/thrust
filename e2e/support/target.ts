// Which target a journey runs against, and the check that it IS that target (issue #45: every journey runs on the stock app against
// the pinned Plane, the UI oracle, and against Thrust; a run aimed at the wrong one proves nothing about either).
//
// scripts/e2e.sh sets E2E_TARGET (default `thrust`). `assertTarget` runs before any click and fails with a message that starts
// `TARGET GUARD (<wanted>)`; scripts/e2e.sh's wrong-target control greps for exactly that, so the words are part of the contract.
//
// Why the checks are not "GET /api/instances/ has Plane's shape": Thrust serves it in the pin's shape on purpose (S01), so it cannot
// tell the two apart. And `/_info` is read by its BODY, never its status: behind a proxy or a single-page fallback a path that does not
// exist is a 200 text/html (ADR-0011), and a check on the status alone would call that a Thrust.
export type Target = "thrust" | "plane";

const adminToken = process.env.THRUST_ADMIN_TOKEN ?? "";

export function wantedTarget(): Target {
  const wanted = process.env.E2E_TARGET ?? "thrust";
  if (wanted !== "thrust" && wanted !== "plane") throw new Error(`E2E_TARGET is '${wanted}': it is thrust or plane`);
  return wanted;
}

type Probe = { status: number; type: string; text: string; json: unknown };

async function probe(url: string, headers: Record<string, string> = {}): Promise<Probe> {
  const response = await fetch(url, { headers, redirect: "manual" });
  const text = await response.text();
  let json: unknown;
  try {
    json = JSON.parse(text);
  } catch {
    json = undefined;
  }
  return { status: response.status, type: response.headers.get("content-type") ?? "(no type)", text, json };
}

const isObject = (value: unknown): value is Record<string, unknown> => typeof value === "object" && value !== null && !Array.isArray(value);

const guard = (wanted: Target, why: string) => new Error(`TARGET GUARD (${wanted}): ${why}`);

export async function assertTarget(baseURL: string, wanted: Target): Promise<void> {
  const info = await probe(`${baseURL}/_info`, { "x-admin-token": adminToken });
  // what a Thrust started with test routes says about itself: a JSON object with a `target` (scripts/contract.sh reads the same)
  const ours = info.status === 200 && isObject(info.json) && typeof info.json.target === "string" ? info.json : undefined;

  if (wanted === "thrust") {
    if (!ours) {
      throw guard(
        wanted,
        `not a Thrust target: GET /_info answered ${info.status} ${info.type}, not the JSON that a Thrust started with test routes gives ` +
          `(is THRUST_ADMIN_TOKEN set? is this the pinned Plane?)`,
      );
    }
    if (ours.csrf_enforce !== false) {
      throw guard(wanted, `/_info says csrf_enforce is ${String(ours.csrf_enforce)}; the journey runs with CSRF enforcement off, the production default (Q-0050.8)`);
    }
    return;
  }

  if (ours) throw guard(wanted, "not the pinned Plane: GET /_info is a Thrust answer");
  // the oracle runs with ENABLE_DRF_SPECTACULAR=1, which is why its api healthcheck can read this route
  const schema = await probe(`${baseURL}/api/schema/`);
  if (schema.status !== 200 || !/^\s*"?openapi"?\s*:/m.test(schema.text)) {
    throw guard(wanted, `not the pinned Plane: GET /api/schema/ answered ${schema.status} ${schema.type}, not an OpenAPI document`);
  }
}
