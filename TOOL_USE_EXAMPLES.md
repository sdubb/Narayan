# Complete Tool Use Implementation Examples

## 1. Define Tools (Real Narayan System Examples)

### Tool #1: Web Search Tool
```rust
// src/tools/web_search_tool.rs - Real tool in Narayan
pub struct WebSearchTool {
    api_key: Option<String>,
}

#[async_trait]
impl Tool for WebSearchTool {
    fn name(&self) -> &str {
        "web_search_tool"
    }

    fn description(&self) -> &str {
        "Search the web for information. Returns titles, URLs, and snippets for the top results."
    }

    fn parameters_schema(&self) -> Vec<ParameterSchema> {
        vec![
            ParameterSchema::required("query", "string", "Search query"),
            ParameterSchema::optional("count", "integer", "Number of results (default: 10, max: 20)"),
            ParameterSchema::optional("region", "string", "Region code, e.g. 'us' (default: 'us')"),
        ]
    }

    async fn execute(&self, args: serde_json::Value) -> Result<ToolResult> {
        let query = args["query"].as_str().ok_or(anyhow::anyhow!("query required"))?;
        let count = args["count"].as_u64().unwrap_or(10).min(20) as usize;
        
        let results = search_serpapi(query, count, &self.api_key)await?;
        Ok(ToolResult::ok(serde_json::json!({
            "query": query,
            "count": count,
            "results": results
        })))
    }
}
```

### Tool #2: File Read Tool
```rust
// src/tools/file_read.rs - Real tool in Narayan
pub struct FileReadTool;

#[async_trait]
impl Tool for FileReadTool {
    fn name(&self) -> &str {
        "file_read"
    }

    fn description(&self) -> &str {
        "Read the contents of a file. Supports optional line range. Capped at 10 MiB."
    }

    fn parameters_schema(&self) -> Vec<ParameterSchema> {
        vec![
            ParameterSchema::required("path", "string", "Absolute or workspace-relative file path"),
            ParameterSchema::optional("start_line", "integer", "First line to read (1-based)"),
            ParameterSchema::optional("end_line", "integer", "Last line to read (1-based)"),
            ParameterSchema::optional("encoding", "string", "'utf8' (default) or 'base64'"),
        ]
    }

    async fn execute(&self, args: serde_json::Value) -> Result<ToolResult> {
        let path = args["path"].as_str().ok_or(anyhow::anyhow!("path required"))?;
        let start_line = args["start_line"].as_u64().map(|n| n as usize);
        let end_line = args["end_line"].as_u64().map(|n| n as usize);
        
        let content = read_file_content(path, start_line, end_line).await?;
        Ok(ToolResult::ok(serde_json::json!({
            "path": path,
            "content": content,
            "total_lines": count_lines(&content),
            "size_bytes": content.len()
        })))
    }
}
```

---

## 2. Convert Tools to JSON Schema (All Providers)

### WebSearchTool as JSON Schema:
```json
{
  "name": "web_search_tool",
  "description": "Search the web for information. Returns titles, URLs, and snippets for the top results.",
  "parameters": {
    "type": "object",
    "properties": {
      "query": {
        "type": "string",
        "description": "Search query"
      },
      "count": {
        "type": "integer",
        "description": "Number of results (default: 10, max: 20)"
      },
      "region": {
        "type": "string",
        "description": "Region code, e.g. 'us' (default: 'us')"
      }
    },
    "required": ["query"]
  },
  "output_schema": {
    "type": "object",
    "properties": {
      "query": { "type": "string" },
      "count": { "type": "integer" },
      "results": {
        "type": "array",
        "items": {
          "type": "object",
          "properties": {
            "title": { "type": "string" },
            "url": { "type": "string" },
            "snippet": { "type": "string" }
          }
        }
      }
    }
  }
}
```

### FileReadTool as JSON Schema:
```json
{
  "name": "file_read",
  "description": "Read the contents of a file. Supports optional line range. Capped at 10 MiB.",
  "parameters": {
    "type": "object",
    "properties": {
      "path": {
        "type": "string",
        "description": "Absolute or workspace-relative file path"
      },
      "start_line": {
        "type": "integer",
        "description": "First line to read (1-based)"
      },
      "end_line": {
        "type": "integer",
        "description": "Last line to read (1-based)"
      },
      "encoding": {
        "type": "string",
        "enum": ["utf8", "base64"],
        "description": "'utf8' (default) or 'base64'"
      }
    },
    "required": ["path"]
  },
  "output_schema": {
    "type": "object",
    "properties": {
      "path": { "type": "string" },
      "content": { "type": "string" },
      "total_lines": { "type": "integer" },
      "size_bytes": { "type": "integer" }
    }
  }
}
```

---

## 3. OpenAI / Groq Provider - Real Example

### Request Phase:
```json
{
  "model": "gpt-4o",
  "messages": [
    {
      "role": "user",
      "content": "What are the latest developments in Rust language and show me the ARCHITECTURE.md from the Narayan project?"
    }
  ],
  "tools": [
    {
      "type": "function",
      "function": {
        "name": "web_search_tool",
        "description": "Search the web for information. Returns titles, URLs, and snippets for the top results.",
        "parameters": {
          "type": "object",
          "properties": {
            "query": { "type": "string", "description": "Search query" },
            "count": { "type": "integer", "description": "Number of results (default: 10, max: 20)" },
            "region": { "type": "string", "description": "Region code, e.g. 'us'" }
          },
          "required": ["query"]
        }
      }
    },
    {
      "type": "function",
      "function": {
        "name": "file_read",
        "description": "Read the contents of a file or directory.",
        "parameters": {
          "type": "object",
          "properties": {
            "path": { "type": "string", "description": "File path" },
            "start_line": { "type": "integer", "description": "Start line (1-based)" },
            "end_line": { "type": "integer", "description": "End line (1-based)" }
          },
          "required": ["path"]
        }
      }
    }
  ]
}
```

### Response Phase - Model Decides to Call Tools:
```json
{
  "choices": [
    {
      "message": {
        "role": "assistant",
        "content": "I'll search for the latest Rust developments and read your project architecture.",
        "tool_calls": [
          {
            "id": "call_1",
            "type": "function",
            "function": {
              "name": "web_search_tool",
              "arguments": "{\"query\": \"latest developments Rust language 2026\", \"count\": 5}"
            }
          },
          {
            "id": "call_2",
            "type": "function",
            "function": {
              "name": "file_read",
              "arguments": "{\"path\": \"ARCHITECTURE.md\"}"
            }
          }
        ]
      }
    }
  ],
  "usage": {
    "prompt_tokens": 320,
    "completion_tokens": 85
  }
}
```

### Parsing (src/providers/mod.rs):
```rust
let tool_calls = choice["tool_calls"]
    .as_array()
    .unwrap_or(&vec![])
    .iter()
    .filter_map(|tc| {
        let id = tc["id"].as_str()?.to_string();
        let name = tc["function"]["name"].as_str()?.to_string();
        let arguments: serde_json::Value =
            serde_json::from_str(tc["function"]["arguments"].as_str().unwrap_or("{}"))?;
        Some(ToolCall { id, name, arguments })
    })
    .collect();

// Result:
// [
//   ToolCall { 
//     id: "call_1", 
//     name: "web_search_tool", 
//     arguments: { "query": "latest developments Rust language 2026", "count": 5 }
//   },
//   ToolCall { 
//     id: "call_2", 
//     name: "file_read", 
//     arguments: { "path": "ARCHITECTURE.md" }
//   }
// ]
```
                .unwrap_or_default();
        Some(ToolCall { id, name, arguments })
    })
    .collect();
```

---

## 4. Anthropic Provider - Real Example

### Request Phase (Different Format):
```json
{
  "model": "claude-opus-4-20250514",
  "max_tokens": 4096,
  "system": "You are a helpful assistant that can search the web and read files within the Narayan project.",
  "messages": [
    {
      "role": "user",
      "content": "What are the latest developments in Rust language and show me the ARCHITECTURE.md from the Narayan project?"
    }
  ],
  "tools": [
    {
      "name": "web_search_tool",
      "description": "Search the web for information. Returns titles, URLs, and snippets for the top results.",
      "input_schema": {
        "type": "object",
        "properties": {
          "query": { "type": "string", "description": "Search query" },
          "count": { "type": "integer", "description": "Number of results (default: 10, max: 20)" },
          "region": { "type": "string", "description": "Region code" }
        },
        "required": ["query"]
      }
    },
    {
      "name": "file_read",
      "description": "Read the contents of a file or directory.",
      "input_schema": {
        "type": "object",
        "properties": {
          "path": { "type": "string", "description": "File path" },
          "start_line": { "type": "integer", "description": "Start line (1-based)" },
          "end_line": { "type": "integer", "description": "End line (1-based)" }
        },
        "required": ["path"]
      }
    }
  ]
}
```

### Response Phase - Anthropic Returns tool_use Blocks:
```json
{
  "content": [
    {
      "type": "text",
      "text": "I'll search for the latest Rust developments and retrieve your project architecture."
    },
    {
      "type": "tool_use",
      "id": "toolu_01a8aUCYKZXw52UJRtU7Gvll",
      "name": "web_search_tool",
      "input": {
        "query": "latest developments Rust language 2026",
        "count": 5
      }
    },
    {
      "type": "tool_use",
      "id": "toolu_01eV6CvSJ9q4M5Xk2NpLwZ3Q",
      "name": "file_read",
      "input": {
        "path": "ARCHITECTURE.md"
      }
    }
  ],
  "usage": {
    "input_tokens": 350,
    "output_tokens": 120
  }
}
```

### Parsing (src/providers/mod.rs - AnthropicProvider):
```rust
let tool_calls = resp["content"]
    .as_array()
    .unwrap_or(&vec![])
    .iter()
    .filter_map(|content| {
        if content["type"].as_str() == Some("tool_use") {
            let id = content["id"].as_str()?.to_string();
            let name = content["name"].as_str()?.to_string();
            let input = content["input"].clone(); // Note: input is JSON object, not string
            Some(ToolCall { id, name, arguments: input })
        } else {
            None
        }
    })
    .collect();

// Result:
// [
//   ToolCall {
//     id: "toolu_01a8aUCYKZXw52UJRtU7Gvll",
//     name: "web_search_tool",
//     arguments: { "query": "latest developments Rust language 2026", "count": 5 }
//   },
//   ToolCall {
//     id: "toolu_01eV6CvSJ9q4M5Xk2NpLwZ3Q",
//     name: "file_read",
//     arguments: { "path": "ARCHITECTURE.md" }
//   }
// ]
```

### Key Difference from OpenAI:
- **OpenAI**: `arguments` is a **JSON string** - needs `.as_str().unwrap_or("{}")` then `serde_json::from_str()`
- **Anthropic**: `input` is already a **JSON object** - use directly as `.arguments`

---

## 5. Groq Provider - Real Example

### Request Phase (OpenAI-Compatible):
```json
{
  "model": "llama-3.3-70b-versatile",
  "messages": [
    {
      "role": "user",
      "content": "What are the latest developments in Rust language and show me the ARCHITECTURE.md from the Narayan project?"
    }
  ],
  "tools": [
    {
      "type": "function",
      "function": {
        "name": "web_search_tool",
        "description": "Search the web for information. Returns titles, URLs, and snippets for the top results.",
        "parameters": {
          "type": "object",
          "properties": {
            "query": { "type": "string", "description": "Search query" },
            "count": { "type": "integer", "description": "Number of results (default: 10, max: 20)" }
          },
          "required": ["query"]
        }
      }
    },
    {
      "type": "function",
      "function": {
        "name": "file_read",
        "description": "Read the contents of a file or directory.",
        "parameters": {
          "type": "object",
          "properties": {
            "path": { "type": "string", "description": "File path" }
          },
          "required": ["path"]
        }
      }
    }
  ]
}
```

**Endpoint**: `https://api.groq.com/openai/v1/chat/completions`

### Response Phase - Same as OpenAI:
```json
{
  "choices": [
    {
      "message": {
        "role": "assistant",
        "content": "I'll search and retrieve the information for you.",
        "tool_calls": [
          {
            "id": "call_groq_1",
            "type": "function",
            "function": {
              "name": "web_search_tool",
              "arguments": "{\"query\": \"latest developments Rust language 2026\", \"count\": 5}"
            }
          },
          {
            "id": "call_groq_2",
            "type": "function",
            "function": {
              "name": "file_read",
              "arguments": "{\"path\": \"ARCHITECTURE.md\"}"
            }
          }
        ]
      }
    }
  ]
}
```

### Parsing (src/providers/groq_impl.rs):
```rust
// Same parsing as OpenAI - format is identical!
let tool_calls = choice["tool_calls"]
    .as_array()
    .unwrap_or(&vec![])
    .iter()
    .filter_map(|tc| {
        let id = tc["id"].as_str()?.to_string();
        let name = tc["function"]["name"].as_str()?.to_string();
        let arguments: serde_json::Value =
            serde_json::from_str(tc["function"]["arguments"].as_str().unwrap_or("{}"))?;
        Some(ToolCall { id, name, arguments })
    })
    .collect();
```

---

## 6. Gemini Provider - Real Example

### Request Phase (Native Google Format):
```json
{
  "contents": [
    {
      "role": "user",
      "parts": [
        {
          "text": "What are the latest developments in Rust language and show me the ARCHITECTURE.md from the Narayan project?"
        }
      ]
    }
  ],
  "systemInstruction": {
    "parts": [
      {
        "text": "You are a helpful assistant that can search the web and read files."
      }
    ]
  },
  "tools": [
    {
      "functionDeclarations": [
        {
          "name": "web_search_tool",
          "description": "Search the web for information. Returns titles, URLs, and snippets for the top results.",
          "parameters": {
            "type": "object",
            "properties": {
              "query": { "type": "string", "description": "Search query" },
              "count": { "type": "integer", "description": "Number of results (default: 10, max: 20)" }
            },
            "required": ["query"]
          }
        },
        {
          "name": "file_read",
          "description": "Read the contents of a file or directory.",
          "parameters": {
            "type": "object",
            "properties": {
              "path": { "type": "string", "description": "File path" }
            },
            "required": ["path"]
          }
        }
      ]
    }
  ]
}
```

**Endpoint**: `https://generativelanguage.googleapis.com/v1beta/models/{model}:generateContent?key={api_key}`

### Response Phase - Gemini Returns functionCall Blocks:
```json
{
  "candidates": [
    {
      "content": {
        "parts": [
          {
            "text": "I'll search for the latest Rust developments and retrieve your architecture file."
          },
          {
            "functionCall": {
              "name": "web_search_tool",
              "args": {
                "query": "latest developments Rust language 2026",
                "count": 5
              }
            }
          },
          {
            "functionCall": {
              "name": "file_read",
              "args": {
                "path": "ARCHITECTURE.md"
              }
            }
          }
        ]
      }
    }
  ]
}
```

### Parsing (src/providers/gemini_impl.rs):
```rust
let tool_calls = resp["candidates"][0]["content"]["parts"]
    .as_array()
    .unwrap_or(&vec![])
    .iter()
    .filter_map(|part| {
        let func_call = &part["functionCall"];
        if func_call.is_object() {
            let name = func_call["name"].as_str()?.to_string();
            let arguments = func_call["args"].clone(); // Note: args is JSON object, not string
            // Generate unique ID for tool call
            let id = format!("gemini-fc-{}", chrono::Utc::now().timestamp_nanos_opt().unwrap_or(0));
            Some(ToolCall { id, name, arguments })
        } else {
            None
        }
    })
    .collect();

// Result:
// [
//   ToolCall {
//     id: "gemini-fc-1711929600000000000",
//     name: "web_search_tool",
//     arguments: { "query": "latest developments Rust language 2026", "count": 5 }
//   },
//   ToolCall {
//     id: "gemini-fc-1711929600000000001",
//     name: "file_read",
//     arguments: { "path": "ARCHITECTURE.md" }
//   }
// ]
```

### Key Differences:
- **Format**: `functionDeclarations` array inside a `tools` object (unique to Gemini)
- **Response**: `functionCall.args` is **JSON object** (like Anthropic)
- **Tool Call ID**: Generated by application (Gemini doesn't provide one)
- **Message Structure**: Uses `contents[].parts[]` instead of `messages[].content`

## 7. Complete Executor Flow

```rust
// src/agent/executor.rs
async fn execute_step_with_tools(
    provider: Arc<dyn Provider>,
    messages: Vec<Message>,
    tool_registry: &ToolRegistry,
) -> Result<StepResult> {
    // STEP 1: Prepare tool specs from registry (web_search_tool, file_read, sql_query, etc.)
    let tools = [
        "web_search_tool",
        "file_read",
        "sql_query",
        "memory_store",
    ]
    .iter()
    .filter_map(|name| tool_registry.get_spec(*name))
    .collect();
    
    // STEP 2: Send to model with available tools
    tracing::info!("Calling provider: {}, available tools: {:?}", 
        provider.name(), 
        tools.iter().map(|t| t.name.clone()).collect::<Vec<_>>());
    
    let response = provider.chat(messages.clone(), tools).await?;
    
    // STEP 3: Check if model called any tools
    if response.tool_calls.is_empty() {
        tracing::info!("Model returned no tool calls - returning direct answer");
        return Ok(StepResult {
            success: true,
            output: response.content.unwrap_or_default(),
            tool_results: vec![],
            tools_called: vec![],
        });
    }
    
    // STEP 4: Execute each tool call
    tracing::info!("Model called {} tools", response.tool_calls.len());
    let mut tool_results = Vec::new();
    let mut tools_called = Vec::new();
    
    for tool_call in &response.tool_calls {
        tools_called.push(tool_call.name.clone());
        tracing::info!("Executing tool: {} (id: {})", tool_call.name, tool_call.id);
        
        // Find and execute the tool
        if let Some(tool) = tool_registry.get(&tool_call.name) {
            match tool.execute(tool_call.arguments.clone()).await {
                Ok(result) => {
                    tracing::info!("Tool {} executed successfully", tool_call.name);
                    tool_results.push(result);
                }
                Err(e) => {
                    tracing::error!("Tool {} failed: {}", tool_call.name, e);
                    tool_results.push(ToolResult::err(format!("Tool execution failed: {}", e)));
                }
            }
        } else {
            tracing::warn!("Tool '{}' not found in registry", tool_call.name);
            tool_results.push(ToolResult::err(format!("Tool '{}' not found", tool_call.name)));
        }
    }
    
    // STEP 5: Send results back to model for final response
    let mut updated_messages = messages.clone();
    
    // Add assistant message with tool calls
    updated_messages.push(Message {
        role: Role::Assistant,
        content: response.content.unwrap_or_default(),
    });
    
    // Add tool results (format varies by provider, but Handler normalizes it)
    for (tool_call, result) in response.tool_calls.iter().zip(&tool_results) {
        updated_messages.push(Message {
            role: Role::Tool,
            tool_name: Some(tool_call.name.clone()),
            tool_call_id: Some(tool_call.id.clone()),
            content: serde_json::to_string(&result)?,
        });
    }
    
    // STEP 6: Get final response from model with tool results
    tracing::info!("Requesting final response from model with {} tool results", 
        tool_results.len());
    let final_response = provider.chat(updated_messages, vec![]).await?;
    
    Ok(StepResult {
        success: true,
        output: final_response.content.unwrap_or_default(),
        tool_results,
        tools_called,
    })
}
```

### Example Execution Trace:
```
INFO Calling provider: gpt-4o, available tools: 
      ["web_search_tool", "file_read", "sql_query", "memory_store"]
INFO Model called 2 tools
INFO Executing tool: web_search_tool (id: call_1)
INFO Tool web_search_tool executed successfully
INFO Executing tool: file_read (id: call_2)
INFO Tool file_read executed successfully
INFO Requesting final response from model with 2 tool results
INFO Agent step completed successfully with tools_called: 
      ["web_search_tool", "file_read"]
```

---

## 8. Provider Comparison Matrix

### How Each Provider Handles Narayan Tools

| Aspect | OpenAI / Groq | Anthropic | Gemini | OpenRouter |
|--------|---|---|---|---|
| **Tool Request Format** | `"tools": [{ "type": "function", "function": {...} }]` | `"tools": [{ "name": "...", "input_schema": {...} }]` | `"tools": [{ "functionDeclarations": [...] }]` | OpenAI passthrough |
| **Arguments in Request** | String (JSON serialized) | JSON object | Object properties | String (JSON serialized) |
| **Response tool_calls** | `choice.message.tool_calls[]` | `content[].tool_use` blocks | `content[].parts[].functionCall` | `choice.message.tool_calls[]` |
| **Arguments in Response** | String (JSON serialized) | JSON object (input) | JSON object (args) | String (JSON serialized) |
| **Tool Call ID** | Model-provided (`call_*`) | Model-provided (`toolu_*`) | Generated by app (timestamp) | Model-provided |
| **Example Tools** | web_search_tool, file_read | web_search_tool, file_read | web_search_tool, file_read | web_search_tool, file_read |
| **Testability** | High (free tier available) | High (free tier available) | Medium (requires API key) | High (auto-transforms) |

### Real Narayan Tool: web_search_tool
```
Request:  { "query": "latest Rust developments", "count": 5 }
Response: { 
  "query": "latest Rust developments",
  "count": 5,
  "results": [
    { "title": "Rust 1.76 Release", "url": "https://...", "snippet": "..." },
    ...
  ]
}
```

### Real Narayan Tool: file_read
```
Request:  { "path": "ARCHITECTURE.md", "start_line": 1, "end_line": 50 }
Response: {
  "path": "ARCHITECTURE.md",
  "content": "# Narayan Architecture\n...",
  "total_lines": 150,
  "size_bytes": 5240
}
```

---

## 9. Implementation Checklist for New Tools

To add a new tool to Narayan and make it available to all LLM providers:

```rust
// 1. Create new tool in src/tools/
pub struct MyNewTool;

#[async_trait]
impl Tool for MyNewTool {
    fn name(&self) -> &str {
        "my_new_tool"
    }
    
    fn description(&self) -> &str {
        "Description for LLM guidance"
    }
    
    fn parameters_schema(&self) -> Vec<ParameterSchema> {
        vec![
            ParameterSchema::required("param1", "string", "Required parameter"),
            ParameterSchema::optional("param2", "integer", "Optional parameter"),
        ]
    }
    
    async fn execute(&self, args: serde_json::Value) -> Result<ToolResult> {
        // Implementation
        Ok(ToolResult::ok(serde_json::json!({"result": "success"})))
    }
}

// 2. Register in tool_registry (once added, automatically available to ALL providers)
tool_registry.register(Arc::new(MyNewTool));

// 3. Test with each provider:
// - OpenAI: Arguments are JSON strings
// - Anthropic: Input is JSON objects
// - Gemini: Args are JSON objects  
// - Groq: Arguments are JSON strings (like OpenAI)
// - All providers handle errors from tool.execute() the same way
```

---

## 10. Common Pitfalls and Debugging

### Issue: Tool not called by model
**Cause**: Tool description is too vague or parameters are overly complex
**Fix**: Review `description()` and `parameters_schema()` - be specific about when the tool should be used

### Issue: "Tool X not found" error
**Cause**: Tool name mismatch between request and registry
**Debug**:
```rust
// Check available tools
for spec in &tools {
    println!("Available: {}", spec.name);
}
// Check what model tried to call
for call in &response.tool_calls {
    println!("Model called: {}", call.name);
}
```

### Issue: Anthropic returns tool_use but OpenAI returns no tool_calls
**Cause**: Tool definitions differ between providers
**Fix**: Ensure JSON schemas are identical - use helper functions to generate once
