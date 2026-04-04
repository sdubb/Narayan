
The user is saying:

Plan mode is NOT filling up steps correctly - URLs, tools, and for each tool which function to call, what to do with results, etc.
Because of that, the planner is being called to correct it
But the planner is not at plan mode level (it's at execution level, not at planning/configuration level)
So the issue is:

Plan mode should be generating COMPLETE, deterministic workflow steps with all parameters filled in
Instead, it's generating vague hints like "Fetch records from database (requires database connector)"
Then at execution time, the system tries to salvage it by having the LLM planner generate a plan again
But the LLM planner is designed for execution, not for filling in plan mode details
The real problem: Plan mode compilation is incomplete. It should output:

{
  "description": "Fetch records from database (requires database connector)",
  "tool": "data_engine",
  "tool_args": {
    "records": [...],
    "pipeline": [...]
  },
  "success_criteria": "Retrieved abnormal records"
}
But it's only outputting:

{
  "description": "Fetch records from database (requires database connector)",
  "tool": "web_fetch",
  "tool_args": {"url": "{input.url}"}  // <- placeholder!
}
The user is right - the workflow outline compilation phase (enrich_workflow_outline and resolve_tool_for_hint) is shallow. It needs to:

Actually understand the intent JSON structure
Extract concrete parameters (query, filters, etc.)
Generate proper tool_args with real values, not templates
Map results to outputs properly
Let me explain this and figure out what needs to be fixed.

You're absolutely right! This is the core architectural gap. Let me break it down:

The Problem: Plan Mode ≠ Complete Workflow Compilation
What's Happening Now (WRONG)
Then at runtime: Execution planner called to fix it (wrong layer!)
Plan Mode:
┌─ Intent JSON: preferred_tools=["data_engine"], uses_external_db="shub"
│  workflow_outline: ["Fetch records...", "Analyze records...", "Send alert..."]
│
└─> enrich_workflow_outline()
    └─> resolve_tool_for_hint("Fetch records from database...")
        └─> (INCOMPLETE!) Returns:
            {
              "tool": "web_fetch",  // ❌ WRONG (keyword matched)
              "tool_args": {"url": "{input.url}"}  // ❌ PLACEHOLDER
            }
What Should Happen (RIGHT)
Plan Mode:
┌─ Intent JSON: preferred_tools=["data_engine"], uses_external_db="shub"
│  actions: ["monitor database", "detect abnormalities", "send alert"]
│
└─> compile_workflow_with_intent()
    ├─ Extract: tool=data_engine, db=shub
    ├─ Generate: Actual SQL/filter query from intent["actions"]
    ├─ Fill args: {
    │    "records": {query_results},
    │    "pipeline": [
    │      {"op": "filter", "condition": "abnormal_detection"},
    │      {"op": "map", "assign": {"alert": true}}
    │    ]
    │  }
    ├─ Define: success_criteria, result_handling
    └─> COMPLETE step with NO placeholders

The Real Issues
Issue 1: resolve_tool_for_hint() is Too Shallow
plan_mode.rs:3665:

✅ Matches keywords
❌ Does NOT extract concrete parameters from intent JSON
❌ Does NOT know what query to run
❌ Does NOT know what "abnormal" means
❌ Returns placeholder {input.url} instead of real query
Issue 2: enrich_workflow_outline() Lacks Context
plan_mode.rs:3512:

It should also receive and use:

intent["data_sources"] → which database
intent["actions"] → what to detect
intent["output_destination_hint"] → where to send results
intent["trigger_hint"] → when/how triggered
Issue 3: No Result Handling
Steps don't specify what to do with results:

What Needs to Be Fixed
Function: resolve_tool_for_hint() [Line 3665]

Should become resolve_tool_and_args_from_intent() with signature:

This should:

Identify tool from preferred_tools array
Extract concrete parameters from intent (queries, thresholds, etc.)
Generate FULL tool_args with NO {input.*} placeholders
Include success_criteria and result_mapping