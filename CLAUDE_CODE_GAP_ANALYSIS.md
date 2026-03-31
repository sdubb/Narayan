# Claude Code Gap Analysis For Narayan

_Last updated: March 31, 2026_

## Executive summary

Narayan is not missing the core agent platform.

It already has:

- plan mode
- deterministic workflow execution
- DB-backed scheduling
- webhook delivery
- a large tool registry
- MCP tool execution
- queue-backed subagents
- skills
- a web dashboard

Relevant local references:

- `README.md:3`
- `src/agent/plan_mode.rs:1`
- `src/scheduler/ticker.rs:1`
- `src/webhooks/delivery.rs:1`
- `src/tools/mod.rs:256`
- `src/tools/delegate.rs:53`

The main difference between Narayan and Claude Code is not the core runtime.
The main difference is that Claude Code has a much thicker local-agent shell around that runtime:

- stronger permission policy
- stronger coordinator/worker discipline
- task-native shell state
- more explicit tool-governance prompts
- worktree and scratchpad workflow
- first-class MCP resource handling
- richer terminal UX

The key reverse-engineering takeaway is this:

Claude Code's edge is not just "more tools."
Its edge is the combination of:

- tool-specific prompts
- role-specific tool access
- strong orchestration rules
- prompt caching / deferred schema loading
- explicit task state
- disciplined worker prompt synthesis

## What Narayan already has

Narayan already looks strong in the areas below:

- conversational setup through plan mode
- deterministic execution from saved workflow outlines
- persistent scheduling through the role/scheduler layer
- inbound and outbound webhook handling
- broad built-in tool surface
- MCP server tool execution
- queue-backed child-agent spawning
- skill registry and marketplace primitives
- web UI plus backend API architecture

This means Narayan is already much closer to a production automation platform than a simple coding agent.

Narayan is also broader than Claude Code in some areas:

- more business/integration-oriented tools
- deeper connector strategy
- stronger automation-platform framing
- more data / infra / browser / workflow coverage in the default registry

Relevant local references:

- `src/agent/prompts.rs:8`
- `src/tools/request_more_tools.rs:1`
- `src/tools/mcp_session.rs:336`

## What Claude Code has on top

### 1. Permission and security depth

Claude Code has a much more complete permission model:

- multiple permission modes
- risk classes
- protected-file rules
- path traversal hardening
- generated permission explanations
- sandbox-aware retry behavior

Evidence:

- Upstream reference: `README.md:347`
- Upstream prompt policy: `constants/prompts.ts:186`
- Upstream bash safety: `tools/BashTool/prompt.ts:1`
- Narayan today: `src/tools/plane_guard.rs:10`
- Narayan WASM limits: `src/tools/run_registered_wasm.rs:49`

Current assessment:

Narayan has the start of a safety layer, but not a full permission system.

### 2. Coordinator-grade multi-agent orchestration

Claude Code has a more developed coordinator/worker model:

- coordinator-owned planning
- parallel worker research / implementation / verification
- self-contained worker prompts
- worker result notifications
- teammate messaging
- scratchpad sharing
- worktree isolation
- inheritance of permissions and plugins

Evidence:

- Upstream reference: `README.md:247`
- Upstream tool gating: `constants/tools.ts:55`
- Upstream coordinator prompt: `coordinator/coordinatorMode.ts:116`
- Upstream concurrency rule: `coordinator/coordinatorMode.ts:213`
- Narayan child spawning: `src/tools/delegate.rs:59`
- Narayan swarm wrapper: `src/swarm/mod.rs:21`

Current assessment:

Narayan can spawn child agents, but the orchestration layer is still thin compared with Claude Code's coordinator model.

### 3. Remote bridge / CCR-style execution

Claude Code has a more advanced remote execution shell:

- bridge mode
- session spawn modes like `single-session`, `worktree`, and `same-dir`
- remote control session concepts
- ULTRAPLAN-style remote planning

Evidence:

- Upstream reference: `README.md:152`
- Upstream bridge section: `README.md:418`
- Upstream commands surface: `commands.ts:58`
- Narayan ACP messaging: `src/tools/acp_session.rs:10`
- Narayan remote object storage: `src/workspace/remote.rs:165`

Current assessment:

Narayan has remote storage and agent-to-agent messaging primitives, but not a full remote-control compute bridge.

### 4. Hooks and plugin runtime

Claude Code exposes a richer runtime extension surface:

- `/hooks`
- `/plugin`
- `/reload-plugins`
- plugin skills
- plugin-aware command loading

Evidence:

- Upstream commands: `commands.ts:73`
- Upstream command registry: `commands.ts:258`
- Narayan skill registry: `src/skills/registry.rs:112`
- Narayan marketplace: `src/skill_marketplace/marketplace.rs:13`
- Narayan segment plugin host: `src/segments/registry.rs:1`
- Narayan registered WASM execution: `src/tools/run_registered_wasm.rs:49`

Current assessment:

Narayan has extensibility, but mostly through skills, segment plugins, marketplace items, and tenant-approved WASM.
It does not yet have an equivalent runtime hook/plugin system.

### 5. Terminal-native product UX

Claude Code is a heavy terminal-native product:

- vim mode
- bridge mode
- voice mode
- permissions commands
- plugin commands
- task commands
- many slash-command flows

Evidence:

- Upstream commands: `commands.ts:58`
- Upstream built-in command list: `commands.ts:319`
- Narayan product architecture: `README.md:25`

Current assessment:

Narayan is currently a backend platform plus dashboard, not a terminal-native agent shell.

### 6. MCP resources

Claude Code exposes first-class MCP resource primitives:

- resource listing
- resource reading

Evidence:

- Upstream reference: `README.md:329`
- Upstream resource prompts: `tools/ListMcpResourcesTool/prompt.ts:1`, `tools/ReadMcpResourceTool/prompt.ts:1`
- Narayan MCP actions: `src/tools/mcp_session.rs:344`

Current assessment:

Narayan's MCP layer currently focuses on:

- `connect`
- `list_tools`
- `call_tool`

It does not yet expose resource primitives as first-class operations.

### 7. Memory/meta-services

Claude Code includes background memory/meta-services such as:

- `autoDream`
- team-memory sync

Evidence:

- Upstream dream section: `README.md:168`
- Upstream team-memory mention: `README.md:265`
- Upstream consolidation prompt: `services/autoDream/consolidationPrompt.ts:15`

Current assessment:

Narayan has memory store/recall and vector memory, but I did not find a dream-style background consolidation service.

### 8. User-facing task shell

Claude Code has task lifecycle commands directly in the shell.

Evidence:

- Upstream task shell reference: `README.md:327`
- Upstream task prompts: `tools/TaskCreateTool/prompt.ts:21`, `tools/TaskUpdateTool/prompt.ts:43`
- Narayan direct schedule tool: `src/tools/schedule.rs:10`
- Narayan in-memory cron tools: `src/tools/cron.rs:29`
- Narayan real role scheduler: `src/scheduler/ticker.rs:1`

Current assessment:

Narayan has durable scheduling at the role/runtime level, but the direct `schedule` and `cron_*` tools are still lightweight wrappers rather than a full task shell.

## Tool surface delta

The most useful way to compare tools is not "which repo has more files."
It is:

- what Claude Code exposes as first-class model tools
- what Narayan already has equivalents for
- what is missing completely
- what is missing only in behavior / policy, not implementation

### Rough equivalents Narayan already has

| Claude Code | Narayan | Assessment |
|---|---|---|
| `BashTool` / `PowerShellTool` | `shell` | Narayan has shell execution, but less policy and less permission depth |
| `FileReadTool` | `file_read` | Roughly equivalent |
| `FileWriteTool` | `file_write` | Roughly equivalent |
| `FileEditTool` | `file_edit` | Roughly equivalent |
| `WebSearchTool` | `web_search_tool` | Roughly equivalent |
| `WebFetchTool` | `web_fetch` | Roughly equivalent |
| `AgentTool` | `delegate` | Narayan equivalent exists, but coordinator logic is much thinner |
| `AskUserQuestionTool` | `ask_user` | Narayan equivalent exists, but less structured UX |
| `SkillTool` | `skill_wrapper` / skills registry | Roughly equivalent at a high level |
| `MCPTool` | `mcp_session` | Narayan supports MCP tool calls, but not resource reads/lists |
| `ScheduleCronTool` | `schedule` + `cron_*` | Narayan has primitives, but not a full task shell |
| `ToolSearchTool` | `request_more_tools` | Narayan has category expansion, but not deferred per-tool schema loading |

### Claude Code tools Narayan does not have, or only has very thinly

| Claude Code tool / concept | Narayan status | Why it matters |
|---|---|---|
| `TaskCreateTool` / `TaskGetTool` / `TaskListTool` / `TaskUpdateTool` / `TaskStopTool` / `TaskOutputTool` | Missing as a first-class shell task system | Gives the model durable, inspectable work state beyond raw plan steps |
| `SendMessageTool` | Thin / indirect equivalent only | Enables teammate-to-teammate messaging and cross-session coordination |
| `TeamCreateTool` / `TeamDeleteTool` | Missing | Makes swarm/team topology explicit rather than ad hoc |
| `EnterWorktreeTool` / `ExitWorktreeTool` | Missing | Supports safe isolated edits and branch/worktree separation |
| `ListMcpResourcesTool` / `ReadMcpResourceTool` | Missing | Important for grounded use of MCP beyond callable tools |
| `LSPTool` | Missing | Gives definition/reference/symbol intelligence without shell hacks |
| `NotebookEditTool` | Missing | Better notebook-native edits than text patching |
| `REPLTool` | Missing | Interactive runtime loop for certain workflows |
| `RemoteTriggerTool` | Missing | Better remote event / wakeup / control integration |
| `TodoWriteTool` | Missing | Lightweight local task tracking even outside the full task graph |
| `SleepTool` | Missing | Supports autonomous tick-based pacing and long-lived agents |
| `WorkflowTool` | Missing | Exposes reusable workflow scripts as first-class capabilities |
| `BriefTool` / proactive communication helpers | Missing | Narrows text output and changes agent pacing in autonomous mode |

### Areas where Narayan is already broader

Narayan already has many tool categories that are more automation-platform oriented than Claude Code's local CLI shell:

- connector tools
- business system integrations
- browser automation variants
- infra/data tools
- vector / memory primitives
- WASM execution
- API registration / custom connector creation

Relevant local references:

- `src/tools/mod.rs:256`
- `src/tools/request_more_tools.rs:1`
- `src/tools/mcp_session.rs:336`

## How Claude Code uses tools

The more important reverse-engineering insight is not the raw tool names.
It is how the system tells the model to use those tools.

### 1. Tool access is role-scoped

Claude Code does not expose the same tool set to every agent.

Examples:

- async workers get a broad but controlled tool set: `constants/tools.ts:55`
- in-process teammates get task + messaging tools: `constants/tools.ts:77`
- coordinator mode gets only orchestration/output tools: `constants/tools.ts:107`

This matters because it prevents the coordinator from turning into just another full-access worker.
The coordinator stays focused on:

- launching workers
- managing concurrency
- synthesizing findings
- talking to the user

Narayan today has delegation, but not this same level of role-specific tool shaping.

### 2. They use tasks proactively

Claude Code explicitly tells the model to create tasks for:

- complex work
- 3+ step work
- plan mode
- multi-request batches

Evidence:

- `tools/TaskCreateTool/prompt.ts:21`
- `tools/TaskUpdateTool/prompt.ts:16`

This is important because the task system is not just storage.
It is part of the model's operating procedure.

Narayan currently has plan steps and scheduler state, but not this explicit model-facing task workflow.

### 3. They use ToolSearch to lazy-load schema, not just categories

Claude Code defers many tools and only gives the model the full schema when it asks for it.

Evidence:

- `tools/ToolSearchTool/prompt.ts:27`
- `tools/ToolSearchTool/prompt.ts:55`
- `utils/toolSchemaCache.ts:1`

This gives them three advantages:

- smaller initial prompt
- better prompt cache stability
- more precise loading than category-wide expansion

Narayan's `request_more_tools` is directionally similar, but currently expands at the category level rather than loading specific deferred tool schemas:

- `src/tools/request_more_tools.rs:1`

### 4. They use AskUserQuestion as a structured decision UI

Claude Code treats user clarification as a structured UI surface, not just a blocking plain-text question.

Evidence:

- `tools/AskUserQuestionTool/prompt.ts:12`
- `tools/AskUserQuestionTool/prompt.ts:32`

Notable behaviors:

- multiple choice with recommended option
- multi-select
- previews for UI/code comparisons
- clear distinction between clarification and plan approval

Narayan's `ask_user` already has good structure:

- `src/tools/ask_user.rs:7`

But Claude Code pushes this further as a core execution primitive.

### 5. They strictly separate worktree usage from ordinary branching

Claude Code explicitly says:

- only use worktree tool when the user explicitly asks for worktree
- do not treat worktree as generic branching

Evidence:

- `tools/EnterWorktreeTool/prompt.ts:2`

This matters because the model is being taught operational boundaries, not just capabilities.

### 6. They give Bash its own policy prompt

Claude Code's shell tool is not just "run command."
Its prompt tells the model:

- prefer dedicated tools over shell
- run independent commands in parallel
- avoid destructive git commands
- do not skip hooks
- stage specific files
- prefer new commits over amend
- obey sandbox policy

Evidence:

- `tools/BashTool/prompt.ts:1`

This is one of the most important differences between the two systems.
Narayan has shell execution, but much less shell-specific policy guidance.

### 7. They use SendMessage for explicit agent-to-agent protocol

Claude Code treats teammate communication as a first-class tool with specific routing patterns.

Evidence:

- `tools/SendMessageTool/prompt.ts:3`
- `tools/SendMessageTool/prompt.ts:11`

That gives them:

- explicit teammate communication
- cross-session messaging
- formal protocol responses
- less accidental coordination through shared conversation state

### 8. They use memory consolidation as an ongoing maintenance job

Claude Code's `autoDream` is not just "save memories."
It instructs the system to:

- orient to existing memory structure
- gather recent signal
- consolidate into durable topic files
- prune stale or contradictory entries
- convert relative time to absolute time

Evidence:

- `services/autoDream/consolidationPrompt.ts:15`
- `services/autoDream/consolidationPrompt.ts:50`

That is a higher-level memory maintenance strategy than Narayan currently appears to have.

## Prompt architecture

Claude Code's prompt system is one of the biggest things worth copying in spirit.

### What prompts they have

The relevant prompt layers I found are:

- global system prompt assembly: `constants/prompts.ts`
- section caching / cache breaks: `constants/systemPromptSections.ts`
- tool schema cache: `utils/toolSchemaCache.ts`
- coordinator system prompt: `coordinator/coordinatorMode.ts`
- bash policy prompt: `tools/BashTool/prompt.ts`
- task prompts: `tools/TaskCreateTool/prompt.ts`, `tools/TaskUpdateTool/prompt.ts`
- ask-user prompt: `tools/AskUserQuestionTool/prompt.ts`
- deferred-tool prompt: `tools/ToolSearchTool/prompt.ts`
- MCP resource prompts: `tools/ListMcpResourcesTool/prompt.ts`, `tools/ReadMcpResourceTool/prompt.ts`
- worktree prompt: `tools/EnterWorktreeTool/prompt.ts`
- LSP prompt: `tools/LSPTool/prompt.ts`
- messaging prompt: `tools/SendMessageTool/prompt.ts`
- dream consolidation prompt: `services/autoDream/consolidationPrompt.ts`

The big pattern is:

- system prompt provides platform-wide behavior
- each important tool carries its own "how to use this well" policy
- orchestration mode gets a dedicated prompt
- long-lived memory services get their own prompt

### How the main prompt is assembled

Claude Code's system prompt is modular and cached.

Evidence:

- `constants/prompts.ts:114`
- `constants/systemPromptSections.ts:1`

Important traits:

- static and dynamic sections are separated by `SYSTEM_PROMPT_DYNAMIC_BOUNDARY`
- prompt sections are memoized by name
- only selected sections intentionally break cache
- tool schema bytes are cached separately to reduce churn

Evidence:

- `constants/prompts.ts:127`
- `constants/prompts.ts:186`
- `constants/prompts.ts:199`
- `constants/prompts.ts:255`
- `utils/toolSchemaCache.ts:1`

This is a meaningful engineering advantage.
It lets them keep a rich prompt without constantly blowing cache efficiency.

### What the main prompt teaches the model

The system prompt does not just describe the environment.
It teaches a working style:

- tool permission modes exist
- hooks may inject feedback
- tool results may contain prompt injection
- conversation can be summarized automatically
- read code before changing it
- do not create files unnecessarily
- do not over-engineer
- report verification honestly
- be careful with destructive actions

Evidence:

- `constants/prompts.ts:186`
- `constants/prompts.ts:199`
- `constants/prompts.ts:255`

Narayan's prompt stack is already more structured than many agent systems:

- grouped tool manifest
- job-specific planner instructions
- evaluator / reflector / preflight prompts
- tiered history compression

Evidence:

- `src/agent/prompts.rs:8`
- `src/agent/prompts.rs:524`
- `src/agent/prompts.rs:1075`
- `src/agent/prompts.rs:1237`
- `src/agent/prompts.rs:1303`
- `src/agent/prompts.rs:1335`

So Narayan is not starting from zero here.
The gap is more about runtime policy prompts than planner sophistication.

## Logic / algorithm

Claude Code does not appear to rely on some hidden magical algorithm.
The "algorithm" is mostly an operating model implemented through prompts, role scoping, and task state.

### Practical Claude Code operating loop

A reasonable reverse-engineered summary is:

1. Build the system prompt from cached static + dynamic sections.
2. Expose a limited tool set, with some tool schemas deferred.
3. If in coordinator mode, restrict the main agent to orchestration tools only.
4. Spawn workers in parallel for research.
5. Receive worker outputs as structured notifications.
6. Synthesize worker findings into a fresh, self-contained prompt.
7. Continue the same worker or spawn a new one depending on context overlap.
8. Track work in explicit tasks.
9. Run separate verification work instead of trusting the implementation worker.
10. Periodically consolidate memory.

### Coordinator logic

The coordinator prompt makes the orchestration model explicit:

- research -> synthesis -> implementation -> verification
- coordinator synthesizes, workers execute
- workers cannot see the user's conversation
- parallelism is a core strategy
- verification must prove behavior, not just confirm code exists

Evidence:

- `coordinator/coordinatorMode.ts:116`
- `coordinator/coordinatorMode.ts:213`
- `coordinator/coordinatorMode.ts:220`
- `coordinator/coordinatorMode.ts:253`
- `coordinator/coordinatorMode.ts:255`

This is one of the most valuable behaviors to replicate in Narayan.

### Task logic

Claude Code's tasks are not just todos.
They have:

- lifecycle states
- ownership
- dependencies
- update discipline
- explicit honesty rules about completion

Evidence:

- `tools/TaskCreateTool/prompt.ts:21`
- `tools/TaskUpdateTool/prompt.ts:43`

### Deferred-tool logic

ToolSearch exists because Claude Code does not want every tool schema loaded up front.

The practical algorithm is:

- keep some tools deferred
- show only names initially
- fetch JSON schema only when needed
- cache schema bytes for session stability

Evidence:

- `tools/ToolSearchTool/prompt.ts:27`
- `tools/ToolSearchTool/prompt.ts:55`
- `utils/toolSchemaCache.ts:1`

### Memory logic

`autoDream` appears to work like a background compaction / consolidation job:

- inspect memory index
- gather new signal from logs and transcripts
- merge into durable topic files
- prune outdated or contradictory memories

Evidence:

- `services/autoDream/consolidationPrompt.ts:15`

## What Narayan should adapt

The right move is not to copy Claude Code literally.
The right move is to adapt the design patterns that fit Narayan's product direction.

### 1. Build a real permission engine first

This is still the top priority.

Use Claude Code's model as inspiration for:

- permission modes
- risk classes
- file/path policies
- explanation strings for denied actions
- tool-specific risk handling
- shell-specific safety guidance

Narayan touchpoints:

- `src/tools/plane_guard.rs`
- `src/tools/shell.rs`
- `src/tools/file_edit.rs`
- `src/tools/file_write.rs`
- `src/tools/git_operations.rs`

### 2. Turn `delegate` into a true coordinator system

Narayan should add a coordinator mode instead of treating every agent as a near-peer.

Needed changes:

- coordinator-only tool set
- worker-only tool set
- structured worker notifications
- continue-vs-spawn heuristics
- explicit research / implementation / verification phases
- shared scratchpad or team memory

Narayan touchpoints:

- `src/tools/delegate.rs`
- `src/swarm/mod.rs`
- `src/agent/prompts.rs`

### 3. Add first-class task objects

Narayan already has plans and schedulers.
What it does not yet have is a model-facing task shell.

Add:

- create / get / list / update task tools
- owner / dependency / status fields
- clear task prompt guidance
- UI visibility for task progress

This is likely a better next step than trying to copy terminal cosmetics.

### 4. Upgrade prompt architecture, not just prompts

Narayan should borrow the architectural pattern:

- prompt section registry
- static vs dynamic prompt split
- cache-stable prompt assembly
- tool-specific behavior prompts
- separate coordinator prompt

Do not copy text verbatim.
Translate the ideas into Narayan's own operating model.

Narayan already has a good foundation:

- `src/agent/prompts.rs:8`

The next level is runtime policy modularity.

### 5. Replace category-only expansion with deferred tool schemas

Narayan's `request_more_tools` is a good primitive, but it is still coarse.

Adapt the idea into:

- initial tool-name hints
- fetch-schema-on-demand
- per-session schema caching
- tighter prompt footprint

Narayan touchpoints:

- `src/tools/request_more_tools.rs`
- `src/tools/tool_validation.rs`
- `src/tools/mod.rs`

### 6. Expand MCP from tool calls to resources

This is one of the cleanest, highest-value parity upgrades.

Add:

- `list_resources`
- `read_resource`
- maybe cached resource metadata

Narayan touchpoints:

- `src/tools/mcp_session.rs`

### 7. Add LSP and worktree primitives before flashy UX extras

If Narayan wants to become better at coding workflows specifically, these are high leverage:

- LSP navigation
- worktree enter / exit
- scratchpad directory semantics
- notebook-aware editing if notebooks matter

These improve actual engineering behavior more than voice or buddy features.

### 8. Add memory consolidation later

Narayan already has memory primitives.
What is missing is consolidation policy.

Add a background service that:

- summarizes recent runs
- merges repeated facts
- removes stale knowledge
- converts relative dates to absolute dates

Narayan touchpoints:

- `src/memory/store.rs`
- `src/memory/vector.rs`
- `src/skill_evolution/evolution.rs`

## Suggested clean-room build order

If the goal is to get the highest-value Claude-Code-like capabilities into Narayan without taking on unnecessary risk, this is the recommended order:

1. Real permission system
2. Coordinator-grade multi-agent orchestration
3. First-class task shell
4. Prompt section registry + tool-specific policy prompts
5. Deferred tool schema loading
6. MCP resource primitives
7. LSP + worktree + scratchpad primitives
8. Memory consolidation/meta-services
9. Remote bridge and terminal-native polish

## Recommended interpretation

Narayan should not be viewed as "missing Claude Code."

A better framing is:

- Narayan already has the platform/runtime side
- Claude Code is stronger on the local-agent shell side
- Claude Code's biggest hidden advantage is policy and orchestration, not just tool count
- the best path is to selectively build the shell layers that actually strengthen Narayan's product direction

That means prioritizing:

- safety
- task state
- orchestration
- prompt architecture
- grounded tool usage

before spending time on cosmetic or novelty features.

## Clean-room note

This document should be used as a behavioral reverse-engineering reference, not as a copy plan.

Recommended boundary:

- copy patterns
- copy product behavior
- copy architecture ideas
- do not copy source code
- do not copy prompts verbatim
- re-express the policies in Narayan language and Narayan abstractions

## Sources

- Upstream repo: `https://github.com/Kuberwastaken/claude-code?tab=readme-ov-file`
- Upstream README references:
  - `README.md:152`
  - `README.md:168`
  - `README.md:247`
  - `README.md:265`
  - `README.md:327`
  - `README.md:329`
  - `README.md:347`
  - `README.md:418`
- Upstream prompt / orchestration references:
  - `constants/tools.ts:55`
  - `constants/tools.ts:77`
  - `constants/tools.ts:107`
  - `constants/prompts.ts:114`
  - `constants/prompts.ts:127`
  - `constants/prompts.ts:186`
  - `constants/prompts.ts:199`
  - `constants/prompts.ts:255`
  - `constants/prompts.ts:758`
  - `constants/prompts.ts:797`
  - `constants/systemPromptSections.ts:1`
  - `utils/toolSchemaCache.ts:1`
  - `coordinator/coordinatorMode.ts:116`
  - `coordinator/coordinatorMode.ts:213`
  - `coordinator/coordinatorMode.ts:220`
  - `coordinator/coordinatorMode.ts:253`
  - `coordinator/coordinatorMode.ts:255`
- Upstream tool prompt references:
  - `commands.ts:58`
  - `commands.ts:73`
  - `commands.ts:258`
  - `commands.ts:319`
  - `tools/BashTool/prompt.ts:1`
  - `tools/TaskCreateTool/prompt.ts:21`
  - `tools/TaskUpdateTool/prompt.ts:16`
  - `tools/TaskUpdateTool/prompt.ts:43`
  - `tools/AskUserQuestionTool/prompt.ts:12`
  - `tools/AskUserQuestionTool/prompt.ts:32`
  - `tools/ToolSearchTool/prompt.ts:27`
  - `tools/ToolSearchTool/prompt.ts:55`
  - `tools/ListMcpResourcesTool/prompt.ts:1`
  - `tools/ReadMcpResourceTool/prompt.ts:1`
  - `tools/EnterWorktreeTool/prompt.ts:2`
  - `tools/LSPTool/prompt.ts:1`
  - `tools/SendMessageTool/prompt.ts:3`
  - `tools/SendMessageTool/prompt.ts:11`
  - `services/autoDream/consolidationPrompt.ts:15`
  - `services/autoDream/consolidationPrompt.ts:50`
- Narayan local references:
  - `README.md:3`
  - `README.md:25`
  - `src/agent/plan_mode.rs:1`
  - `src/agent/prompts.rs:8`
  - `src/agent/prompts.rs:524`
  - `src/agent/prompts.rs:1075`
  - `src/agent/prompts.rs:1237`
  - `src/agent/prompts.rs:1303`
  - `src/agent/prompts.rs:1335`
  - `src/scheduler/ticker.rs:1`
  - `src/webhooks/delivery.rs:1`
  - `src/tools/mod.rs:256`
  - `src/tools/delegate.rs:53`
  - `src/tools/plane_guard.rs:10`
  - `src/tools/run_registered_wasm.rs:49`
  - `src/swarm/mod.rs:21`
  - `src/tools/acp_session.rs:10`
  - `src/workspace/remote.rs:165`
  - `src/skills/registry.rs:112`
  - `src/skill_marketplace/marketplace.rs:13`
  - `src/segments/registry.rs:1`
  - `src/tools/ask_user.rs:7`
  - `src/tools/request_more_tools.rs:1`
  - `src/tools/tool_validation.rs:11`
  - `src/tools/mcp_session.rs:336`
  - `src/tools/mcp_session.rs:344`
  - `src/tools/schedule.rs:10`
  - `src/tools/cron.rs:29`
