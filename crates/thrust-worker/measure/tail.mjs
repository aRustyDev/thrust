// Tail consumer for thrust-spike-auth: one KV entry per HTTP invocation, keyed by the request's `?run=` label, holding
// the CPU and wall time the platform recorded for it (milliseconds). `wrangler tail` does not print cpuTime and a Tail
// Worker's own console output is not streamed, so KV is the channel; scripts/measure-argon2.sh reads it back with
// `wrangler kv bulk get`. `outcome` says "exceededCpu" or "exceededMemory" when a limit was hit.
export default {
  async tail(events, env) {
    await Promise.all(
      events.map(async (e) => {
        const request = e.event && e.event.request;
        const response = e.event && e.event.response;
        const url = (request && request.url) || "";
        const m = /[?&]run=([^&]+)/.exec(url);
        const key = m ? decodeURIComponent(m[1]) : `other-${e.eventTimestamp}-${Math.random().toString(36).slice(2, 8)}`;
        const record = {
          ts: e.eventTimestamp,
          outcome: e.outcome,
          cpuTime: e.cpuTime,
          wallTime: e.wallTime,
          method: request && request.method,
          status: response && response.status,
          exceptions: (e.exceptions || []).length,
        };
        await env.CPU.put(key, JSON.stringify(record), { expirationTtl: 86400 });
      }),
    );
  },
};
