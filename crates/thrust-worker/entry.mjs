// JS SHIM (bounded exception per ADR-0006 round 22): re-exports the Rust Worker unchanged and adds
// the one thing workers-rs 0.8.7 cannot define: a Workflow class (`WorkflowEntrypoint` is a JS class).
//
// Shim contract (what the Rust side may rely on):
//   * trigger : Rust calls `env.WORKFLOW.create({ params })` through wasm-bindgen (see probes.rs)
//   * input   : `event.payload` is the JSON given as `params`
//   * logic   : NONE here. The class only sequences `step.do` calls and calls BACK into the Rust
//               Worker through the `SELF` service binding. Business logic stays in Rust.
//   * exit    : replace this file with a Rust-defined Workflow when workers-rs gains one, or when a
//               Rust `WorkflowEntrypoint` can be exported through wasm-bindgen (unverified).
import { WorkflowEntrypoint } from "cloudflare:workers";
import rust from "./build/worker/shim.mjs";

export * from "./build/worker/shim.mjs";
export default rust;

export class ThrustWorkflow extends WorkflowEntrypoint {
  async run(event, step) {
    const doubled = await step.do("double", async () => event.payload.n * 2);
    const health = await step.do("callback-into-rust-worker", async () => {
      const r = await this.env.SELF.fetch("https://self.internal/healthz");
      return { status: r.status, body: await r.text() };
    });
    return { doubled, health };
  }
}
