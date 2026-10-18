// Dump the emoji table the Plane editor uses (gitHubEmojis with a glyph, as filtered in emoji/extension.ts):
// [name, glyph, shortcodes[]] in dataset order, because HTML rendering of an emoji node looks the glyph up by name/shortcode.
import { gitHubEmojis } from "@tiptap/extension-emoji";

const rows = gitHubEmojis.filter((item: any) => item.emoji).map((e: any) => [e.name, e.emoji, e.shortcodes]);
process.stdout.write(JSON.stringify(rows) + "\n");
