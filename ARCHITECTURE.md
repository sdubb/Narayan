# Plan Mode Runtime Split

Narayan keeps plan mode and runtime separated, but they share one registry contract.

## Recommended Split

### 1. Registry Search
- Start with the user request and a small planning tool surface.
- Let the LLM ask the backend for narrow registry searches when it needs grounding.
- Return only the connectors, MCP servers, ACP peers, and tool categories that match the query.
- Keep this step small, narrow, and deterministic.

### 2. Plan-Mode Workflow Synthesis
- Let the LLM produce structured intent, clarification steps, and a typed `workflow_dsl`.
- Prefer explicit tool names, operations, resource IDs, and read-only flags from the returned search results.
- Keep this stage focused on drafting, not execution.

### 3. Compiler Validation
- Validate the draft workflow against the live registry and available connectors.
- Reject unsupported tools or missing bindings early.
- Ask for missing cards or connections when required.

### 4. Runtime Dispatch
- Execute the compiled workflow through the DAG engine and step orchestrator.
- Use the registry again here as the source of truth for actual tool dispatch.
- Persist tool results, errors, retries, and workflow state transitions.

### 5. Agent MCP Exposure
- Expose the agent’s live tool registry over a small MCP-compatible endpoint.
- Publish the agent tool manifest as a resource.
- Publish per-agent snapshots as read-only MCP resources.
- Provide a small prompt catalog for external MCP clients.
- Offer a lightweight SSE handshake endpoint alongside the JSON-RPC POST surface.
- Let external MCP clients list tools and call them through the existing runtime.
- Keep this separate from MCP client consumption, which remains the agent’s integration path into external services.

## Why This Split Works
- The LLM gets grounded planning inputs.
- The backend stays the source of truth for what exists.
- Workflow execution remains deterministic and auditable.
- Tool registration and execution can evolve independently without breaking plan mode.
- The agent can also be surfaced as a tool server for other MCP clients without changing the runtime core.
