# Seed recipes

Seeds and regressions are **raw bytes**. Write them with `printf` octal escapes (`\ddd`, three digits) and
read them back before you commit. Do not use `echo`: bash prints the escapes as text plus a newline, while
zsh and macOS `sh` interpret them but still append a newline byte, so either way the file is wrong
`[re-run]`. `printf` is the only portable choice.

Example inputs for the length-prefixed frame `[tag: u8][len: u16 big-endian][payload]` used by
`fuzz_harness.rs`. The names are descriptive and carry no hash or colon.

| File | Bytes (hex) | Meaning | Command |
|---|---|---|---|
| `seeds/parse_frame/empty-payload` | `07 00 00` | valid, tag 7, empty payload | `printf '\007\000\000' > fuzz/seeds/parse_frame/empty-payload` |
| `seeds/parse_frame/abc` | `01 00 03 61 62 63` | valid, tag 1, payload `abc` | `printf '\001\000\003abc' > fuzz/seeds/parse_frame/abc` |
| `seeds/parse_frame/length-mismatch` | `07 00 02 01` | rejected: declares 2, has 1 | `printf '\007\000\002\001' > fuzz/seeds/parse_frame/length-mismatch` |
| `regressions/parse_frame/too-short` | `ff ff` | rejected: fewer than 3 bytes | `printf '\377\377' > fuzz/regressions/parse_frame/too-short` |

Read each file back and compare with the hex column; commit only when they match. The four recipes above
produce exactly that hex `[re-run]`. Run `xxd` on **one file per command**: a second argument is its output
file and would overwrite it `[re-run]`.

```sh
xxd fuzz/seeds/parse_frame/abc
# 00000000: 0100 0361 6263                           ...abc
```

`od -An -tx1 <file>` shows the same bytes where `xxd` is absent.

If the dump shows `5c 78` (a backslash and an `x`), the file holds text. Delete it and write it again with
`printf`. The first two rows are the seeds that make the replay reach the harness's round-trip assertion;
the last two exercise the rejection path.
