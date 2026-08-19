---
domain: tooling
category: tokscale
sub-category: tui
date-created: 2026-08-19
date-revised: 2026-08-19
type: plan
status: DRAFT
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

`UnifiedMessage` carries no role, no message type, and no content discriminator. `is_turn_start` is the only structural flag and it distinguishes only the first assistant response after a user turn.

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

Contingent on the transcript survey. Skills and slash commands are only reportable if they appear as distinguishable events on disk; if the Skill tool surfaces as a `tool_use` with the skill name in its input, this stage collapses into a presentation layer over stage 2 rather than new parsing.

Subagent hierarchy is a separate matter. The `agent` field exists but flattens parent and child into one composite display name (`normalize_agent_name`, `sessions/mod.rs:107`), so a subagent appears as its own top-level row rather than nested under the agent that dispatched it. Making the hierarchy real means retaining the parent link, not just the label.

### stage 4: hooks, rules, and session or context events

Lowest confidence, and possibly not buildable. Hook executions and rule loads are harness-side activity that a transcript may never record. This stage does not start until the survey says the events exist on disk; if they do not, the honest outcome is to record that and stop rather than to synthesize numbers.

`session-data/` (3.1G, with `sessions.db`, `claims.db`, and `.session-data/session-scripts.db`) may already hold this history in queryable form. If it does, reading it is far cheaper than parsing it out of transcripts, and this stage becomes an integration rather than a parser change.

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
| 1 | `GroupBy::WorkspaceProviderModel` | 1 | NEW | |
| 2 | project multi-select dialog | 1 | NEW | clone `source_picker.rs` |
| 3 | project filter in `aggregate_messages` | 1 | NEW | key on `workspace_key`, not the label |
| 4 | register group-by in picker | 1 | NEW | |
| 5 | transcript survey for tool/skill/hook/command events | 2 | INPRG | gates stages 3 and 4 |
| 6 | `session-data/` database survey | 4 | INPRG | may replace transcript parsing entirely |
| 7 | retain tool names on the record | 2 | NEW | blocked on 5 |
| 8 | Tools tab | 2 | NEW | blocked on 7 |
| 9 | skills and commands | 3 | NEW | blocked on 5 |
| 10 | subagent parent link | 3 | NEW | needs parent retained, not just the label |
| 11 | hooks, rules, session events | 4 | NEW | blocked on 5 and 6; may prove impossible |

## decisions still open

`context-session-new` in the original request is not yet defined well enough to build. The two readings are a timeline of new-session and compaction events, or a per-session context-window utilization view. These need different data, so the item stays unscoped until it is settled.

## related

[codex-reasoning-double-count](../codex-reasoning-double-count.md) for the cache-invalidation constraints any `parser_version` bump inherits.
