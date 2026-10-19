# Agent tool configuration

Per-repo configuration for the code-intelligence tools. The real files live here. The paths the tools actually read are
symlinks, because neither tool has a per-repo setting for where it looks (measured 2026-10-18; scripts and logs in
`.agents/plans/project-skills/verification/serena-gitnexus-config-2026-10-18/`).

| Tool | Real location | Symlink the tool reads | Why a symlink |
|---|---|---|---|
| Serena | `.agents/config/serena/` | `.github/config/serena` | Serena's per-project data folder comes from one global template in `~/.serena/serena_config.yml`, `project_serena_folder_location: "$projectDir/.github/config/serena"`. Its only placeholders are `$projectDir` and `$projectFolderName`, so there is no per-repo override, and changing the template would move the folder for every repo |
| GitNexus | `.agents/config/gitnexus/gitnexusrc.json` | `.gitnexusrc` | `analyze` reads `.gitnexusrc` from the repo root only, and the name is fixed |

## Serena

- `project.yml` is the full file Serena writes, not a hand-trimmed one. Serena rewrote a trimmed file twice (to about 9.8 KB,
  then to the full 11 KB form; the first rewrite is logged as `Project configuration ... was incomplete, re-saving with
  default`) and then left it byte-identical across three further starts. A trimmed file would leave the tree dirty on first
  use. Each session starts two Serena servers (user-level and the plugin's), which may be different versions: if a later
  Serena adds keys, commit its rewrite instead of fighting it.
- `cache/` is ignored (`.gitignore` in the directory); `memories/` is where committed memories go.
- `project_name` is `thrust` in every worktree. Serena activates by path from the working directory
  (`--project-from-cwd`), so the shared name only matters for activation by name.
- `ignored_paths: [".agents/"]` keeps skills, plans and the code copies in other worktrees out of symbol search. Its
  effect was not tested.
- The Rust language server started (`rust-analyzer` from the rustup toolchain). Whether it reached readiness was not checked.

## GitNexus

- `.gitnexusrc` sets `indexOnly`: a plain `gitnexus analyze` then never writes into `AGENTS.md`, `CLAUDE.md` or
  `.claude/skills/`. Without it, `analyze` created `AGENTS.md`, `CLAUDE.md` and a `.claude/` directory (control run).
- The index is generated storage in `.gitnexus/` and is not relocated. Symlinking `.gitnexus` to a directory here was tried
  and rejected: `gitnexus clean` deletes the link and leaves the data behind, the next `analyze` creates a real `.gitnexus/`
  beside it, and GitNexus overwrites any `.gitignore` placed in the target directory with `*`.
- Hidden directories such as `.agents/` are never indexed (the walker skips dot-directories), so no `.gitnexusignore` is needed.
- The registry (`~/.gitnexus/registry.json`) is keyed by path, so index a checkout that stays; a pruned worktree leaves a stale entry.
- Analyzing from a feature-branch checkout prints `the default branch "main" is not the primary index` and indexes that branch
  in the flat slot. This happens with or without `defaultBranch` in the rc (measured both ways), so index a checkout of `main`.
- Not set: `defaultBranch`. With `indexOnly` nothing uses it (it only names the base ref in a generated example).

## The GitNexus Claude Code plugin

`.claude/settings.json` registers the `gitnexus-marketplace` (GitHub `abhigyanpatwari/GitNexus`) and enables
`gitnexus@gitnexus-marketplace` for this repo. It references the plugin; nothing of it is copied into the repository.

- **Read from the project settings:** with an empty, isolated config directory, Claude Code's debug log says `Skipping orphaned
  enabledPlugins entry gitnexus@gitnexus-marketplace: marketplace not registered` inside the repo and says nothing in a control
  directory. "Orphaned" means a fresh environment must register the marketplace first; the prompt that does that on first open is
  expected but was not exercised.
- **What it adds** (plugin 1.6.12, read from its files, not run here): an MCP server started as `npx -y gitnexus@latest mcp`
  (unpinned, fetched from npm), twelve skills, and two hooks. The `PreToolUse` hook runs on `Grep`, `Glob` and `Bash` and asks
  `gitnexus augment` for graph context. The `PostToolUse` hook runs on `Bash`; after `git commit`, `merge`, `rebase`,
  `cherry-pick` or `pull` it compares `HEAD` with the registered index and only *tells the agent* to run
  `gitnexus analyze --index-only` when the index is stale. It never re-indexes by itself. Both hooks return immediately unless
  the working directory is in a registered repo or one with its own `.gitnexus/` index.
- **Licence:** GitNexus is PolyForm Noncommercial 1.0.0. Enabling it from settings does not bundle or redistribute it, which is why
  `gitnexus-plan` is still not copied into this repo, but it does recommend a noncommercial-only tool to every contributor.
- **Overlaps and skew:** a machine that also registers its own `gitnexus` MCP server gets two servers with the same tools. The plugin
  runs `@latest`; a globally installed `gitnexus` may be older (1.6.8 here, 1.6.12 in the plugin) and shares `~/.gitnexus`.
- **Opting out locally:** set `"enabledPlugins": {"gitnexus@gitnexus-marketplace": false}` in `.claude/settings.local.json`
  (local settings override project settings; not tested here).

Symlinks need `core.symlinks` on Windows; `.claude/skills` already depends on it.
