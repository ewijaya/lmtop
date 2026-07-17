# Privacy model

This application is **local-first**. Its job is to summarize usage metadata
that already exists on your machine — it does not need, and does not take,
anything else.

## What is read

| Source | What is used | What is ignored |
|---|---|---|
| Codex CLI session logs (`~/.codex/sessions/`) | session id, timestamps, working-directory basename, model ids, token counters, provider rate-limit percentages | prompt text, model output, tool calls, instructions |
| Claude Code session logs (`~/.claude/projects/`) | session id, timestamps, working-directory basename, model ids, per-request token usage | message content, tool output, attachments |
| Claude Code quota cache (`~/.claude.json` → `cachedUsageUtilization` only) | quota percentages, reset times, model-scoped limit percentages, fetch time | every other key in the file — account ids, OAuth metadata, machine ids, feature flags |
| Codex state database (`~/.codex/state_N.sqlite`, `threads` table, read-only) | thread id, timestamps, working-directory basename, model id, cumulative token total | thread titles, first user messages, previews, git metadata, every other column and table |
| Custom provider source (only if you configure one) | the JSON you point it at | — |

Session files are parsed line by line for the specific metadata fields
above. Prompt bodies, assistant output, and tool results are present in
those files but are **never stored, aggregated, logged, or displayed** —
parsing extracts token counts and identifiers only.

## Opt-in live quota (`network_quota` / `--live`)

Disabled by default. When you enable it, and only then:

- For Codex, lmtop first spawns a short-lived `codex app-server`
  subprocess and asks it for rate limits over JSON-RPC. The CLI
  authenticates itself with its own stored credentials — lmtop reads no
  token at all on this path — and the subprocess is killed once the
  response arrives.
- Only when that is unavailable (or for Claude, always) does lmtop read
  the **access token** the provider's own CLI already stores locally
  (`~/.claude/.credentials.json` → `claudeAiOauth.accessToken`;
  `~/.codex/auth.json` → `tokens.access_token` and `tokens.account_id`).
- The token is used for exactly one thing: the `Authorization` header of a
  GET request to that provider's own usage endpoint
  (`api.anthropic.com/api/oauth/usage`;
  `chatgpt.com/backend-api/codex/usage`). It is never logged, persisted,
  displayed, or sent anywhere else, and nothing else in the credential
  files is read into program state.
- Responses contain only quota percentages, reset times, plan/credit
  metadata — the same numbers the provider's CLI shows in its own status
  screen. Refresh tokens are never touched; tokens are never refreshed or
  modified.
- `--offline` always wins over `network_quota`, whatever the config says.

## What is never done

- **No credential access without opt-in.** By default, authentication
  files (`auth.json`, OAuth tokens, keychains) are never read, printed,
  persisted, uploaded, or logged. `doctor` reports only whether an auth
  artifact *exists*. With `network_quota` enabled, only the token fields
  listed above are read, for the single purpose described above.
- **No credential modification.** Codex and Claude authentication files
  are never written to.
- **No network calls by default.** Without `network_quota`/`--live`, zero
  network requests are made. With it, the only requests are the two usage
  GETs above. `--offline` guarantees nothing touches the network.
- **No API keys.** Core functionality never asks for one.
- **No telemetry.** Nothing leaves your machine.
- **No transcript copies.** Raw session files are never copied into any
  application database or cache. The one thing lmtop persists (unless
  `history.persist = false`) is its own history file
  (`~/.local/share/lmtop/history.jsonl` or platform equivalent):
  per-minute token totals and quota percentages with provider names and
  timestamps — no session ids, no project names, no content. It never
  leaves your machine and is pruned after `history.retention_days`.
- **No prompt content in errors.** Parse failures are counted, not quoted.
  Diagnostics replace the home directory with `~` and never include file
  contents.
- **No full paths in the UI.** Sessions display only the basename of their
  working directory (e.g. `myproject`), not the full path.

## Logging

Logging is off by default. Setting the diagnostic log environment variable
writes tracing output to the platform cache directory; log messages contain
file names and counters, never session content or credentials.

## Snapshot output

`snapshot --json` emits exactly the normalized domain model: token counts,
model identifiers, session ids, project basenames, quota percentages, and
health states. Review it once (`snapshot --json | less`) if you plan to
pipe it anywhere — it is designed to be safe to share, but it does include
project directory basenames and session ids.
