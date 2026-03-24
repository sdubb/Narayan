# Narayan — Autonomous AI Employee Platform

Narayan is a **distributed AI employee platform** capable of running **millions of autonomous agents** that perform real-world jobs end-to-end.

## Architecture

Agents are implemented as **state machines + scheduler**, not long-running processes.

```
scheduler wakes agent → worker loads state → agent executes one step → state saved → reschedule
```

## Quick Start

```bash
cp .env.example .env   # configure DATABASE_URL, REDIS_URL, etc.
cargo build --release
./target/release/narayan
```

## Scaling

| Deployment         | Agent Steps/sec |
|--------------------|-----------------|
| 1 worker node      | ~5,000          |
| 100 worker nodes   | ~500,000        |

## Supported Providers

`anthropic` · `openai` · `gemini` · `ollama` · `openrouter` · `openai_codex` · `glm` · `novita` · `sglang` · `router`

## Tool Categories

- **Filesystem**: file_read, file_write, file_edit, glob_search, git_operations
- **Web**: browser, web_fetch, web_search_tool, http_request, screenshot
- **Memory**: memory_store, memory_recall, memory_forget
- **Automation**: schedule, workflow, delegate, cron_*
- **Integrations**: api_call, request_credential, mcp_session, email
- **System**: shell, data_extractor, image_info
# Narayan
