# Verification pack: enabling the GitNexus plugin from project settings (2026-10-18)

Evidence for `.claude/settings.json`. Claude Code 2.1.285, GitNexus plugin 1.6.12. Both runs use an empty, isolated
`CLAUDE_CONFIG_DIR`, because the maintainer's own user-level config already enables the plugin and would mask any effect.
Paths are scrubbed. Each script writes its own status lines; a script's exit code means nothing.

| Log | What it shows |
|---|---|
| `plugin-test` | **Inconclusive, kept as a false start.** `claude plugin list --json` printed `[]` both inside the repo and in a control directory, because it lists *installed* plugins, and a project setting only recommends one. The third section (real user config) shows the plugin installed at user scope only, version 1.6.12 |
| `plugin-test2` | **The discriminating run.** With the debug log on, inside the repo Claude Code logs `Skipping orphaned enabledPlugins entry gitnexus@gitnexus-marketplace: marketplace not registered`; in the control directory that line is absent. So the project's `enabledPlugins` is read. "Orphaned" means a fresh environment must register the marketplace first. Both runs end `Not logged in` (the isolated config has no credentials), which does not affect the debug output |

Read, not run: the plugin's hooks (`hooks/gitnexus-hook.js`, `hooks/registry-query.cjs`), manifest, MCP definition and licence, all
described in `.agents/config/README.md`. Their behaviour was read from the source, not exercised.

Not done: the interactive trust prompt that registers the marketplace and installs the plugin for a new contributor; opting out
through `.claude/settings.local.json`; a session that actually loads the plugin's MCP server from this setting.
