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

Session files are parsed line by line for the specific metadata fields
above. Prompt bodies, assistant output, and tool results are present in
those files but are **never stored, aggregated, logged, or displayed** —
parsing extracts token counts and identifiers only.

## What is never done

- **No credential access.** Authentication files (`auth.json`, OAuth
  tokens, keychains) are never read, printed, persisted, uploaded, or
  logged. `doctor` reports only whether an auth artifact *exists*.
- **No credential modification.** Codex and Claude authentication files
  are never written to.
- **No network calls.** The MVP makes zero network requests. Future
  network-backed quota collectors will be opt-in per provider
  (`network_quota = true`), clearly labeled, and disabled by default.
  `--offline` guarantees nothing touches the network.
- **No API keys.** Core functionality never asks for one.
- **No telemetry.** Nothing leaves your machine.
- **No transcript copies.** Raw session files are never copied into any
  application database or cache. Only normalized usage numbers live in
  memory, and they are not persisted in the MVP.
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
