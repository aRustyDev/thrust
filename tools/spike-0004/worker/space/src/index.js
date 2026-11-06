// Spike 0004 stand-in for the public `space` Worker (ADR-0022): its OWN Worker on its OWN public hostname, with a
// host-wide Access BYPASS, read-only, and NO authenticated routes at all.
//
// The assertion under test (spike method step 6) is that the narrow bypass on this hostname cannot be turned into
// access to the product hostname's gated routes. This Worker deliberately has no /internal/* route and no secret
// marker: any gated marker seen through THIS hostname means the hostname is wired to the wrong Worker, or Access and
// the edge disagree about which hostname a request is for (SNI vs Host).

export default {
  async fetch(request) {
    const url = new URL(request.url);
    const h = request.headers;
    const body = {
      worker: "spike-0004-space",
      surface: "space",
      pathname: url.pathname,
      host: url.host,
      access: {
        jwt_present: h.has("cf-access-jwt-assertion"),
        cf_authorization_cookie_present: /(?:^|;\s*)CF_Authorization=/.test(h.get("cookie") || ""),
      },
    };
    // Anything an authenticated surface would live under does not exist here.
    const status = url.pathname.startsWith("/internal") || url.pathname.startsWith("/gated") ? 404 : 200;
    return Response.json(body, { status, headers: { "cache-control": "no-store" } });
  },
};
