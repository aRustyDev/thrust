# Research 0001 — `apps/admin` (god-mode) capability inventory

Source: read-only code survey, 2026-10-17 (`apps/admin/app/(all)/`, `apps/api/plane/license/`, `api/instances/`).
Purpose: input to ADR-0021 / Q-0021.2. **Classification is a proposal awaiting owner review.**
Classes: **A** workspace setting · **B** IaC/env only (secrets, client ids, URLs, deployment config) · **C** drop.
Not traced: every consumer file in `apps/web`/`apps/space` (see "Dependencies").

## Backend surface
- `api/instances/` → `plane/license/urls.py`. Models (`license/models/instance.py`): `Instance` (singleton: instance_name,
  instance_id, whitelist_emails, versions, edition, domain, is_telemetry_enabled, is_support_required, is_setup_done,
  is_signup_screen_visited, is_verified, is_test), `InstanceAdmin` (user FK, role 20), `InstanceConfiguration`
  (key unique, value, category, is_encrypted), `ChangeLog`.
- Bootstrap commands: `register_instance` (creates Instance, version, calls GitHub for latest release),
  `configure_instance` (seeds config keys from env, encrypting secrets). Encryption helper `license/utils/encryption.py`.
- Backend auth providers read `InstanceConfiguration` via `utils/instance_value.py::get_configuration_value` with env fallback.

## Capabilities and proposed class
| Group | Capability (route) | Keys / endpoints | Class |
|---|---|---|---|
| Instance | Name, telemetry opt-in (`/general`) | `Instance.instance_name`, `is_telemetry_enabled` | name **A**; telemetry **B** (or C); id/version **C** |
| Instance | Admin first-run setup (`/`) | `admins/sign-up|sign-in|sign-up-screen-visited/`, `is_setup_done` | **C** as a flow; bootstrap owner via env/seed (**B**) |
| Instance | Admin session/me/sign-out; admin list/delete | `admins/*`, `InstanceAdmin` | **C** (workspace admin role replaces) |
| Instance | Register / version check | `register_instance`, `ChangeLog` | **C** (deploy concern) |
| Auth | Signup / magic-link / email-password toggles (`/authentication`) | `ENABLE_SIGNUP`, `ENABLE_MAGIC_LINK_LOGIN`, `ENABLE_EMAIL_PASSWORD` | **A** (+ **B** default so first boot can log in) |
| Auth | Google / GitHub / GitLab / Gitea | `IS_*_ENABLED`, `*_CLIENT_ID`, `*_CLIENT_SECRET` (enc), host/org keys | enable **A**; client id **B**; **secret B** (or **C** if provider dropped) |
| Auth | OIDC (woven addition) | `IS_OIDC_ENABLED`, `OIDC_CLIENT_ID/SECRET`, `OIDC_URL_*` | enable/URLs **A or B**; **secret B**; seed from env (bootstrap path) |
| Auth | `ENABLE_*_SYNC` keys | seeded, no admin UI | **C** |
| Email | SMTP config, test email, disable (`/email`) | `EMAIL_HOST/PORT/USER/PASSWORD/FROM`, `ENABLE_SMTP`, `email-credentials-check/` | **B**; mostly **C** once Cloudflare Email Service replaces SMTP (ADR-0073) |
| AI | LLM provider/model (`/ai`) | `LLM_API_KEY` (secret), `LLM_MODEL` | key **B**; model **A** |
| Image | Unsplash (`/image`) | `UNSPLASH_ACCESS_KEY` (secret) | **B** (or **C** if cover picker drops Unsplash) |
| Workspace | List / create / disable creation (`/workspace`) | `workspaces/`, `workspace-slug-check/`, `DISABLE_WORKSPACE_CREATION` | **C** (single org; org created at bootstrap **B**) |
| Other keys in `GET instances/` | PostHog key/host; Slack client id; base URLs (`ADMIN/SPACE/APP_BASE_URL`); `FILE_SIZE_LIMIT` (env, not stored) | | **B** (`FILE_SIZE_LIMIT` **A** if tunable) |
| Other | `INSTANCE_CHANGELOG_URL`, `IS_SELF_MANAGED`; `whitelist_emails`, `domain`, `is_support_required` | | **C** — check whether a signup allowlist is enforced anywhere before dropping `whitelist_emails` |

## Dependencies other apps have on instance config
- `apps/web` calls public `GET api/instances/` on load (`core/lib/wrappers/instance-wrapper.tsx`, `core/store/instance.store.ts`,
  `core/services/instance.service.ts` → `packages/services/src/instance/instance.service.ts`). If instance not set up → redirect to `/god-mode`.
- Auth screens read `is_*_enabled`, `is_magic_login_enabled`, `is_email_password_enabled`, `is_smtp_configured`
  (`components/account/auth-forms/*`, `hooks/oauth/core.tsx`); onboarding reads `is_workspace_creation_disabled`;
  image picker `has_unsplash_configured`; editor/AI `has_llm_configured`; Slack `slack_client_id`; GitHub `github_app_name`; PostHog init.
- `apps/space` uses the same instance/auth flags (files not confirmed).
- Write endpoints use `InstanceAdminPermission` (`InstanceAdmin`).

## Suggested shape (from the survey; for owner review)
Keep **one trimmed, public config-read endpoint** for web and space. Make auth toggles, the signup toggle, LLM model and
instance name **workspace settings**. Move every secret, client id, URL, SMTP/email setting and PostHog key to **env/IaC**.
Seed owner, org and OIDC at first boot. Retire `apps/admin`. Note: with BetterAuth (ADR-0050 A1) the provider toggles/credentials
model changes, so the Auth rows should be re-derived against that ADR.
