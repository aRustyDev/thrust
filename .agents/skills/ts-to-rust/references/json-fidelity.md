# JSON shape and value fidelity (JS vs Rust)

Every row below was checked by running it `[verified: node v26.9.0, rustc 1.98.1, serde 1.0.229, serde_json 1.0.154
and 1.0.143, chrono 0.4.45, uuid 1.28.0, python3 3.14.7]`, with and without serde_json's `preserve_order` feature.
Versions move: if a row matters to you, re-assert it in your own suite rather than trusting this page. Cargo
features needed: `chrono` with `serde`, `uuid` with `serde`.

## 1. Shape: TypeScript pattern to serde

| TypeScript | Rust | Verified behaviour |
|---|---|---|
| `sequenceId` keys | `#[serde(rename_all = "camelCase")]` | `sequence_id` serialises as `"sequenceId"` |
| `a?: T` | `Option<T>` + `#[serde(default, skip_serializing_if = "Option::is_none")]` | missing key deserialises to `None`; `None` is omitted on output |
| `a: T \| null` | `Option<T>` | serialises `"a":null` unless skipped; JS `undefined` is dropped by `JSON.stringify`, `null` is kept: do not conflate them. A plain `Option<T>` field also **accepts an absent key** with no `#[serde(default)]`, so a TS required-nullable field silently accepts absence; add a check if absence must be rejected |
| `a?: T \| null` (three states, PATCH bodies) | `Option<Option<T>>` with `#[serde(default, deserialize_with = "double_option", skip_serializing_if = "Option::is_none")]`, where `double_option` wraps the inner `Option<T>` in `Some` | missing -> `None`, `null` -> `Some(None)`, value -> `Some(Some(v))`. **Without `default`**, `deserialize_with` makes a missing key a `missing field` error; **plain** `Option<Option<T>>` collapses `null` into `None` |
| required field absent | error | `missing field `tags`` unless `#[serde(default)]` |
| extra properties | ignored by default | `#[serde(deny_unknown_fields)]` errors with ``unknown field `z`, expected `a` `` |
| `[k: string]: unknown` | `#[serde(flatten)] extra: BTreeMap<String, Value>` | collects unknown keys |
| `{type: "circle", r}` discriminated union | `#[serde(tag = "type", rename_all = "lowercase")]` | `{"type":"circle","r":1.0}` |
| adjacently / externally tagged | `#[serde(tag = "t", content = "c")]` / default | `{"t":"Num","c":3}` / `{"A":{"x":1}}` |
| untagged union | `#[serde(untagged)]` | **first match wins and extra fields are ignored**: `{"x":1,"y":2}` matched `A{x}` before `B{x,y}`; failure text is only "data did not match any variant of untagged enum U". Put the most specific variant first, or prefer a tag |
| `"open" \| "closed"` | enum + `rename_all` | an unknown string is an error |
| unknown-string tolerance | `#[serde(other)]` unit variant | **lossy**: deserialises, then re-serialises as `"unknown"`, not the original string. Keep the string (custom `Deserialize`) if round-trips matter |
| branded id `string` | `#[serde(transparent)] struct Id(Uuid)` | serialises as the bare string; invalid input rejected. `Uuid` parsing is **lenient**: uppercase, hyphenless, `{braced}` and `urn:uuid:` forms are all accepted and always re-serialise as lowercase hyphenated, so a byte-for-byte id round trip is not guaranteed |
| validated string | `#[serde(try_from = "String")]` | rejects bad values at parse time |
| `[string, number]` | `(String, i64)` | serialises as a JSON array |
| `unknown` / `any` | `serde_json::Value` **at the boundary only**, then `serde_json::from_value::<T>` | error names the type: `invalid type: string "not a number", expected i64` |

Duplicate keys: `JSON.parse` keeps the last; `serde_json::Value` keeps the last; a derived struct **errors**
(`duplicate field `a``).

## 2. Numbers

| Fact | JS | Rust |
|---|---|---|
| Large integers | `JSON.parse('{"id":9007199254740993}')` gives `9007199254740992`; `18446744073709551615` becomes `18446744073709552000` | `u64` parses `18446744073709551615` exactly; `Value::as_i64` keeps `9007199254740993`, `as_f64` gives `9007199254740992.0` |
| Integer-valued floats print | `JSON.stringify(5.0)` is `5`, `-0` is `0` | `5.0` and `-0.0` |
| Other float text | `1e20` is `100000000000000000000`, `1e-6` is `0.000001`, `1e21` is `1e+21` | **depends on the `serde_json` release**: 1.0.154 prints `1e+20`, `1e-6`, `1e+21`; 1.0.143 prints `1e20`, `1e-6`, `1e21` (the float printer changed between them) |
| NaN / Infinity | `null` | `serde_json` writes NaN as `null`; it **rejects** a bare `NaN` token on input |
| Integer where the producer sends `1.0` | n/a | `i64` from `1.0` fails: ``invalid type: floating point `1.0`, expected i64``. Python's `json` prints floats as `1.0` and NaN as a bare `NaN` (checked), so confirm numeric field types against recorded responses |
| Bitwise `\|0` | wraps to 32 bits: `2**32 \| 0` is `0`, `2**31 \| 0` is `-2147483648` | float-to-int `as` **saturates** (`4294967296f64 as i32` is `i32::MAX`, NaN is `0`); int-to-int `as` wraps (`300i32 as u8` is `44`) |
| Rounding | `Math.round(-2.5)` is `-2` (and `Math.round(2.5)` is `3`) | `f64::round(-2.5)` is `-3` (half away from zero); `(x + 0.5).floor()` gives `-2` here, but test your own range |
| Ids | exact only to 2^53 | use `i64`/`u64`/`Uuid`; if a JS client reads these, they must travel as strings |

**Compare JSON by parsed value, never by bytes.** Float text differs in both directions above, and key order
differs too (section 5).

## 3. Dates

- JS `Date#toISOString` always writes three fractional digits and `Z`: `2023-11-14T22:13:20.000Z`.
- chrono's serde output drops the fraction for whole seconds (`"2023-11-14T22:13:20Z"`) and keeps millis
  otherwise (`.123Z`). To match JS exactly use `to_rfc3339_opts(SecondsFormat::Millis, true)` in a
  `#[serde(with = ...)]` module.
- chrono rejects a date-only string (`"2024-01-01"`); `new Date("2024-01-01")` parses it as UTC midnight. Offsets
  are accepted and normalised: `+02:00` input became `2023-12-31T22:00:00.000Z`.
- `new Date("garbage").getTime()` is `NaN`. Decide what the port does with invalid dates and freeze it.
- A JS date-time string **without** an offset is local time. Freeze with a fixed timezone and record it with the
  fixtures.

## 4. Strings and Unicode

| Fact | JS | Rust |
|---|---|---|
| Length of `"😀"` | `.length` is 2 (UTF-16 units); `[..."😀"].length` is 1 | `.len()` is 4 (bytes), `.chars().count()` is 1, `.encode_utf16().count()` is 2 |
| Slicing | `"😀".slice(0,1)` is a lone surrogate (`d83d`) | `s.get(0..1)` is `None`; `&s[0..1]` **panics** (clippy `string_slice` flags it) |
| Normalisation | composed `é` has length 1, decomposed length 2 | the two strings are unequal; nothing normalises implicitly |
| Default sort | UTF-16 code-unit order: `["😀","～"]` sorts as U+1F600, U+FF5E | byte (code point) order: U+FF5E, U+1F600. `sort_by(\|a, b\| a.encode_utf16().cmp(b.encode_utf16()))` reproduces JS |
| `localeCompare` | ICU/locale dependent (case, accents) | nothing in std; pick an explicit order or an ICU crate and record the choice in the exception register |
| `trim()` | removes U+FEFF, **keeps** U+0085 | `trim()` keeps U+FEFF, **removes** U+0085 |

## 5. Ordering and collections

- JS plain objects iterate integer-like keys ascending first, then string keys in insertion order:
  `Object.keys({b:1,2:1,a:1,1:1})` is `["1","2","b","a"]`. A `Map` keeps pure insertion order (`["b",2,"a",1]`).
- `serde_json::Value` objects serialise in **sorted** key order by default (`{"a":2,"b":1}`); insertion order only
  with the `preserve_order` feature. Feature unification means one dependency can flip this for the whole build.
- Rust's `HashMap` iteration order is unspecified. Never serialise one where order is observable: use
  `BTreeMap`, `IndexMap` or a `Vec` of pairs.
- `JSON.stringify(new Map([[1,2]]))` and of a `Set` are both `{}`; `JSON.stringify(10n)` throws. If the TS wire
  shape was already lossy, freeze what it actually emitted, not what the types suggest.
- `Array.prototype.sort` is stable; Rust's `sort_by` is stable and `sort_unstable_by` is not. See the stability
  case in `worked-example.md`: it takes a large, interleaved input to expose the difference.
