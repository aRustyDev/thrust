# Verification pack: Serena and GitNexus configuration under .agents/config (2026-10-18)

Evidence for `.agents/config/` (see its README). Claude Code 2.1.285, Serena as installed on the maintainer's machine, GitNexus
1.6.8, macOS. Scratch repos and a scratch `GITNEXUS_HOME`, so the real GitNexus registry was untouched; Serena auto-registers
every project it opens in `~/.serena/serena_config.yml`, and the probe's entry was removed afterwards. Paths are scrubbed. Each script
writes its own status lines; a script's exit code means nothing.

| Log | What it shows |
|---|---|
| `cfg-test` | Symlinks as the relocation mechanism. GitNexus: `.gitnexus` as a symlink into `.agents/config/gitnexus` indexes fine and `status` is up to date. Serena: `.github/config/serena` as a symlink into `.agents/config/serena`; Serena wrote `cache/` and `memories/` through it and created no real directory. The nested Claude answered `NONE` to a `get_current_config` question (that tool is not exposed by the user-level server), so the proof that Serena used the symlinked `project.yml` is in `serena-log-excerpts.txt` instead: it loaded the probe project under the marker name that exists only in the file behind the symlink |
| `cfg-test2` | **Rejected design**: the GitNexus index directory as a symlink. `gitnexus clean` deleted the link and left the data behind; the next `analyze` created a real `.gitnexus/` beside it; GitNexus overwrote a `.gitignore` placed in the target with `*`. Also: `.gitnexusrc` with `indexOnly` stops `analyze` creating `AGENTS.md`, `CLAUDE.md` and `.claude/` (control run without it created them). Section D (the ignore file) is inconclusive: `context` needs `-r` when several repos are registered; it is redone in `cfg-test3` |
| `cfg-test3` | **Adopted design**: config files kept in `.agents/config/gitnexus/` behind symlinks at the repo root. The rc is followed through its symlink; the ignore file is followed too (`hidden_fn` under the ignored directory absent; control without the ignore file: found); `clean` removes only the real `.gitnexus/` and leaves the config alone |
| `verify-cfg` | The real 11-crate repository, with an EARLIER config (rc with `defaultBranch`, plus a `.gitnexusignore` for `.agents/`). A plain `analyze` left `git status` unchanged and indexed real crate symbols. Two findings removed pieces: the `main is not the primary index` warning, and a control copy without the ignore file that also did not index `.agents/` (identical 1,652 nodes and 4,200 edges), because hidden directories are skipped by default. The ignore file was redundant and was dropped |
| `verify-final` | The final files. `analyze` with no flags leaves `git status` unchanged, creates no `CLAUDE.md` and indexes `from_env`. The `default branch` warning is still printed (count 1): it is caused by analyzing from a feature branch, with or without `defaultBranch`. Serena activated `thrust` at this path and started the Rust language server. Its `project.yml` hash changed once between the two checks in this script (`7c9f7d0e…` to `8399dc57…`): Serena rewrote the file a second time |
| `serena-log-excerpts.txt` | The marker project loaded through the symlink; the first-load `was incomplete, re-saving with default` lines; the Rust language server command |

## Observed outside the scripts

After the `verify-final` rewrite, `project.yml` was hashed before and after two more nested Claude runs (Serena started each
time): `8399dc57395cd148eb419a8e7a3c0f0838d9c1f6`, `8399dc57395cd148eb419a8e7a3c0f0838d9c1f6`,
`8399dc57395cd148eb419a8e7a3c0f0838d9c1f6`. Serena had therefore rewritten the trimmed file twice (9,786 bytes, then 10,979)
and then left it unchanged for three starts. The cause of the second rewrite is a hypothesis, not a finding: each session
starts two Serena servers (the user-level one and the plugin's, run from the latest upstream), which may be different versions
with different default key sets.

## Not done

- the effect of `ignored_paths: [".agents/"]` in Serena (no symbol-search test)
- whether the Rust language server reached readiness (it was started)
- a real GitNexus index of this repository in the real registry: the index is machine-local and keyed by path, so that is left to a
  checkout of `main` that stays
- Windows (`core.symlinks`), other Serena or GitNexus versions
