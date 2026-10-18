"""Spike 0006 XSS corpus: a subset of the OWASP XSS Filter Evasion Cheat Sheet plus Plane-specific constructs.

Each entry: (id, category, malicious, html). `malicious=True` means: if this string reached a browser unsanitised it
would execute script or smuggle a dangerous URL; the independent checker must flag it (control). `malicious=False`
entries are inert text/markup the sanitiser must not be blamed for.

Synthetic strings only; every payload only calls alert(1) or points at example.invalid.
"""

N = "\x00"

P = []


def add(cat, malicious, html, id_suffix=None):
    P.append({"id": f"{cat}-{len(P):03d}", "category": cat, "malicious": malicious, "html": html})


# --- OWASP: basic -------------------------------------------------------------------------------
add("owasp-basic", True, "<script>alert(1)</script>")
add("owasp-basic", True, "<SCRIPT SRC=https://example.invalid/xss.js></SCRIPT>")
add("owasp-basic", True, "<script>alert(String.fromCharCode(88,83,83))</script>")
add("owasp-basic", True, "<IMG SRC=\"javascript:alert('XSS');\">")
add("owasp-basic", True, "<IMG SRC=javascript:alert('XSS')>")
add("owasp-basic", True, "<IMG SRC=JaVaScRiPt:alert('XSS')>")
add("owasp-basic", False, "<IMG SRC=`javascript:alert(\"RSnake says, 'XSS'\")`>")  # backtick quoting is IE-only; in HTML5 the value starts with a backtick
add("owasp-basic", True, "<IMG \"\"\"><SCRIPT>alert(\"XSS\")</SCRIPT>\">")
add("owasp-basic", True, "<IMG SRC=x onerror=alert(1)>")
add("owasp-basic", True, "<img src=x onerror=\"alert(1)\">")
add("owasp-basic", True, "<IMG SRC=# onmouseover=\"alert('xxs')\">")
add("owasp-basic", False, "<IMG SRC= onmouseover=\"alert('xxs')\">")  # HTML5: src's unquoted value is onmouseover="alert('xxs')": no handler attribute
add("owasp-basic", True, "<IMG onmouseover=\"alert('xxs')\">")
add("owasp-basic", True, "<IMG SRC=/ onerror=\"alert(String.fromCharCode(88,83,83))\"></img>")
add("owasp-basic", True, "<a href=\"javascript:alert(1)\">click</a>")
add("owasp-basic", True, "<a href=\" javascript:alert(1)\">click</a>")
add("owasp-basic", True, "<a href=\"JAVASCRIPT:alert(1)\">click</a>")
add("owasp-basic", True, "<a href=\"vbscript:msgbox(1)\">click</a>")
add("owasp-basic", True, "<a href=\"data:text/html;base64,PHNjcmlwdD5hbGVydCgxKTwvc2NyaXB0Pg==\">click</a>")
add("owasp-basic", True, "<a href=\"javascript&colon;alert(1)\">click</a>")

# --- OWASP: encoding / evasion ------------------------------------------------------------------
add("owasp-encoding", True, "<IMG SRC=&#106;&#97;&#118;&#97;&#115;&#99;&#114;&#105;&#112;&#116;&#58;&#97;&#108;&#101;&#114;&#116;&#40;&#39;&#88;&#83;&#83;&#39;&#41;>")
add("owasp-encoding", True, "<IMG SRC=&#0000106&#0000097&#0000118&#0000097&#0000115&#0000099&#0000114&#0000105&#0000112&#0000116&#0000058&#0000097&#0000108&#0000101&#0000114&#0000116&#0000040&#0000039&#0000088&#0000083&#0000083&#0000039&#0000041>")
add("owasp-encoding", True, "<IMG SRC=&#x6A&#x61&#x76&#x61&#x73&#x63&#x72&#x69&#x70&#x74&#x3A&#x61&#x6C&#x65&#x72&#x74&#x28&#x27&#x58&#x53&#x53&#x27&#x29>")
add("owasp-encoding", True, "<a href=\"jav&#x09;ascript:alert(1)\">tab</a>")
add("owasp-encoding", True, "<a href=\"jav&#x0A;ascript:alert(1)\">newline</a>")
add("owasp-encoding", True, "<a href=\"jav&#x0D;ascript:alert(1)\">cr</a>")
add("owasp-encoding", True, "<a href=\"&#x01;javascript:alert(1)\">ctrl</a>")
add("owasp-encoding", True, "<a href=\"  &#x20;javascript:alert(1)\">spaces</a>")
add("owasp-encoding", True, "<a href=\"jAvAsCrIpT&#58;alert(1)\">mixed</a>")
add("owasp-encoding", True, "<a href=\"&#106;avascript:alert(1)\">dec</a>")
add("owasp-encoding", True, "<IMG SRC=\"jav\tascript:alert('XSS');\">")
add("owasp-encoding", True, "<IMG SRC=\"jav\nascript:alert('XSS');\">")
add("owasp-encoding", True, "<IMG SRC=\"jav\rascript:alert('XSS');\">")
add("owasp-encoding", False, f"<IMG SRC=java{N}script:alert(\"XSS\")>")  # HTML5 turns NUL into U+FFFD: not a javascript: URL (legacy parsers only)
add("owasp-encoding", False, f"<SCR{N}IPT>alert(\"XSS\")</SCR{N}IPT>")  # HTML5: tag name scr�ipt, not <script>
add("owasp-encoding", True, "<SCRIPT/XSS SRC=\"https://example.invalid/xss.js\"></SCRIPT>")
add("owasp-encoding", True, "<SCRIPT/SRC=\"https://example.invalid/xss.js\"></SCRIPT>")
add("owasp-encoding", True, "<<SCRIPT>alert(\"XSS\");//\\<</SCRIPT>")
add("owasp-encoding", True, "<SCRIPT SRC=https://example.invalid/xss.js?< B >")
add("owasp-encoding", True, "<SCRIPT SRC=//example.invalid/xss.js>")
add("owasp-encoding", True, "<IMG SRC=\"   javascript:alert('XSS');\">")
add("owasp-encoding", False, "\\\";alert('XSS');//")  # plain text: only dangerous inside a JS string context, never in HTML body
add("owasp-encoding", False, "&lt;script&gt;alert(1)&lt;/script&gt;")
add("owasp-encoding", False, "<p>&#60;script&#62;alert(1)&#60;/script&#62;</p>")
add("owasp-encoding", False, "<IMG SRC=x onerror&#61;alert(1)>")  # attribute NAMES are not entity-decoded: not an event handler
add("owasp-encoding", False, "+ADw-script+AD4-alert(1)+ADw-/script+AD4-")  # UTF-7: inert unless a charset is sniffed

# --- OWASP: event handlers / attributes ---------------------------------------------------------
for ev, tag in [("onclick", "p"), ("onmouseover", "div"), ("onfocus", "a"), ("onload", "body"), ("onanimationstart", "p"), ("onpointerover", "span"), ("ontoggle", "details"), ("onerror", "img"), ("onfocusin", "div"), ("onauxclick", "p"), ("onbeforeinput", "p")]:
    add("owasp-event", True, f"<{tag} {ev}=alert(1) style=\"animation-name:x\">x</{tag}>")
add("owasp-event", True, "<details open ontoggle=alert(1)>x</details>")
add("owasp-event", True, "<input autofocus onfocus=alert(1)>")
add("owasp-event", True, "<input type=image src=x onerror=alert(1)>")
add("owasp-event", True, "<p onclick=\"alert(1)\">a</p>")
add("owasp-event", True, "<p ONCLICK=\"alert(1)\">a</p>")
add("owasp-event", False, "<p on\x0cclick=\"alert(1)\">a</p>")  # form feed splits the name: attributes `on` and `click`
add("owasp-event", True, "<img src=1 href=1 onerror=\"javascript:alert(1)\"></img>")

# --- OWASP: tag vectors (tags outside Plane's allow-list) ---------------------------------------
for tag_html in [
    "<iframe src=\"javascript:alert(1)\"></iframe>",
    "<iframe src=\"https://example.invalid/\"></iframe>",
    "<iframe srcdoc=\"<script>alert(1)</script>\"></iframe>",
    "<object data=\"javascript:alert(1)\"></object>",
    "<object data=\"data:text/html;base64,PHNjcmlwdD5hbGVydCgxKTwvc2NyaXB0Pg==\"></object>",
    "<embed src=\"javascript:alert(1)\">",
    "<form action=\"javascript:alert(1)\"><input type=submit></form>",
    "<form><button formaction=\"javascript:alert(1)\">x</button></form>",
    "<isindex action=\"javascript:alert(1)\" type=image src=1>",
    "<meta http-equiv=\"refresh\" content=\"0;url=javascript:alert(1)\">",
    "<link rel=stylesheet href=\"https://example.invalid/x.css\">",
    "<base href=\"javascript:alert(1)//\">",
    "<style>@import 'javascript:alert(1)';</style>",
    "<style>body{background:url(javascript:alert(1))}</style>",
    "<svg onload=alert(1)>",
    "<svg><script>alert(1)</script></svg>",
    "<svg><a xlink:href=\"javascript:alert(1)\"><text>x</text></a></svg>",
    "<svg><animate onbegin=alert(1) attributeName=x dur=1s>",
    "<svg><foreignObject><iframe srcdoc=\"<script>alert(1)</script>\"></iframe></foreignObject></svg>",
    "<math><mi xlink:href=\"javascript:alert(1)\">x</mi></math>",
    "<video><source onerror=\"alert(1)\"></video>",
    "<audio src=x onerror=alert(1)>",
    "<marquee onstart=alert(1)>x</marquee>",
    "<body onload=alert(1)>",
    "<html><body background=\"javascript:alert(1)\">",
    "<table background=\"javascript:alert(1)\"><tr><td>x</td></tr></table>",
    "<table><tr><td background=\"javascript:alert(1)\">x</td></tr></table>",
    "<xss onafterscriptexecute=alert(1)><script>1</script>",
    "<template><script>alert(1)</script></template>",
    "<xmp><img src=x onerror=alert(1)></xmp>",
    "<plaintext><img src=x onerror=alert(1)>",
    "<select><template><style><img src=x onerror=alert(1)></style></template></select>",
    "<!--><script>alert(1)</script>-->",
    "<!-- --!><img src=x onerror=alert(1)> -->",
    "<?xml version=\"1.0\"?><script>alert(1)</script>",
    "<a href=\"x\"><script>alert(1)</script></a>",
    "<div><script>alert(1)</script>text</div>",
    "<img src=x:alert(alt) onerror=eval(src) alt=1>",
    "<a href=\"data:text/html,<script>alert(1)</script>\">x</a>",
]:
    add("owasp-tag", True, tag_html)
# parse-context dependent or inert payloads: kept as sanitiser inputs but not claimed dangerous by this checker
# (noscript differs by scripting flag; textarea/title are RCDATA; comments and CDATA are bogus comments in HTML)
for tag_html in [
    "<noscript><p title=\"</noscript><img src=x onerror=alert(1)>\">",
    "<textarea><img src=x onerror=alert(1)></textarea>",
    "<title><img src=x onerror=alert(1)></title>",
    "<!--[if gte IE 4]><script>alert(1)</script><![endif]-->",
    "<![CDATA[<script>alert(1)</script>]]>",
    "<scr<script>ipt>alert(1)</scr</script>ipt>",
    "<a href=\"//example.invalid/\">protocol relative (external link, not script)</a>",
]:
    add("owasp-tag", False, tag_html)

# --- OWASP: CSS via style attribute (style IS allowed on every element by Plane's allow-list) -----
add("owasp-css", True, "<p style=\"background:url(javascript:alert(1))\">x</p>")
add("owasp-css", True, "<p style=\"width:expression(alert(1))\">x</p>")
add("owasp-css", True, "<p style=\"behavior:url(xss.htc)\">x</p>")
add("owasp-css", True, "<p style=\"-moz-binding:url(https://example.invalid/xss.xml#xss)\">x</p>")
add("owasp-css", True, "<div style=\"background-image:url('javascript:alert(1)')\">x</div>")
add("owasp-css", True, "<div style=\"list-style:url(javascript:alert(1))\">x</div>")
add("owasp-css", False, "<div style=\"position:fixed;top:0;left:0;width:100%;height:100%;background:red\">overlay (UI redress, not script)</div>")
add("owasp-css", False, "<p style=\"color:red;font-weight:bold\">plain styles</p>")

# --- mXSS / parser differentials --------------------------------------------------------------------
add("mxss", True, "<math><mtext><table><mglyph><style><!--</style><img title=\"--&gt;&lt;img src=1 onerror=alert(1)&gt;\">")
add("mxss", True, "<svg></p><style><a id=\"</style><img src=1 onerror=alert(1)>\">")
add("mxss", True, "<form><math><mtext></form><form><mglyph><style></math><img src onerror=alert(1)>")
add("mxss", True, "<noscript><style></noscript><img src=x onerror=alert(1)>")
add("mxss", True, "<style><style/><img src=x onerror=alert(1)>")
# markup inside attribute values is inert as parsed; these test how the sanitiser SERIALISES it (ammonia 4.0.0 leaves < and > raw)
add("mxss", False, "<a title=\"&lt;script&gt;alert(1)&lt;/script&gt;\" href=\"x\">x</a>")
add("mxss", False, "<p id=\"</p><script>alert(1)</script>\">x</p>")
add("mxss", False, "<div title='\"><script>alert(1)</script>'>x</div>")
add("mxss", True, "<p class=\"x\"><img src=x onerror=alert(1)//\">")
add("mxss", True, "<table><caption><img src=x onerror=alert(1)></caption></table>")

# --- Plane-specific constructs -----------------------------------------------------------------------
add("plane-mention", True, "<mention-component id=\"1\" entity_identifier=\"2\" entity_name=\"user_mention\" onclick=\"alert(1)\"></mention-component>")
add("plane-mention", True, "<mention-component id=\"1\" onmouseover=alert(1) entity_name=\"x\"></mention-component>")
add("plane-mention", False, "<mention-component id=\"&quot; onclick=&quot;alert(1)\"></mention-component>")  # quote entity stays inside the value
add("plane-mention", False, "<mention-component id=\"00000000-0000-4000-8000-000000000001\" entity_identifier=\"00000000-0000-4000-8000-000000000002\" entity_name=\"user_mention\"></mention-component>")
add("plane-image", True, "<image-component src=\"javascript:alert(1)\" width=\"100px\"></image-component>")
add("plane-image", True, "<image-component src=\"data:text/html;base64,PHNjcmlwdD5hbGVydCgxKTwvc2NyaXB0Pg==\"></image-component>")
add("plane-image", True, "<image-component src=\"x\" onerror=\"alert(1)\" onload=\"alert(1)\"></image-component>")
add("plane-image", False, "<image-component src=\"https://example.invalid/a.png\" width=\"1px\" height=\"1px\" aspectRatio=\"javascript:alert(1)\"></image-component>")  # not a URL attribute
add("plane-image", True, "<img src=\"data:image/svg+xml;base64,PHN2ZyBvbmxvYWQ9YWxlcnQoMSk+\">")
add("plane-image", True, "<img src=\"data:image/svg+xml,<svg onload=alert(1)>\">")
add("plane-image", True, "<img src=\"javascript:alert(1)\" alt=\"x\">")
add("plane-image", False, "<image-component src=\"00000000-0000-4000-8000-000000000011\" width=\"400px\" height=\"auto\" alignment=\"center\" status=\"uploaded\"></image-component>")
add("plane-image", False, "<img src=\"https://example.com/a.png\" alt=\"ok\" width=\"50%\">")
add("plane-embed", True, "<issue-embed-component entity_identifier=\"1\" onclick=\"alert(1)\"></issue-embed-component>")
add("plane-embed", True, "<issue-embed-component id=\"x\" entity_name=\"issue\"><script>alert(1)</script></issue-embed-component>")
add("plane-embed", False, "<issue-embed-component entity_identifier=\"1\" project_identifier=\"2\" workspace_identifier=\"3\" id=\"4\" entity_name=\"issue\"></issue-embed-component>")
add("plane-callout", True, "<div data-block-type=\"callout-component\" data-emoji-url=\"javascript:alert(1)\"><p>x</p></div>")
add("plane-callout", True, "<div data-block-type=\"callout-component\" data-icon-name=\"x\" onclick=\"alert(1)\"><p>x</p></div>")
add("plane-callout", False, "<div data-emoji-unicode=\"128161\" data-emoji-url=\"https://cdn.jsdelivr.net/npm/emoji-datasource-apple/img/apple/64/1f4a1.png\" data-logo-in-use=\"emoji\" data-block-type=\"callout-component\"><p>ok</p></div>")
add("plane-table", True, "<table><tr><td colspan=\"2\" style=\"background:url(javascript:alert(1))\">x</td></tr></table>")
add("plane-table", True, "<table><tr onclick=\"alert(1)\" background=\"red\" textColor=\"blue\"><td>x</td></tr></table>")
add("plane-table", False, "<table><tbody><tr><th colspan=\"1\" rowspan=\"1\" colwidth=\"150\" background=\"none\"><p>h</p></th></tr><tr><td colspan=\"1\" rowspan=\"1\" colwidth=\"150\" background=\"peach\" textColor=\"gray\"><p>c</p></td></tr></tbody></table>")
add("plane-link", True, "<a href=\"javascript:alert(document.cookie)\" target=\"_blank\" rel=\"opener\">x</a>")
add("plane-link", False, "<a href=\"https://example.com/?a=1&amp;b=2\" target=\"_blank\" rel=\"noopener noreferrer nofollow\" class=\"link\">ok</a>")
add("plane-link", False, "<a href=\"mailto:someone@example.com\">mail</a><a href=\"tel:+15551234\">tel</a>")
add("plane-link", False, "<a href=\"/relative/path\">relative</a>")
add("plane-task", True, "<ul data-type=\"taskList\"><li data-type=\"taskItem\" data-checked=\"true\" onclick=\"alert(1)\"><label><input type=\"checkbox\" checked=\"checked\" onfocus=\"alert(1)\"><span></span></label><div><p>x</p></div></li></ul>")
add("plane-task", False, "<ul data-type=\"taskList\"><li data-type=\"taskItem\" data-checked><label><input type=\"checkbox\" checked=\"checked\"><span></span></label><div><p>x</p></div></li></ul>")
add("plane-code", True, "<pre><code class=\"language-js\" onclick=\"alert(1)\">let a;</code></pre>")
add("plane-code", False, "<pre><code class=\"language-js\">if (a &lt; b &amp;&amp; c &gt; d) { alert(1) }</code></pre>")
add("plane-code", False, "<p>alert(1) and &lt;script&gt;alert(1)&lt;/script&gt; as text</p>")
add("plane-color", True, "<span data-text-color=\"red\" style=\"color:red\" onmouseover=\"alert(1)\">x</span>")
add("plane-color", False, "<span data-text-color=\"gray\">gray</span><span data-background-color=\"peach\">peach</span>")
# DOM clobbering needs id/name on elements: advisory (reported separately), not script execution
add("plane-id", False, "<p id=\"__proto__\">x</p>")
add("plane-id", False, "<a id=\"x\" name=\"y\" href=\"https://example.com\">dom clobbering candidate</a>")
add("plane-id", True, "<form id=\"x\"></form><input name=\"y\" form=\"x\">")

# --- benign Plane content (must survive; used for the preservation measurement, never "malicious") ----
add("benign", False, "<h2 class=\"editor-heading-block\">Title</h2><p class=\"editor-paragraph-block\">Hello <strong>bold</strong> <em>it</em> <code class=\"rounded-sm\">c</code></p>")
add("benign", False, "<ul class=\"list-disc pl-7\"><li class=\"not-prose space-y-2\"><p>a</p></li></ul>")
add("benign", False, "<ol start=\"3\" type=\"a\"><li><p>x</p></li></ol>")
add("benign", False, "<blockquote><p>quote</p></blockquote><div data-type=\"horizontalRule\"><div></div></div>")
add("benign", False, "<p>unicode 日本語 😀 &amp; &lt; &gt; &quot; &apos; &nbsp;</p>")

CORPUS = P
