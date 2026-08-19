---
domain: tooling
category: tokscale
sub-category: tui
date-created: 2026-08-19
date-revised: 2026-08-19
type: plan
status: INPRG
aliases:
tags:
rigor: high
---

# harness-analytics-views

Staged plan for project/provider/model slicing and the harness-activity views (tool use, skills, hooks, rules, commands, session events) in the tokscale TUI.

## goal

Let a user select one or more projects and see usage broken down across provider and model within that selection, and add views for the harness activity that produced the usage: which tools ran, which skills and commands were invoked, which hooks fired, which subagents did the work, and how sessions and context evolved.

## why this is staged

The request spans two different kinds of work. One is exposing dimensions the core already computes. The other requires data that tokscale does not retain and in some cases does not parse at all. Bundling them would put a multi-week data-model change in front of a change that could ship this week, so they are separated by what the data supports rather than by what the views look like.

## what the data supports today

Verified against `UnifiedMessage` (`crates/tokscale-core/src/sessions/mod.rs:68`) and the `GroupBy` enum (`crates/tokscale-core/src/lib.rs:203`).

| dimension | retained today | notes |
|---|---|---|
| project (workspace) | yes | `workspace_key` and `workspace_label`, set by every client. Claude Code's key is corrected to the reported `cwd` by the fix in this cycle. |
| provider | yes | `provider_id` |
| model | yes | `model_id` |
| client | yes | `client`, already multi-selectable |
| agent / subagent | yes | `agent`, already surfaced in the Agents tab |
| session | yes | `session_id`, `session_title` |
| tool use | no | `tool_use` blocks are parsed in `claudecode.rs:264` only to detect `name: "Agent"` and read `input.subagent_type`, then discarded. Tool names are reachable in the parser but never retained. |
| skills | no | the only `SKILL.md` handling is `crates/tokscale-core/src/mcp.rs:79`, which reads MCP server names out of frontmatter for an unrelated inventory feature. No invocation parsing. |
| hooks | no | zero matches for `PreToolUse` or `PostToolUse` anywhere in `crates/`. |
| slash commands | no | no parsing anywhere. |
| rules | no | no parsing anywhere. |
| session and context events | no | compaction is handled throughout `message_cache.rs`, but only as something the cache must reconcile without double counting. No field records that a compaction happened. |

Read that table as a statement about the parser, not about the data. The next section shows most of these concepts are present in the transcripts and discarded during parsing, which is what makes the later stages tractable at all.

`UnifiedMessage` carries no role, no message type, and no content discriminator. `is_turn_start` is the only structural flag and it distinguishes only the first assistant response after a user turn.

## what the transcripts contain

Surveyed 2026-08-19 against real data on this machine: 120 Claude Code transcripts (20,362 lines) and a 100-file Codex sample drawn from 1,037 files. This is the finding that decides which stages are buildable, and it is more favourable than the retained schema suggests: almost everything is on disk, it is simply discarded during parsing.

| concept | Claude Code | Codex |
|---|---|---|
| tool use | yes: `tool_use` blocks with `name`. 1,840 calls across 15 tools in a 60-file sample, led by Bash 945, Read 346, Edit 171 | yes: `function_call.name` and `custom_tool_call.name`. `exec_command` 7,074, `exec` 1,156, `apply_patch` 543 |
| MCP tools | present as `server_tool_use` blocks, distinct from `tool_use` | yes, and cleanly separated: `mcp_tool_call_end` with `invocation.server` and a dotted `server.tool` name |
| skills | **yes**: the `Skill` tool appears as an ordinary `tool_use` whose `input.skill` names the skill | no: `skills` and `host_skills` appear only inside `world_state` as config flags saying whether instructions were injected. No invocation record |
| hooks | **yes**: `hookCount`, `hookInfos`, `hookErrors`, `hookAdditionalContext`. `hookInfos` entries are `{command}` or `{command, durationMs}`, so both which hook ran and how long it took are recoverable | no: no hook event type exists anywhere in the payload vocabulary |
| slash commands | yes: `<command-name>` markers in user content | text only: the command lands as the leading token of `user_message.message` with no dedicated field, and volume is tiny (10 across the whole corpus) |
| subagents | yes: `isSidechain` and `agentId` on most lines, plus `Agent` tool calls carrying `input.subagent_type` (general-purpose 37, Explore 5, fork, frontmatter-tech) | yes, and with real hierarchy: `spawn_agent.arguments.agent_type` plus `sub_agent_activity` events carrying `agent_path` like `root/<task-slug>` |
| context events | no compaction marker observed in the sample | yes: a `context_compacted` event, though it carries no token counts. A before-and-after delta has to be joined from the surrounding `token_count` events |

The asymmetry matters for scoping. Skills and hooks are Claude Code only, so those views are per-client rather than universal, and must degrade honestly for clients that cannot report them rather than showing a misleading zero.

## session-data is not a shortcut

`~/Projects/session-data` (3.1G) was checked as a possible cheaper source than re-parsing transcripts. It is not.

`sessions.db` holds `fact_event` (257,337 rows) whose `event_type` is only `assistant`, `user`, `last-prompt`, `result`: a four-way split for token accounting with no tool, skill, hook, command or subagent granularity. `claims.db` is a multi-agent work-claim ledger. `.session-data/session-scripts.db` is a catalog of recurring shell snippets. Grepping both ingest scripts for `tool_use`, `tool_name`, `skill`, `hook`, `isSidechain`, `subagent`, `slash`, `command_name` and `mcp_tool` returns zero matches: the pipeline was never built to extract any of it.

So every stage below parses transcripts. The databases remain useful for session-level and token-level questions, and for multi-agent lane reporting, but they cannot back any of the requested views.

## stages

Each stage is independently shippable and leaves the tree green. A stage does not start until the one before it has landed, because each later stage builds on the record shape the earlier one establishes.

### stage 1: project selection and the three-way slice

No data-model change. Everything needed is already on the record.

- Add `GroupBy::WorkspaceProviderModel`. The enum has six variants and none combines workspace with provider (`lib.rs:203`), so the exact project-by-provider-by-model slice cannot be expressed today.
- Add a project multi-select. `ClientPickerDialog` (`crates/tokscale-cli/src/tui/ui/dialog/source_picker.rs`) is the pattern to clone: a shared `Rc<RefCell<HashSet<_>>>`, type-to-filter, checkbox rows, and a refusal to empty the selection. The one real difference is that the item list is discovered from scan results rather than from a static enum, so it has to be rebuilt when data reloads.
- Apply the selection as a filter in `DataLoader::aggregate_messages` (`crates/tokscale-cli/src/tui/data/mod.rs:461`) so every view narrows at once rather than each view filtering for itself.
- Register the new group-by in `GroupByPickerDialog` (`ui/dialog/group_by_picker.rs:36`).

Risk: the workspace label is currently only built when `GroupBy::WorkspaceModel` is active (`data/mod.rs:483`), so the filter must key on `workspace_key` rather than the display label.

### stage 2: tool use

First stage that changes the core record, and the cheapest of the ones that do, because the parser already walks the blocks it would need.

- Decide the retained shape. A per-message list of tool names is the smallest thing that answers both "total tool calls" and "calls by tool", and it keeps the record additive so existing aggregation still works.
- Extend `UnifiedMessage` and `ParsedMessage` together (`lib.rs:362`), plus `unified_to_parsed` and `parsed_to_unified` (`lib.rs:4890`), or the field silently drops on the round trip.
- Retain tool names in `claudecode.rs` where the blocks are already parsed, then extend to the other parsers that expose tool calls.
- Bump `parser_version` for every parser changed. Claude Code's bump is the dangerous one: the warning at `message_cache.rs:984` states it discards cached entries carrying assistant turns compaction has already removed from the live file. Prefer a scheme that leaves cached rows valid, such as treating a missing tool list as unknown rather than as zero.
- New Tools tab per the checklist below.

### stage 3: skills, commands, and subagent hierarchy

Skills turned out to be the cheap case rather than the expensive one. On Claude Code the `Skill` tool is an ordinary `tool_use` whose `input.skill` names the skill, so once stage 2 retains tool calls this is a presentation layer over the same data rather than new parsing. It is Claude Code only: Codex records skills as session config and never as invocations, so the view has to say "not reported by this client" rather than show a zero.

Slash commands are reportable on both clients but barely justify a view at current volume. Claude Code marks them with `<command-name>` in user content, and the entire Codex corpus contains ten. Fold them into the skills view rather than giving them a tab.

Subagent hierarchy is the real work in this stage. The `agent` field flattens parent and child into one composite display name (`normalize_agent_name`, `sessions/mod.rs:107`), so a subagent is its own top-level row rather than nested under whoever dispatched it. Both clients carry what is needed to fix that: Claude Code has `isSidechain` and `agentId` plus `Agent` calls with `input.subagent_type`, and Codex has `sub_agent_activity` with an `agent_path` of the form `root/<task-slug>`. Making the hierarchy real means retaining the parent link, not a prettier label.

### stage 4: hooks and context events

Hooks are buildable, and richer than expected. Claude Code records `hookInfos` as `{command}` or `{command, durationMs}`, so the view can report which hook ran, how often, and how much wall time it cost, with `hookErrors` giving a failure count. The wall-time number is the most valuable thing here: a slow or failing hook taxes every single turn and nothing surfaces it today. Codex records no hook events, so this view is Claude Code only.

Rules are closed as not buildable. Neither transcript format records which rule files were loaded, and parsing cannot recover what was never written.

Context events are asymmetric. Codex emits `context_compacted` but carries no token counts on it, so a before-and-after delta has to be joined from the `token_count` events either side. No compaction marker appeared in the Claude Code sample. Worth building only if the compaction view is wanted for its own sake.

## adding a tab

The per-tab checklist, since every stage after the first adds one. `Tab` is hand-maintained and the compiler catches only some of the omissions.

1. `crates/tokscale-cli/src/tui/app.rs:56` variant
2. `app.rs:70` `Tab::all()`, which also sets header order
3. `app.rs:85` `as_str()` and `app.rs:100` `short_name()`
4. `app.rs:115` `next()` and `app.rs:130` `prev()`, both directions and both by hand
5. `app.rs:1612` `default_sort_for_tab` if the tab should default to date rather than cost
6. `app.rs:1623` `tab_visible` only if the tab is conditional
7. new `ui/<tab>.rs` with `pub fn render`, following `ui/agents.rs` as the smallest example
8. `ui/mod.rs:1` module declaration and `ui/mod.rs:49` dispatch arm
9. `ui/footer.rs:151` `current_count_label`, which is an exhaustive match and will fail the build if missed
10. `data/mod.rs:194` `UsageData` field plus population in `aggregate_messages`
11. `cache.rs` only if the data must survive a restart. Monthly, Minutely and Sessions are not cached today, so a new tab may follow that precedent and skip it. If it is cached, `CACHE_SCHEMA_VERSION` (`cache.rs:26`) bumps.

## tracking

| # | item | stage | status | notes |
|---|---|---|---|---|
| 1 | `GroupBy::WorkspaceProviderModel` | 1 | DONE | three tests; verified on real data at 320 rows across 166 projects |
| 2 | project multi-select dialog | 1 | DONE | `w` opens it; eight tests |
| 3 | project filter in `aggregate_messages` | 1 | DONE | keys on `workspace_key`; filtered scans are never cached |
| 4 | register group-by in picker | 1 | DONE | |
| 5 | transcript survey for tool/skill/hook/command events | 2 | DONE | both clients surveyed; results above |
| 6 | `session-data/` database survey | 4 | DONE | ruled out, holds none of this data |
| 7 | retain tool names on the record | 2 | NEW | confirmed present in both clients |
| 8 | Tools tab | 2 | NEW | blocked on 7 |
| 9 | skills and commands view | 3 | NEW | Claude Code only; rides on 7 |
| 10 | subagent parent link | 3 | NEW | both clients carry the parent; needs retaining |
| 11 | hooks view | 4 | NEW | Claude Code only; `hookInfos` gives command and durationMs |
| 12 | rules view | 4 | ARCHIVED | not recorded in either transcript format |
| 13 | context/compaction view | 4 | NEW | Codex only; deltas joined from surrounding `token_count` |

## decisions still open

`context-session-new` in the original request is not yet defined well enough to build. The two readings are a timeline of new-session and compaction events, or a per-session context-window utilization view. These need different data, so the item stays unscoped until it is settled.

## related

[codex-reasoning-double-count](../codex-reasoning-double-count.md) for the cache-invalidation constraints any `parser_version` bump inherits.
