// Spike 0006: differential fuzz of HTML entity decoding (zeed-dom's `unescapeHTML` = entities 5.0.0 `decodeHTML`).
//   tools/spike-0006/run-oracle.sh <workdir> entities-fuzz.ts [count] [seed] > entities-fuzz.json
import { createRequire } from "node:module";

// zeed-dom is a dependency of @tiptap/html (what generateJSON uses), not of the editor package: resolve it from there
const req = createRequire(import.meta.url);
const { unescapeHTML } = createRequire(req.resolve("@tiptap/html"))("zeed-dom") as { unescapeHTML: (s: string) => string };

const COUNT = Number(process.argv[2] ?? 4000);
let a = Number(process.argv[3] ?? 99) >>> 0;
const r = () => {
  a = (a + 0x6d2b79f5) >>> 0;
  let x = Math.imul(a ^ (a >>> 15), 1 | a);
  x = (x + Math.imul(x ^ (x >>> 7), 61 | x)) ^ x;
  return ((x ^ (x >>> 14)) >>> 0) / 4294967296;
};
const pick = <T>(xs: T[]): T => xs[Math.floor(r() * xs.length)];
const NAMES =
  "amp lt gt quot apos nbsp copy reg euro trade hellip mdash ndash lsquo rsquo ldquo rdquo bull middot times divide plusmn frac12 frac14 deg micro para sect laquo raquo iexcl iquest Aacute aacute Eacute eacute ntilde szlig oslash aring AElig aelig Alpha alpha beta Omega omega pi sum infin ne le ge larr rarr uarr darr harr spades clubs diams hearts check checkmark cross star phone bigstar rightarrow LeftArrow NotEqual NewLine Tab excl num dollar percnt lpar rpar ast plus comma period sol colon semi equals quest commat lsqb bsol rsqb Hat lowbar grave lcub verbar rcub emsp ensp thinsp zwj zwnj lrm rlm shy not notin notit AMP LT GT QUOT COPY REG uml acute cedil macr sup1 sup2 sup3 yen pound cent curren brvbar ordf ordm yuml THORN thorn ETH eth foo notanentity amp2 ampx lt1 ltx".split(" ");
const NUMS = ["0", "1", "9", "10", "13", "32", "38", "60", "127", "128", "130", "133", "140", "141", "145", "150", "159", "160", "255", "256", "8364", "55295", "55296", "57343", "57344", "65535", "65536", "128512", "1114111", "1114112", "99999999999999999999"];
const HEX = ["0", "1f", "7f", "80", "9f", "a0", "ff", "d7ff", "d800", "dfff", "e000", "ffff", "10000", "1F600", "10ffff", "110000", "fffffffff"];
const TEXT = ["a", "b ", " ", "x<y", "q&r", "Hello", "日本", "😀", "=", "&", "&&", ";", "#", "x#y"];

function piece(): string {
  const k = r();
  if (k < 0.3) return `&${pick(NAMES)};`;
  if (k < 0.45) return `&${pick(NAMES)}`;
  if (k < 0.55) return `&${pick(NAMES)}${pick(["", "x", "1", " ", "=", "-", "."])}`;
  if (k < 0.7) return `&#${pick(NUMS)};`;
  if (k < 0.78) return `&#${pick(NUMS)}`;
  if (k < 0.88) return `&#${pick(["x", "X"])}${pick(HEX)}${pick([";", "", "g", " "])}`;
  if (k < 0.92) return pick(["&#", "&#x", "&#;", "&;", "&&", "& ", "&#xg;", "&#-1;", "&amp;amp;"]);
  return pick(TEXT);
}
const rows: { input: string; expected: string }[] = [];
for (let i = 0; i < COUNT; i++) {
  let s = "";
  const n = 1 + Math.floor(r() * 5);
  for (let j = 0; j < n; j++) s += piece();
  rows.push({ input: s, expected: unescapeHTML(s) });
}
process.stdout.write(JSON.stringify({ meta: { count: COUNT }, rows }) + "\n");
