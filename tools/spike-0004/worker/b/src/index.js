// Second Worker for spike 0004: reached ONLY through a service binding from the edge
// Worker (workers_dev and preview_urls are off, and no route/custom domain is attached).
//
// Exposes the same compute() both as an HTTP-style fetch handler and as an RPC method,
// so the harness can compare binding-fetch and binding-RPC against in-process.

import { WorkerEntrypoint } from "cloudflare:workers";
import { compute } from "../../shared/logic.js";

export class Compute extends WorkerEntrypoint {
  // HTTP style: env.B.fetch(request)
  async fetch(request) {
    const url = new URL(request.url);
    // WebSocket upgrade forwarded by the edge Worker through the service binding (edge -> collab-style Worker).
    // Replies are prefixed "b-echo:" so a test can tell B answered, not the edge.
    if ((request.headers.get("upgrade") || "").toLowerCase() === "websocket") {
      const [client, server] = Object.values(new WebSocketPair());
      server.binaryType = "arraybuffer";
      server.accept();
      server.send(JSON.stringify({ hello: "spike-0004-b", via: "service-binding-ws" }));
      server.addEventListener("message", (ev) => {
        server.send(typeof ev.data === "string" ? `b-echo:${ev.data}` : ev.data);
      });
      return new Response(null, { status: 101, webSocket: client });
    }
    const out = compute(url.searchParams.get("n"));
    return Response.json({ worker: "spike-0004-b", via: "fetch", ...out });
  }

  // RPC style: env.B.compute(n)
  async compute(n) {
    return { worker: "spike-0004-b", via: "rpc", ...compute(n) };
  }
}

// A default export is required for a Worker module that is also the target of a
// service binding to the default entrypoint; it refuses direct traffic.
export default {
  async fetch() {
    return new Response("spike-0004-b has no public surface", { status: 404 });
  },
};
