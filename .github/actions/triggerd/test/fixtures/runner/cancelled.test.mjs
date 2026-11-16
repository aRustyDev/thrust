import test from "node:test";

const controller = new AbortController();
controller.abort();
test("intentional cancellation", { signal: controller.signal }, () => {});
