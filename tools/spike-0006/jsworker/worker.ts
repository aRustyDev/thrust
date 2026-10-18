// Spike 0006 Option C: the REAL @plane/editor conversion code (packages/editor/src/helpers/yjs-utils.ts) running as a
// Cloudflare Worker with no DOM shim. Same routes as the Rust Worker so the client can time both the same way.
//
//   POST /noop?n=K
//   POST /html-to-all?variant=rich|document&n=K     body = html          -> convertHTMLDocumentToAllFormats (what apps/live /convert-document runs)
//   POST /binary-to-all?variant=rich|document&n=K   body = base64 binary -> getAllDocumentFormats...BinaryData (the collab projection)
//   GET  /health, GET /env   (what browser-ish globals does workerd expose?)
import { convertHTMLDocumentToAllFormats, convertBase64StringToBinaryData, getAllDocumentFormatsFromDocumentEditorBinaryData, getAllDocumentFormatsFromRichTextEditorBinaryData } from "@/helpers/yjs-utils";

export default {
  async fetch(req: Request): Promise<Response> {
    const url = new URL(req.url);
    const n = Number(url.searchParams.get("n") ?? "1");
    const variant = (url.searchParams.get("variant") ?? "rich") as "rich" | "document";
    const full = url.searchParams.get("full") === "1";
    try {
      if (url.pathname === "/health") return new Response("ok");
      if (url.pathname === "/env") {
        const g = globalThis as any;
        return Response.json({ window: typeof g.window, document: typeof g.document, navigator: typeof g.navigator, DOMParser: typeof g.DOMParser, Node: typeof g.Node, localStorage: typeof g.localStorage, Buffer: typeof g.Buffer, TextEncoder: typeof g.TextEncoder });
      }
      if (url.pathname === "/noop") return new Response("noop");
      if (url.pathname === "/html-to-all") {
        const html = await req.text();
        let last: any;
        for (let i = 0; i < n; i++) last = convertHTMLDocumentToAllFormats({ document_html: html, variant });
        if (full) return Response.json(last);
        return Response.json({ json_bytes: JSON.stringify(last.description_json).length, html_bytes: last.description_html.length, binary_b64_bytes: last.description_binary.length, n });
      }
      if (url.pathname === "/binary-to-all") {
        const b64 = (await req.text()).trim();
        const bin = new Uint8Array(convertBase64StringToBinaryData(b64));
        let last: any;
        for (let i = 0; i < n; i++) last = variant === "rich" ? getAllDocumentFormatsFromRichTextEditorBinaryData(bin) : getAllDocumentFormatsFromDocumentEditorBinaryData(bin, false);
        if (full) return Response.json({ json: last.contentJSON, html: last.contentHTML });
        return Response.json({ json_bytes: JSON.stringify(last.contentJSON).length, html_bytes: last.contentHTML.length, n });
      }
      return new Response("not found", { status: 404 });
    } catch (e) {
      return new Response(String((e as Error).stack ?? e), { status: 500 });
    }
  },
};
