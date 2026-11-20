// Stand-ins for the modules that exist only inside workerd, so that the Worker's wasm32 unit tests can load in Node (G-6, stage 8 of the Test
// phase). Loaded by scripts/wasm-test-runner.sh through NODE_OPTIONS (`--require`); nothing else uses it.
//
// Why it exists: the Worker links all of the `worker` crate, and four of that crate's JavaScript snippets import a module that only workerd
// provides (worker-sys 0.9.0: types/tracing.rs, types/socket.rs, types/email.rs; worker 0.9.0: bindings/email.rs). `wasm-bindgen-test-runner`
// runs the test binary in Node, which stops at the first of them: `Cannot find module 'cloudflare:workers'`, and with a CommonJS-only patch
// `ERR_UNSUPPORTED_ESM_URL_SCHEME ... 'cloudflare:'` (the snippets are ES modules). Measured 2026-11-20 on Node 26.9.0, wasm-bindgen 0.2.130; the
// public tools were looked at first and none answers these modules for Node (the phase document, stage 8, has the survey).
//
// What it does: `module.registerHooks` (Node 22.15 and later; CI pins 22.22.0 in web/.node-version) covers `require` and `import` alike. Each module
// below is answered with one that exports the single name the Worker asks of it. Three of them as `undefined`: no unit test calls them, and a call
// that reached one would fail at the call, which is what a test of the glue should see. `tracing` is the exception, because the entry points
// DO ask it: the code `#[event]` generates wraps every request and every queue batch in `worker::observability::in_current_span`, which calls
// `tracing.getActiveSpan()` (worker-sys 0.9.0, types/tracing.rs:34 documents the answer outside an invocation: `undefined`, no active span). So
// `tracing` is an object with that one method and that one answer; `startSpan` and the rest are absent and fail at the call, as above. (The first run of
// the entry-point tests found it: "Cannot read properties of undefined (reading 'getActiveSpan')".)
// A `cloudflare:` module that is not listed is refused with the name of this file, so that a new import in a new `worker` version fails by saying
// what to add; a new NAME from a listed module is refused by Node itself ("does not provide an export named").
//
// One global, too: `FixedLengthStream`, which `worker::http::response` pipes every response body of a known size through (worker 0.9.0,
// http/response.rs:52; workerd emits Content-Length only for such a body, and errors the response if the body is not exactly that long). A
// request that gets any answer with a body reaches it: the first run of the fetch test hit "FixedLengthStream is not defined" on `/healthz`.
// The stand-in is a TransformStream that counts the bytes and errors on too many or too few, the one behaviour the crate relies on (and its own
// comment states), so a length that did not add up would still be a 500 here. It is defined only when the runtime has none.
const { registerHooks } = require('node:module');

globalThis.FixedLengthStream ??= class FixedLengthStream extends TransformStream {
  constructor(length) {
    const expected = BigInt(length); // a number, or a BigInt for 4 GiB and up, as worker-sys passes either
    let seen = 0n;
    const mismatch = () => new TypeError(`fixed length stream had different length than expected (expected ${expected}, got ${seen})`);
    super({
      transform(chunk, controller) {
        seen += BigInt(chunk.byteLength);
        if (seen > expected) throw mismatch();
        controller.enqueue(chunk);
      },
      flush() {
        if (seen !== expected) throw mismatch();
      },
    });
  }
};

const undefinedExports = (...names) => names.map((name) => `export const ${name} = undefined;`).join('\n');

const stubs = {
  'cloudflare:workers': 'export const tracing = { getActiveSpan() { return undefined; } };',
  'cloudflare:sockets': undefinedExports('connect'),
  'cloudflare:node': undefinedExports('handleAsNodeConnection'),
  'cloudflare:email': undefinedExports('EmailMessage'),
};

registerHooks({
  resolve(specifier, context, nextResolve) {
    if (Object.hasOwn(stubs, specifier)) return { url: specifier, format: 'module', shortCircuit: true };
    if (specifier.startsWith('cloudflare:')) {
      throw new Error(`${specifier} is a workerd module the wasm32 tests have no stand-in for: add it to scripts/lib/workerd-stubs.cjs`);
    }
    return nextResolve(specifier, context);
  },
  load(url, context, nextLoad) {
    if (Object.hasOwn(stubs, url)) {
      return { format: 'module', source: stubs[url], shortCircuit: true };
    }
    return nextLoad(url, context);
  },
});
