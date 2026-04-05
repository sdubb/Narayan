// src/tools/registry.rs
//
// Single source of truth for all tool metadata.
// The compiler reads this to bind DSL steps → tools deterministically.
// The runtime reads this to validate execution.
// No tool knowledge lives in workflow_compiler.rs or plan_mode.rs.

// ─────────────────────────────────────────────────────────────────────────────
// Core types
// ─────────────────────────────────────────────────────────────────────────────

/// The 8 allowed DSL step types from ARCHITECTURE2.md.
/// Plan mode must only emit these. The compiler maps each one to a tool family.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DslStepType {
    FetchRecords,   // retrieve data from any source
    Filter,         // narrow a record set
    Compute,        // derive new fields / score / formula
    Aggregate,      // group-by, count, sum, roll-up
    DetectAnomaly,  // pattern match, threshold check, outlier detection
    Branch,         // conditional routing — tool: None (structural only)
    Notify,         // send a message, alert, or notification
    StoreResult,    // persist output to memory, file, db, or workspace
}

/// Top-level tool family. Narrows 73 tools to ~5-10 per DSL step type.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ToolFamily {
    Web,            // web_search_tool, web_fetch, browser_*, http_request, screenshot
    Database,       // external_db, sql_query, vector_store, vector_search, vector_delete
    Transform,      // data_engine, data_extractor, spreadsheet_*, image_*, pdf_*
    Connector,      // connector_tool (28 SaaS connectors), external_api, api_call
    Notification,   // email, notification, pushover, send_message
    Storage,        // file_read, file_write, file_edit, glob_search, content_search, compress, decompress
    Memory,         // memory_store, memory_recall, memory_forget, memory_consolidate
    Code,           // code_run, shell, git_operations, diff_patch, ssh_exec, wasm, docker, kubernetes
    Scheduling,     // cron_*, schedule, delegate
    Security,       // crypto_tool, request_credential, plane_guard
    Meta,           // ask_user, tool_search, request_more_tools, model_routing, etc.
}

/// What kind of external resource binding the tool requires.
/// None = no external resource needed (compiler never emits ask_user for these).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ResourceKind {
    Database,       // postgres, mysql, sqlite connection
    HttpEndpoint,   // URL or API base URL
    Connector,      // named SaaS connector (salesforce, slack, etc.)
    AcpPeer,        // ACP peer or internal agent endpoint
    FileSystem,     // local or mounted path
    ApiKey,         // arbitrary secret (SMTP, webhook URL, etc.)
    SshHost,        // SSH connection
    DockerDaemon,   // Docker socket
    KubeCluster,    // Kubernetes cluster
    McpServer,      // MCP server URL
}

pub struct ToolRegistryEntry {
    // ── Identity ──────────────────────────────────────────────────────────
    pub name: &'static str,
    pub version: &'static str,
    pub description: &'static str,

    // ── Tier 2 tags: what the compiler filters on ─────────────────────────
    pub family: ToolFamily,

    /// Which DSL step types this tool can serve.
    /// Must be non-empty for every executable tool.
    /// Structural-only tools (Branch) have tool: None and are never in registry.
    pub dsl_step_types: &'static [DslStepType],

    /// The exact operation strings this tool accepts.
    /// Plan mode must emit one of these in the DSL step `operation` field.
    /// The compiler validates the operation is in this list before binding.
    pub operations: &'static [&'static str],

    // ── Resource requirements ─────────────────────────────────────────────
    /// None = tool works without any external resource.
    /// Some(kind) = compiler emits ask_user card_open if resource not bound.
    pub requires_resource: Option<ResourceKind>,

    // ── Policy ────────────────────────────────────────────────────────────
    pub read_only: bool,        // tool never mutates external state
    pub requires_approval: bool, // runtime must pause for human approval

    // ── Binding priority ──────────────────────────────────────────────────
    /// Lower number = preferred when multiple tools match all filters.
    pub priority: u8,
}

// ─────────────────────────────────────────────────────────────────────────────
// Registry entries — all 73 tools
// ─────────────────────────────────────────────────────────────────────────────

pub static TOOL_REGISTRY: &[ToolRegistryEntry] = &[

    // ── WEB ──────────────────────────────────────────────────────────────────

    ToolRegistryEntry {
        name: "web_search_tool",
        version: "1.0",
        description: "Search the web. Returns titles, URLs, and snippets for top results.",
        family: ToolFamily::Web,
        dsl_step_types: &[DslStepType::FetchRecords],
        operations: &[
            "search",    // general keyword search
            "discover",  // broad topic discovery
        ],
        requires_resource: None,
        read_only: true,
        requires_approval: false,
        priority: 1,
    },

    ToolRegistryEntry {
        name: "web_fetch",
        version: "1.0",
        description: "Fetch the content of a known URL. Returns extracted text or raw HTML.",
        family: ToolFamily::Web,
        dsl_step_types: &[DslStepType::FetchRecords],
        operations: &[
            "fetch",      // fetch URL, return plain text (HTML stripped)
            "fetch_raw",  // fetch URL, return full HTML
        ],
        requires_resource: None,
        read_only: true,
        requires_approval: false,
        priority: 2,
    },

    ToolRegistryEntry {
        name: "browser_open",
        version: "1.0",
        description: "Open a URL in a headless browser to verify reachability or load JS-rendered pages.",
        family: ToolFamily::Web,
        dsl_step_types: &[DslStepType::FetchRecords],
        operations: &[
            "open",            // open URL and return rendered content
            "check_reachable", // verify URL responds with 2xx
        ],
        requires_resource: None,
        read_only: true,
        requires_approval: false,
        priority: 3,
    },

    ToolRegistryEntry {
        name: "browser_interact",
        version: "1.0",
        description: "Interact with a web page in a headless browser: click, type, navigate, extract.",
        family: ToolFamily::Web,
        dsl_step_types: &[DslStepType::FetchRecords, DslStepType::Compute],
        operations: &[
            "click",    // click an element by selector
            "type",     // type text into a field
            "select",   // select a dropdown option
            "navigate", // go to a URL
            "extract",  // extract content from current page
        ],
        requires_resource: None,
        read_only: false,
        requires_approval: false,
        priority: 4,
    },

    ToolRegistryEntry {
        name: "browser_network",
        version: "1.0",
        description: "Intercept and monitor browser network traffic.",
        family: ToolFamily::Web,
        dsl_step_types: &[DslStepType::FetchRecords, DslStepType::DetectAnomaly],
        operations: &[
            "intercept", // intercept requests matching a pattern
            "monitor",   // passively record all network activity
        ],
        requires_resource: None,
        read_only: true,
        requires_approval: false,
        priority: 5,
    },

    ToolRegistryEntry {
        name: "browser_pdf",
        version: "1.0",
        description: "Print a browser page to PDF.",
        family: ToolFamily::Web,
        dsl_step_types: &[DslStepType::StoreResult],
        operations: &[
            "print_to_pdf", // render current page as PDF bytes
        ],
        requires_resource: None,
        read_only: true,
        requires_approval: false,
        priority: 5,
    },

    ToolRegistryEntry {
        name: "http_request",
        version: "1.0",
        description: "Make arbitrary HTTP requests. Use for APIs that don't have a dedicated connector.",
        family: ToolFamily::Web,
        dsl_step_types: &[DslStepType::FetchRecords, DslStepType::StoreResult],
        operations: &[
            "get",    // HTTP GET
            "post",   // HTTP POST
            "put",    // HTTP PUT
            "patch",  // HTTP PATCH
            "delete", // HTTP DELETE
            "head",   // HTTP HEAD (check existence)
        ],
        requires_resource: Some(ResourceKind::HttpEndpoint),
        read_only: false,
        requires_approval: false,
        priority: 3,
    },

    ToolRegistryEntry {
        name: "screenshot",
        version: "1.0",
        description: "Take a screenshot of a URL or the current browser state.",
        family: ToolFamily::Web,
        dsl_step_types: &[DslStepType::FetchRecords],
        operations: &[
            "screenshot", // capture viewport as PNG
        ],
        requires_resource: None,
        read_only: true,
        requires_approval: false,
        priority: 5,
    },

    // ── DATABASE ──────────────────────────────────────────────────────────────

    ToolRegistryEntry {
        name: "external_db",
        version: "1.0",
        description: "Query or inspect an external database (Postgres, MySQL) connected by the tenant.",
        family: ToolFamily::Database,
        dsl_step_types: &[DslStepType::FetchRecords, DslStepType::Aggregate, DslStepType::StoreResult],
        operations: &[
            "schema",        // list all tables and column types
            "query",         // SELECT data (read-only)
            "execute",       // INSERT / UPDATE / DELETE (write, if enabled)
            "table_preview", // return first N rows of a table
            "explain",       // return the query execution plan
        ],
        requires_resource: Some(ResourceKind::Database),
        read_only: false, // execute op can write; validated at runtime by allow_writes flag
        requires_approval: false,
        priority: 1,
    },

    ToolRegistryEntry {
        name: "sql_query",
        version: "1.0",
        description: "Execute a SQL query against Postgres, MySQL, or SQLite. Store the connection string first.",
        family: ToolFamily::Database,
        dsl_step_types: &[DslStepType::FetchRecords, DslStepType::Aggregate],
        operations: &[
            "select", // read rows
            "insert", // insert rows
            "update", // update rows
            "delete", // delete rows
            "create", // DDL: create table
        ],
        requires_resource: Some(ResourceKind::Database),
        read_only: false,
        requires_approval: false,
        priority: 2,
    },

    ToolRegistryEntry {
        name: "vector_store",
        version: "1.0",
        description: "Embed text and store it in the agent's semantic memory (pgvector).",
        family: ToolFamily::Database,
        dsl_step_types: &[DslStepType::StoreResult],
        operations: &[
            "store",  // embed and store a new document
            "upsert", // embed and upsert by key
        ],
        requires_resource: None,
        read_only: false,
        requires_approval: false,
        priority: 1,
    },

    ToolRegistryEntry {
        name: "vector_search",
        version: "1.0",
        description: "Search agent semantic memory using a natural language query.",
        family: ToolFamily::Database,
        dsl_step_types: &[DslStepType::FetchRecords, DslStepType::DetectAnomaly],
        operations: &[
            "search", // cosine similarity search over stored documents
            "query",  // alias for search with explicit top-k
        ],
        requires_resource: None,
        read_only: true,
        requires_approval: false,
        priority: 1,
    },

    ToolRegistryEntry {
        name: "vector_delete",
        version: "1.0",
        description: "Delete documents from semantic memory by ID or clear all memory.",
        family: ToolFamily::Database,
        dsl_step_types: &[DslStepType::StoreResult],
        operations: &[
            "delete",    // delete a specific document by ID
            "clear_all", // clear all memory for this agent
        ],
        requires_resource: None,
        read_only: false,
        requires_approval: true,
        priority: 2,
    },

    // ── TRANSFORM / DATA ──────────────────────────────────────────────────────

    ToolRegistryEntry {
        name: "data_engine",
        version: "1.0",
        description: "Deterministic record pipeline: filter, map, compute, clean, rank, aggregate. No side effects.",
        family: ToolFamily::Transform,
        dsl_step_types: &[
            DslStepType::Filter,
            DslStepType::Compute,
            DslStepType::Aggregate,
            DslStepType::DetectAnomaly,
        ],
        operations: &[
            "transform_records",       // full pipeline mode (multi-step)
            "filter",                  // keep records matching condition
            "map",                     // reshape or rename fields
            "compute_formula",         // derive new field from formula
            "clean_data",              // normalize, dedupe, fill nulls
            "apply_rules",             // evaluate rule set against records
            "rank_items",              // score and sort records
            "aggregate_records",       // group-by, count, sum, avg, min, max
            "extract_structured_data", // pull typed fields from free-form text
        ],
        requires_resource: None,
        read_only: true,
        requires_approval: false,
        priority: 1,
    },

    ToolRegistryEntry {
        name: "data_extractor",
        version: "1.0",
        description: "Extract structured fields from HTML, PDF-like text, or unstructured content.",
        family: ToolFamily::Transform,
        dsl_step_types: &[DslStepType::FetchRecords, DslStepType::Compute],
        operations: &[
            "extract",       // extract fields matching a schema from text/HTML
            "extract_table", // extract a table structure from HTML or text
        ],
        requires_resource: None,
        read_only: true,
        requires_approval: false,
        priority: 2,
    },

    ToolRegistryEntry {
        name: "spreadsheet_read",
        version: "1.0",
        description: "Read rows from a local spreadsheet (xlsx, csv).",
        family: ToolFamily::Transform,
        dsl_step_types: &[DslStepType::FetchRecords],
        operations: &[
            "read",       // read all rows
            "read_sheet", // read a specific sheet by name
            "list_sheets",// list available sheet names
        ],
        requires_resource: Some(ResourceKind::FileSystem),
        read_only: true,
        requires_approval: false,
        priority: 2,
    },

    ToolRegistryEntry {
        name: "spreadsheet_write",
        version: "1.0",
        description: "Write rows to a local spreadsheet (xlsx, csv).",
        family: ToolFamily::Transform,
        dsl_step_types: &[DslStepType::StoreResult],
        operations: &[
            "write",        // overwrite or create spreadsheet
            "append_rows",  // append rows to existing sheet
        ],
        requires_resource: Some(ResourceKind::FileSystem),
        read_only: false,
        requires_approval: false,
        priority: 2,
    },

    ToolRegistryEntry {
        name: "image_info",
        version: "1.0",
        description: "Read metadata from an image file (dimensions, format, EXIF).",
        family: ToolFamily::Transform,
        dsl_step_types: &[DslStepType::FetchRecords],
        operations: &[
            "info", // return image dimensions, format, EXIF metadata
        ],
        requires_resource: Some(ResourceKind::FileSystem),
        read_only: true,
        requires_approval: false,
        priority: 3,
    },

    ToolRegistryEntry {
        name: "image_process",
        version: "1.0",
        description: "Transform images: rotate, flip, crop, resize.",
        family: ToolFamily::Transform,
        dsl_step_types: &[DslStepType::Compute],
        operations: &[
            "rotate", // rotate by degrees
            "flip",   // flip horizontal or vertical
            "crop",   // crop to bounding box
            "resize", // resize to target dimensions
        ],
        requires_resource: Some(ResourceKind::FileSystem),
        read_only: false,
        requires_approval: false,
        priority: 3,
    },

    ToolRegistryEntry {
        name: "pdf_read",
        version: "1.0",
        description: "Read and extract content from a PDF file.",
        family: ToolFamily::Transform,
        dsl_step_types: &[DslStepType::FetchRecords],
        operations: &[
            "read",          // extract all text
            "read_page",     // extract text from specific pages
            "extract_tables",// extract table structures from PDF
        ],
        requires_resource: Some(ResourceKind::FileSystem),
        read_only: true,
        requires_approval: false,
        priority: 2,
    },

    ToolRegistryEntry {
        name: "pdf_create",
        version: "1.0",
        description: "Create or merge PDF files.",
        family: ToolFamily::Transform,
        dsl_step_types: &[DslStepType::StoreResult],
        operations: &[
            "create", // create a new PDF from HTML or text
            "merge",  // merge multiple PDFs into one
            "split",  // split a PDF into individual pages
        ],
        requires_resource: Some(ResourceKind::FileSystem),
        read_only: false,
        requires_approval: false,
        priority: 2,
    },

    // ── NOTIFICATION ──────────────────────────────────────────────────────────

    ToolRegistryEntry {
        name: "email",
        version: "1.0",
        description: "Send email via SMTP, Mailgun, SendGrid, or Resend. Supports HTML, CC, BCC, attachments.",
        family: ToolFamily::Notification,
        dsl_step_types: &[DslStepType::Notify],
        operations: &[
            "send",      // send a new email
            "send_html", // send HTML email
            "reply",     // reply to an existing thread
            "draft",     // create draft without sending
        ],
        requires_resource: Some(ResourceKind::ApiKey),
        read_only: false,
        requires_approval: false,
        priority: 1,
    },

    ToolRegistryEntry {
        name: "notification",
        version: "1.0",
        description: "Send a notification to Slack, Discord, Telegram, or MS Teams via webhook.",
        family: ToolFamily::Notification,
        dsl_step_types: &[DslStepType::Notify],
        operations: &[
            "send",         // post a message to a channel
            "send_alert",   // post an alert with severity level
        ],
        requires_resource: Some(ResourceKind::ApiKey),
        read_only: false,
        requires_approval: false,
        priority: 1,
    },

    ToolRegistryEntry {
        name: "pushover",
        version: "1.0",
        description: "Send push notifications to mobile devices via Pushover.",
        family: ToolFamily::Notification,
        dsl_step_types: &[DslStepType::Notify],
        operations: &[
            "send", // send push notification with optional priority and sound
        ],
        requires_resource: Some(ResourceKind::ApiKey),
        read_only: false,
        requires_approval: false,
        priority: 3,
    },

    ToolRegistryEntry {
        name: "send_message",
        version: "1.0",
        description: "Send a durable structured message to a parent, child, or teammate agent.",
        family: ToolFamily::Notification,
        dsl_step_types: &[DslStepType::Notify, DslStepType::StoreResult],
        operations: &[
            "send",          // send message to an agent
            "send_result",   // send typed result contract to parent
            "send_artifact", // send a file or data artifact to parent
        ],
        requires_resource: None,
        read_only: false,
        requires_approval: false,
        priority: 2,
    },

    // ── STORAGE / FILESYSTEM ──────────────────────────────────────────────────

    ToolRegistryEntry {
        name: "file_read",
        version: "1.0",
        description: "Read files from the local filesystem or workspace.",
        family: ToolFamily::Storage,
        dsl_step_types: &[DslStepType::FetchRecords],
        operations: &[
            "read",      // read file contents as text
            "read_bytes",// read file as raw bytes
            "list",      // list files in a directory
            "stat",      // get file metadata (size, modified time)
        ],
        requires_resource: Some(ResourceKind::FileSystem),
        read_only: true,
        requires_approval: false,
        priority: 1,
    },

    ToolRegistryEntry {
        name: "file_write",
        version: "1.0",
        description: "Write files to the local filesystem or workspace.",
        family: ToolFamily::Storage,
        dsl_step_types: &[DslStepType::StoreResult],
        operations: &[
            "write",   // write or overwrite a file
            "append",  // append content to a file
            "mkdir",   // create a directory
            "delete",  // delete a file or directory
        ],
        requires_resource: Some(ResourceKind::FileSystem),
        read_only: false,
        requires_approval: false,
        priority: 1,
    },

    ToolRegistryEntry {
        name: "file_edit",
        version: "1.0",
        description: "Patch or replace content inside an existing file without overwriting the whole file.",
        family: ToolFamily::Storage,
        dsl_step_types: &[DslStepType::Compute, DslStepType::StoreResult],
        operations: &[
            "replace",     // find-and-replace a specific string in a file
            "insert_line", // insert a line at a specific line number
            "delete_lines",// delete a range of lines
        ],
        requires_resource: Some(ResourceKind::FileSystem),
        read_only: false,
        requires_approval: false,
        priority: 2,
    },

    ToolRegistryEntry {
        name: "glob_search",
        version: "1.0",
        description: "Search for files matching a glob pattern in the workspace.",
        family: ToolFamily::Storage,
        dsl_step_types: &[DslStepType::FetchRecords],
        operations: &[
            "search", // return matching file paths for a glob pattern
        ],
        requires_resource: Some(ResourceKind::FileSystem),
        read_only: true,
        requires_approval: false,
        priority: 2,
    },

    ToolRegistryEntry {
        name: "content_search",
        version: "1.0",
        description: "Full-text search inside files in the workspace.",
        family: ToolFamily::Storage,
        dsl_step_types: &[DslStepType::FetchRecords, DslStepType::DetectAnomaly],
        operations: &[
            "search",  // search for a string or regex across files
            "grep",    // grep-style search with line numbers
        ],
        requires_resource: Some(ResourceKind::FileSystem),
        read_only: true,
        requires_approval: false,
        priority: 2,
    },

    ToolRegistryEntry {
        name: "compress",
        version: "1.0",
        description: "Compress files or directories into a zip or tar archive.",
        family: ToolFamily::Storage,
        dsl_step_types: &[DslStepType::StoreResult],
        operations: &[
            "compress",   // create a zip or tar.gz archive
            "decompress", // extract an archive
            "list",       // list contents of an archive without extracting
        ],
        requires_resource: Some(ResourceKind::FileSystem),
        read_only: false,
        requires_approval: false,
        priority: 3,
    },

    // ── MEMORY ────────────────────────────────────────────────────────────────

    ToolRegistryEntry {
        name: "memory_store",
        version: "1.0",
        description: "Store a key-value pair in the agent's memory for later recall across steps.",
        family: ToolFamily::Memory,
        dsl_step_types: &[DslStepType::StoreResult],
        operations: &[
            "store",  // store a key-value pair
            "upsert", // overwrite if key exists
        ],
        requires_resource: None,
        read_only: false,
        requires_approval: false,
        priority: 1,
    },

    ToolRegistryEntry {
        name: "memory_recall",
        version: "1.0",
        description: "Retrieve a stored value from agent memory by key.",
        family: ToolFamily::Memory,
        dsl_step_types: &[DslStepType::FetchRecords],
        operations: &[
            "recall", // retrieve value by key
            "list",   // list all stored keys
        ],
        requires_resource: None,
        read_only: true,
        requires_approval: false,
        priority: 1,
    },

    ToolRegistryEntry {
        name: "memory_forget",
        version: "1.0",
        description: "Delete a key from agent memory.",
        family: ToolFamily::Memory,
        dsl_step_types: &[DslStepType::StoreResult],
        operations: &[
            "forget", // delete a single key
        ],
        requires_resource: None,
        read_only: false,
        requires_approval: false,
        priority: 2,
    },

    ToolRegistryEntry {
        name: "memory_consolidate",
        version: "1.0",
        description: "Run a durable memory consolidation pass. Merges recent signals into topic memories and prunes stale topics.",
        family: ToolFamily::Memory,
        dsl_step_types: &[DslStepType::Compute],
        operations: &[
            "consolidate", // merge and prune memory for this agent
        ],
        requires_resource: None,
        read_only: false,
        requires_approval: false,
        priority: 3,
    },

    // ── CODE / EXECUTION ──────────────────────────────────────────────────────

    ToolRegistryEntry {
        name: "code_run",
        version: "1.0",
        description: "Run a short code snippet (Python, JS, Bash). Not for multi-file apps or long-running services.",
        family: ToolFamily::Code,
        dsl_step_types: &[DslStepType::Compute],
        operations: &[
            "run", // execute a code snippet and return stdout/stderr
        ],
        requires_resource: None,
        read_only: false,
        requires_approval: false,
        priority: 3,
    },

    ToolRegistryEntry {
        name: "shell",
        version: "1.0",
        description: "Run a shell command in the local workspace.",
        family: ToolFamily::Code,
        dsl_step_types: &[DslStepType::Compute, DslStepType::FetchRecords],
        operations: &[
            "exec", // execute a shell command and return output
        ],
        requires_resource: Some(ResourceKind::FileSystem),
        read_only: false,
        requires_approval: false,
        priority: 2,
    },

    ToolRegistryEntry {
        name: "git_operations",
        version: "1.0",
        description: "Perform Git operations: clone, status, add, commit, push, pull, branch, diff, log.",
        family: ToolFamily::Code,
        dsl_step_types: &[DslStepType::FetchRecords, DslStepType::StoreResult],
        operations: &[
            "clone",    // clone a remote repository
            "status",   // show working tree status
            "add",      // stage files
            "commit",   // create a commit
            "push",     // push to remote
            "pull",     // pull from remote
            "branch",   // list or create branches
            "checkout", // switch branches or restore files
            "diff",     // show diff between refs
            "log",      // show commit log
            "init",     // initialize a new repo
        ],
        requires_resource: Some(ResourceKind::FileSystem),
        read_only: false,
        requires_approval: false,
        priority: 1,
    },

    ToolRegistryEntry {
        name: "diff",
        version: "1.0",
        description: "Compute a unified diff between two text bodies or files.",
        family: ToolFamily::Code,
        dsl_step_types: &[DslStepType::Compute, DslStepType::DetectAnomaly],
        operations: &[
            "diff",  // produce unified diff
            "patch", // apply a patch to a file
        ],
        requires_resource: None,
        read_only: true,
        requires_approval: false,
        priority: 2,
    },

    ToolRegistryEntry {
        name: "ssh_exec",
        version: "1.0",
        description: "Execute a command on a remote host over SSH.",
        family: ToolFamily::Code,
        dsl_step_types: &[DslStepType::Compute, DslStepType::FetchRecords],
        operations: &[
            "exec",         // run a command on the remote host
            "upload_file",  // scp a file to the remote host
            "download_file",// scp a file from the remote host
        ],
        requires_resource: Some(ResourceKind::SshHost),
        read_only: false,
        requires_approval: true,
        priority: 3,
    },

    ToolRegistryEntry {
        name: "docker",
        version: "1.0",
        description: "Run and manage Docker containers.",
        family: ToolFamily::Code,
        dsl_step_types: &[DslStepType::Compute, DslStepType::FetchRecords],
        operations: &[
            "run",     // run a container
            "pull",    // pull an image
            "status",  // list running containers
            "exec",    // exec into a running container
            "build",   // build an image from a Dockerfile
            "logs",    // fetch container logs
            "stop",    // stop a running container
        ],
        requires_resource: Some(ResourceKind::DockerDaemon),
        read_only: false,
        requires_approval: false,
        priority: 3,
    },

    ToolRegistryEntry {
        name: "kubernetes",
        version: "1.0",
        description: "Manage Kubernetes resources: pods, deployments, services, jobs.",
        family: ToolFamily::Code,
        dsl_step_types: &[DslStepType::FetchRecords, DslStepType::Compute, DslStepType::StoreResult],
        operations: &[
            "get",     // get a specific resource by name
            "list",    // list resources of a kind
            "delete",  // delete a resource
            "scale",   // scale a deployment
            "rollout", // trigger or check a rollout status
            "logs",    // fetch pod logs
            "apply",   // apply a manifest
        ],
        requires_resource: Some(ResourceKind::KubeCluster),
        read_only: false,
        requires_approval: true,
        priority: 3,
    },

    ToolRegistryEntry {
        name: "wasm",
        version: "1.0",
        description: "Run a WebAssembly module in the sandbox.",
        family: ToolFamily::Code,
        dsl_step_types: &[DslStepType::Compute],
        operations: &[
            "run", // execute a WASM module with given inputs
            "add", // load a WASM module into the runtime
        ],
        requires_resource: None,
        read_only: false,
        requires_approval: false,
        priority: 4,
    },

    ToolRegistryEntry {
        name: "process_monitor",
        version: "1.0",
        description: "List, find, and manage local OS processes.",
        family: ToolFamily::Code,
        dsl_step_types: &[DslStepType::FetchRecords, DslStepType::DetectAnomaly],
        operations: &[
            "list",    // list all running processes
            "find",    // find processes by name filter
            "kill",    // kill a process by PID
            "status",  // get status of a specific PID
        ],
        requires_resource: None,
        read_only: false,
        requires_approval: true,
        priority: 4,
    },

    // ── CONNECTORS (28 SaaS connectors via connector_tool) ───────────────────
    //
    // Each connector is addressed as: connector_tool { connector: "<name>", operation: "..." }
    // Operations below are the exact strings each connector accepts.

    ToolRegistryEntry {
        name: "connector:salesforce",
        version: "1.0",
        description: "Salesforce CRM: query, create, update records; log notes; create tasks.",
        family: ToolFamily::Connector,
        dsl_step_types: &[DslStepType::FetchRecords, DslStepType::StoreResult, DslStepType::Notify],
        operations: &[
            "query_records",  // SOQL query
            "get_record",     // fetch a single record by Id
            "create_record",  // create Lead, Contact, Opportunity, Task
            "update_record",  // update fields on an existing record
            "log_note",       // create a Chatter note or activity
        ],
        requires_resource: Some(ResourceKind::Connector),
        read_only: false,
        requires_approval: false,
        priority: 1,
    },

    ToolRegistryEntry {
        name: "connector:hubspot",
        version: "1.0",
        description: "HubSpot CRM: search contacts, create/update deals, add notes.",
        family: ToolFamily::Connector,
        dsl_step_types: &[DslStepType::FetchRecords, DslStepType::StoreResult],
        operations: &[
            "search_contacts", // find contacts by email, name, or property
            "create_contact",  // create a new contact
            "update_deal",     // update deal stage or properties
            "add_note",        // add a note to a contact or deal
        ],
        requires_resource: Some(ResourceKind::Connector),
        read_only: false,
        requires_approval: false,
        priority: 1,
    },

    ToolRegistryEntry {
        name: "connector:zendesk",
        version: "1.0",
        description: "Zendesk Support: list/create/update tickets, add comments, assign agents.",
        family: ToolFamily::Connector,
        dsl_step_types: &[DslStepType::FetchRecords, DslStepType::StoreResult, DslStepType::Notify],
        operations: &[
            "list_tickets",   // list tickets with optional status/priority filter
            "get_ticket",     // fetch a ticket by ID
            "create_ticket",  // open a new support ticket
            "update_ticket",  // update status, assignee, priority
            "add_comment",    // add a public or internal comment
        ],
        requires_resource: Some(ResourceKind::Connector),
        read_only: false,
        requires_approval: false,
        priority: 1,
    },

    ToolRegistryEntry {
        name: "connector:intercom",
        version: "1.0",
        description: "Intercom: manage conversations, reply to users, search contacts.",
        family: ToolFamily::Connector,
        dsl_step_types: &[DslStepType::FetchRecords, DslStepType::StoreResult, DslStepType::Notify],
        operations: &[
            "list_conversations", // list open or unassigned conversations
            "get_conversation",   // fetch a conversation by ID
            "reply",              // send a reply in a conversation
            "create_note",        // add an internal note
            "search_contacts",    // find contacts by email or name
        ],
        requires_resource: Some(ResourceKind::Connector),
        read_only: false,
        requires_approval: false,
        priority: 2,
    },

    ToolRegistryEntry {
        name: "connector:freshdesk",
        version: "1.0",
        description: "Freshdesk: create/update tickets, add notes, look up contacts.",
        family: ToolFamily::Connector,
        dsl_step_types: &[DslStepType::FetchRecords, DslStepType::StoreResult],
        operations: &[
            "list_tickets",  // list tickets with filters
            "create_ticket", // open a new ticket
            "update_ticket", // update status, priority, or assignee
            "add_note",      // add a private or public note
            "get_contact",   // look up a contact by email
        ],
        requires_resource: Some(ResourceKind::Connector),
        read_only: false,
        requires_approval: false,
        priority: 2,
    },

    ToolRegistryEntry {
        name: "connector:github",
        version: "1.0",
        description: "GitHub: read files, manage issues/PRs, push commits, trigger workflows.",
        family: ToolFamily::Connector,
        dsl_step_types: &[DslStepType::FetchRecords, DslStepType::StoreResult],
        operations: &[
            "get_file",     // read a file from a repo
            "list_issues",  // list open issues
            "create_issue", // open a new issue
            "create_pr",    // open a pull request
            "merge_pr",     // merge a pull request
            "push_commit",  // push file changes as a commit
            "run_workflow", // trigger a GitHub Actions workflow
        ],
        requires_resource: Some(ResourceKind::Connector),
        read_only: false,
        requires_approval: false,
        priority: 1,
    },

    ToolRegistryEntry {
        name: "connector:jira",
        version: "1.0",
        description: "Jira: search issues (JQL), create/update bugs/stories/tasks, add comments.",
        family: ToolFamily::Connector,
        dsl_step_types: &[DslStepType::FetchRecords, DslStepType::StoreResult],
        operations: &[
            "search_issues", // JQL search
            "get_issue",     // fetch issue details by key
            "create_issue",  // create a new bug, story, or task
            "update_issue",  // change status, assignee, priority
            "add_comment",   // add a comment to an issue
        ],
        requires_resource: Some(ResourceKind::Connector),
        read_only: false,
        requires_approval: false,
        priority: 1,
    },

    ToolRegistryEntry {
        name: "connector:notion",
        version: "1.0",
        description: "Notion: search pages, read/append content, create database entries.",
        family: ToolFamily::Connector,
        dsl_step_types: &[DslStepType::FetchRecords, DslStepType::StoreResult],
        operations: &[
            "search_pages",  // search by title or content keyword
            "get_page",      // read a page's full content blocks
            "create_page",   // create a new page or database entry
            "append_block",  // append content blocks to a page
            "update_props",  // update database entry properties
        ],
        requires_resource: Some(ResourceKind::Connector),
        read_only: false,
        requires_approval: false,
        priority: 1,
    },

    ToolRegistryEntry {
        name: "connector:asana",
        version: "1.0",
        description: "Asana: list/create/update tasks, add comments, manage project sections.",
        family: ToolFamily::Connector,
        dsl_step_types: &[DslStepType::FetchRecords, DslStepType::StoreResult],
        operations: &[
            "list_tasks",   // list tasks in a project or assigned to a user
            "create_task",  // create a new task
            "update_task",  // update status, assignee, or due date
            "add_comment",  // add a comment to a task
        ],
        requires_resource: Some(ResourceKind::Connector),
        read_only: false,
        requires_approval: false,
        priority: 2,
    },

    ToolRegistryEntry {
        name: "connector:linear",
        version: "1.0",
        description: "Linear: search/create/update issues and projects.",
        family: ToolFamily::Connector,
        dsl_step_types: &[DslStepType::FetchRecords, DslStepType::StoreResult],
        operations: &[
            "search_issues", // search issues by keyword or filter
            "get_issue",     // fetch issue by ID
            "create_issue",  // create a new issue
            "update_issue",  // update status, assignee, or priority
        ],
        requires_resource: Some(ResourceKind::Connector),
        read_only: false,
        requires_approval: false,
        priority: 2,
    },

    ToolRegistryEntry {
        name: "connector:monday",
        version: "1.0",
        description: "Monday.com: read/create/update board items and columns.",
        family: ToolFamily::Connector,
        dsl_step_types: &[DslStepType::FetchRecords, DslStepType::StoreResult],
        operations: &[
            "list_items",    // list items in a board
            "get_item",      // fetch a specific item
            "create_item",   // create a new board item
            "update_item",   // update column values on an item
        ],
        requires_resource: Some(ResourceKind::Connector),
        read_only: false,
        requires_approval: false,
        priority: 3,
    },

    ToolRegistryEntry {
        name: "connector:slack",
        version: "1.0",
        description: "Slack: send messages, read channels, search messages, manage threads.",
        family: ToolFamily::Connector,
        dsl_step_types: &[DslStepType::FetchRecords, DslStepType::Notify],
        operations: &[
            "send_message",   // post a message to a channel or DM
            "list_channels",  // list available channels
            "search_messages",// search messages across workspace
            "add_reaction",   // add an emoji reaction to a message
            "get_thread",     // fetch a message thread
        ],
        requires_resource: Some(ResourceKind::Connector),
        read_only: false,
        requires_approval: false,
        priority: 1,
    },

    ToolRegistryEntry {
        name: "connector:gmail",
        version: "1.0",
        description: "Gmail: search, read, send, and reply to email.",
        family: ToolFamily::Connector,
        dsl_step_types: &[DslStepType::FetchRecords, DslStepType::Notify],
        operations: &[
            "search_emails", // search emails by query string
            "get_email",     // fetch a specific email by ID
            "send_email",    // send a new email
            "reply",         // reply to an existing thread
            "label",         // apply or remove a label
        ],
        requires_resource: Some(ResourceKind::Connector),
        read_only: false,
        requires_approval: false,
        priority: 1,
    },

    ToolRegistryEntry {
        name: "connector:google_calendar",
        version: "1.0",
        description: "Google Calendar: list/create/update events, check availability.",
        family: ToolFamily::Connector,
        dsl_step_types: &[DslStepType::FetchRecords, DslStepType::StoreResult],
        operations: &[
            "list_events",   // list upcoming events
            "get_event",     // fetch a specific event
            "create_event",  // create a new calendar event
            "update_event",  // update event time, location, or attendees
            "delete_event",  // delete an event
            "check_free_busy",// check availability for a time range
        ],
        requires_resource: Some(ResourceKind::Connector),
        read_only: false,
        requires_approval: false,
        priority: 1,
    },

    ToolRegistryEntry {
        name: "connector:outlook",
        version: "1.0",
        description: "Microsoft Outlook: read/send email, manage calendar events.",
        family: ToolFamily::Connector,
        dsl_step_types: &[DslStepType::FetchRecords, DslStepType::Notify],
        operations: &[
            "search_emails", // search emails
            "send_email",    // send a new email
            "reply",         // reply to an existing thread
            "list_events",   // list calendar events
            "create_event",  // create a calendar event
        ],
        requires_resource: Some(ResourceKind::Connector),
        read_only: false,
        requires_approval: false,
        priority: 2,
    },

    ToolRegistryEntry {
        name: "connector:quickbooks",
        version: "1.0",
        description: "QuickBooks: query invoices, customers, expenses, and accounts.",
        family: ToolFamily::Connector,
        dsl_step_types: &[DslStepType::FetchRecords, DslStepType::Aggregate],
        operations: &[
            "query",          // run a QuickBooks query (SQL-like)
            "get_invoice",    // fetch a specific invoice
            "create_invoice", // create a new invoice
            "list_customers", // list customers
            "list_expenses",  // list expenses
        ],
        requires_resource: Some(ResourceKind::Connector),
        read_only: false,
        requires_approval: false,
        priority: 2,
    },

    ToolRegistryEntry {
        name: "connector:stripe",
        version: "1.0",
        description: "Stripe: query charges, customers, subscriptions, invoices, and refunds.",
        family: ToolFamily::Connector,
        dsl_step_types: &[DslStepType::FetchRecords, DslStepType::Aggregate],
        operations: &[
            "list_charges",      // list charges with optional filters
            "get_customer",      // fetch a customer by ID or email
            "list_subscriptions",// list active subscriptions
            "create_refund",     // issue a refund on a charge
            "list_invoices",     // list invoices
        ],
        requires_resource: Some(ResourceKind::Connector),
        read_only: false,
        requires_approval: true,
        priority: 1,
    },

    ToolRegistryEntry {
        name: "connector:servicenow",
        version: "1.0",
        description: "ServiceNow: manage incidents, change requests, and CMDB records.",
        family: ToolFamily::Connector,
        dsl_step_types: &[DslStepType::FetchRecords, DslStepType::StoreResult],
        operations: &[
            "list_incidents",  // list incidents with optional filter
            "get_incident",    // fetch incident by sys_id
            "create_incident", // create a new incident
            "update_incident", // update incident fields
            "query_cmdb",      // query CMDB for a CI
        ],
        requires_resource: Some(ResourceKind::Connector),
        read_only: false,
        requires_approval: false,
        priority: 2,
    },

    ToolRegistryEntry {
        name: "connector:pagerduty",
        version: "1.0",
        description: "PagerDuty: list/acknowledge/resolve incidents, trigger alerts.",
        family: ToolFamily::Connector,
        dsl_step_types: &[DslStepType::FetchRecords, DslStepType::Notify],
        operations: &[
            "list_incidents",  // list active incidents
            "get_incident",    // fetch incident details
            "acknowledge",     // acknowledge an incident
            "resolve",         // resolve an incident
            "trigger_alert",   // trigger a new PagerDuty alert
        ],
        requires_resource: Some(ResourceKind::Connector),
        read_only: false,
        requires_approval: false,
        priority: 1,
    },

    ToolRegistryEntry {
        name: "connector:greenhouse",
        version: "1.0",
        description: "Greenhouse ATS: list jobs, candidates, applications; add notes.",
        family: ToolFamily::Connector,
        dsl_step_types: &[DslStepType::FetchRecords, DslStepType::StoreResult],
        operations: &[
            "list_jobs",         // list open job postings
            "list_candidates",   // list candidates with optional filter
            "get_application",   // fetch application details
            "add_note",          // add a note to a candidate
            "update_stage",      // move candidate to a different stage
        ],
        requires_resource: Some(ResourceKind::Connector),
        read_only: false,
        requires_approval: false,
        priority: 2,
    },

    ToolRegistryEntry {
        name: "connector:docusign",
        version: "1.0",
        description: "DocuSign: send envelopes for signature, check status, download signed docs.",
        family: ToolFamily::Connector,
        dsl_step_types: &[DslStepType::StoreResult, DslStepType::FetchRecords],
        operations: &[
            "send_envelope",     // send a document for signature
            "get_envelope",      // check envelope/signature status
            "list_envelopes",    // list envelopes with optional filter
            "download_document", // download the signed document
        ],
        requires_resource: Some(ResourceKind::Connector),
        read_only: false,
        requires_approval: true,
        priority: 2,
    },

    ToolRegistryEntry {
        name: "connector:dbt_cloud",
        version: "1.0",
        description: "dbt Cloud: trigger jobs, check run status, fetch logs.",
        family: ToolFamily::Connector,
        dsl_step_types: &[DslStepType::Compute, DslStepType::FetchRecords],
        operations: &[
            "trigger_job",  // trigger a dbt Cloud job
            "get_run",      // get the status of a run
            "list_jobs",    // list all jobs in a project
            "get_logs",     // fetch logs from a completed run
        ],
        requires_resource: Some(ResourceKind::Connector),
        read_only: false,
        requires_approval: false,
        priority: 2,
    },

    ToolRegistryEntry {
        name: "connector:twilio",
        version: "1.0",
        description: "Twilio: send SMS and voice calls.",
        family: ToolFamily::Connector,
        dsl_step_types: &[DslStepType::Notify],
        operations: &[
            "send_sms",   // send an SMS message
            "make_call",  // initiate a voice call
        ],
        requires_resource: Some(ResourceKind::Connector),
        read_only: false,
        requires_approval: true,
        priority: 2,
    },

    ToolRegistryEntry {
        name: "connector:teams",
        version: "1.0",
        description: "Microsoft Teams: send messages to channels, read messages, manage meetings.",
        family: ToolFamily::Connector,
        dsl_step_types: &[DslStepType::FetchRecords, DslStepType::Notify],
        operations: &[
            "send_message",   // post a message to a channel
            "list_channels",  // list channels in a team
            "get_messages",   // read recent messages
            "create_meeting", // schedule a Teams meeting
        ],
        requires_resource: Some(ResourceKind::Connector),
        read_only: false,
        requires_approval: false,
        priority: 2,
    },

    ToolRegistryEntry {
        name: "connector:shopify",
        version: "1.0",
        description: "Shopify: manage products, orders, customers, and inventory.",
        family: ToolFamily::Connector,
        dsl_step_types: &[DslStepType::FetchRecords, DslStepType::StoreResult, DslStepType::Aggregate],
        operations: &[
            "list_orders",    // list orders with optional filter
            "get_order",      // fetch a specific order
            "list_products",  // list products
            "update_product", // update product details or inventory
            "list_customers", // list customers
        ],
        requires_resource: Some(ResourceKind::Connector),
        read_only: false,
        requires_approval: false,
        priority: 1,
    },

    ToolRegistryEntry {
        name: "connector:airtable",
        version: "1.0",
        description: "Airtable: list/create/update/delete records in a base.",
        family: ToolFamily::Connector,
        dsl_step_types: &[DslStepType::FetchRecords, DslStepType::StoreResult, DslStepType::Filter],
        operations: &[
            "list_records",   // list records in a table with optional filter
            "get_record",     // fetch a single record by ID
            "create_record",  // create a new record
            "update_record",  // update fields on an existing record
            "delete_record",  // delete a record
        ],
        requires_resource: Some(ResourceKind::Connector),
        read_only: false,
        requires_approval: false,
        priority: 1,
    },

    ToolRegistryEntry {
        name: "connector:mailchimp",
        version: "1.0",
        description: "Mailchimp: manage lists, subscribers, campaigns, and tags.",
        family: ToolFamily::Connector,
        dsl_step_types: &[DslStepType::FetchRecords, DslStepType::StoreResult, DslStepType::Notify],
        operations: &[
            "list_members",     // list members in an audience
            "add_member",       // add or update a subscriber
            "remove_member",    // unsubscribe a member
            "send_campaign",    // trigger a campaign send
            "get_campaign_stats",// fetch open/click stats for a campaign
        ],
        requires_resource: Some(ResourceKind::Connector),
        read_only: false,
        requires_approval: false,
        priority: 2,
    },

    // ── API / INTEGRATION ─────────────────────────────────────────────────────

    ToolRegistryEntry {
        name: "external_api",
        version: "1.0",
        description: "Call a tenant-registered external REST API. Use register_api_tool to set it up first.",
        family: ToolFamily::Connector,
        dsl_step_types: &[DslStepType::FetchRecords, DslStepType::StoreResult],
        operations: &[
            "call", // invoke a registered API endpoint
        ],
        requires_resource: Some(ResourceKind::ApiKey),
        read_only: false,
        requires_approval: false,
        priority: 3,
    },

    ToolRegistryEntry {
        name: "api_call",
        version: "1.0",
        description: "Execute a registered or dynamically configured API call with full control over method, headers, and body.",
        family: ToolFamily::Connector,
        dsl_step_types: &[DslStepType::FetchRecords, DslStepType::StoreResult],
        operations: &[
            "get",    // HTTP GET against the registered API
            "post",   // HTTP POST
            "put",    // HTTP PUT
            "patch",  // HTTP PATCH
            "delete", // HTTP DELETE
        ],
        requires_resource: Some(ResourceKind::ApiKey),
        read_only: false,
        requires_approval: false,
        priority: 2,
    },

    ToolRegistryEntry {
        name: "mcp_session",
        version: "1.0",
        description: "Connect to an MCP server and invoke its tools.",
        family: ToolFamily::Connector,
        dsl_step_types: &[DslStepType::FetchRecords, DslStepType::Compute, DslStepType::StoreResult],
        operations: &[
            "connect",        // initialize the MCP session and negotiate capabilities
            "list_tools",     // list available tools on the MCP server
            "call_tool",      // call a named MCP tool
            "list_resources", // list exposed MCP resources
            "read_resource",  // read a specific MCP resource
        ],
        requires_resource: Some(ResourceKind::McpServer),
        read_only: false,
        requires_approval: false,
        priority: 2,
    },

    ToolRegistryEntry {
        name: "acp_session",
        version: "1.0",
        description: "Connect to an ACP (Agent Communication Protocol) peer and exchange messages with remote agents.",
        family: ToolFamily::Connector,
        dsl_step_types: &[DslStepType::Compute, DslStepType::FetchRecords, DslStepType::Notify, DslStepType::StoreResult],
        operations: &[
            "list_agents",  // list available agents on the ACP peer
            "receive_messages", // poll the ACP peer for inbound messages
            "send_message", // send a message to a remote ACP agent
        ],
        requires_resource: Some(ResourceKind::AcpPeer),
        read_only: false,
        requires_approval: false,
        priority: 3,
    },

    // ── SCHEDULING ────────────────────────────────────────────────────────────

    ToolRegistryEntry {
        name: "cron_add",
        version: "1.0",
        description: "Add a cron schedule to trigger this workflow on a recurring basis.",
        family: ToolFamily::Scheduling,
        dsl_step_types: &[DslStepType::StoreResult],
        operations: &[
            "add", // register a new cron expression for this workflow
        ],
        requires_resource: None,
        read_only: false,
        requires_approval: false,
        priority: 1,
    },

    ToolRegistryEntry {
        name: "cron_list",
        version: "1.0",
        description: "List all active cron schedules for this agent.",
        family: ToolFamily::Scheduling,
        dsl_step_types: &[DslStepType::FetchRecords],
        operations: &[
            "list", // return all active cron schedules
        ],
        requires_resource: None,
        read_only: true,
        requires_approval: false,
        priority: 1,
    },

    ToolRegistryEntry {
        name: "cron_remove",
        version: "1.0",
        description: "Remove a cron schedule by ID.",
        family: ToolFamily::Scheduling,
        dsl_step_types: &[DslStepType::StoreResult],
        operations: &[
            "remove", // delete a cron schedule by ID
        ],
        requires_resource: None,
        read_only: false,
        requires_approval: false,
        priority: 2,
    },

    ToolRegistryEntry {
        name: "cron_update",
        version: "1.0",
        description: "Update an existing cron schedule.",
        family: ToolFamily::Scheduling,
        dsl_step_types: &[DslStepType::StoreResult],
        operations: &[
            "update", // change the cron expression or enabled state
        ],
        requires_resource: None,
        read_only: false,
        requires_approval: false,
        priority: 2,
    },

    ToolRegistryEntry {
        name: "cron_run",
        version: "1.0",
        description: "Manually trigger a cron-scheduled job immediately.",
        family: ToolFamily::Scheduling,
        dsl_step_types: &[DslStepType::Compute],
        operations: &[
            "run", // trigger the job immediately without waiting for its schedule
        ],
        requires_resource: None,
        read_only: false,
        requires_approval: false,
        priority: 2,
    },

    ToolRegistryEntry {
        name: "cron_runs",
        version: "1.0",
        description: "List recent execution history for a cron job.",
        family: ToolFamily::Scheduling,
        dsl_step_types: &[DslStepType::FetchRecords],
        operations: &[
            "list_runs", // return recent run records for a cron job
        ],
        requires_resource: None,
        read_only: true,
        requires_approval: false,
        priority: 3,
    },

    ToolRegistryEntry {
        name: "delegate",
        version: "1.0",
        description: "Delegate a subtask to a child agent and wait for its result.",
        family: ToolFamily::Scheduling,
        dsl_step_types: &[DslStepType::Compute, DslStepType::FetchRecords],
        operations: &[
            "delegate", // spawn a child agent and return its result
        ],
        requires_resource: None,
        read_only: false,
        requires_approval: false,
        priority: 1,
    },

    // ── SECURITY ──────────────────────────────────────────────────────────────

    ToolRegistryEntry {
        name: "crypto_tool",
        version: "1.0",
        description: "Cryptographic operations: hash, encrypt, decrypt, sign, verify.",
        family: ToolFamily::Security,
        dsl_step_types: &[DslStepType::Compute],
        operations: &[
            "hash",    // hash text or a file (SHA-256, SHA-512, MD5)
            "encrypt", // encrypt text or a file
            "decrypt", // decrypt ciphertext
            "sign",    // sign a payload with a private key
            "verify",  // verify a signature
        ],
        requires_resource: None,
        read_only: false,
        requires_approval: false,
        priority: 1,
    },

    ToolRegistryEntry {
        name: "request_credential",
        version: "1.0",
        description: "Prompt the user to store a secret (API key, token, password) securely before use.",
        family: ToolFamily::Security,
        dsl_step_types: &[DslStepType::FetchRecords],
        operations: &[
            "request", // prompt for a credential and store it securely
        ],
        requires_resource: None,
        read_only: false,
        requires_approval: false,
        priority: 1,
    },

    // ── META / CONTROL ────────────────────────────────────────────────────────

    ToolRegistryEntry {
        name: "ask_user",
        version: "1.0",
        description: "Pause compilation and surface a structured question or setup card to the user.",
        family: ToolFamily::Meta,
        // ask_user is compiler-only. It never appears as a compiled DAG step.
        // It is emitted by the compiler when a required input is missing.
        dsl_step_types: &[],
        operations: &[
            "mcq",          // multiple choice question
            "multi_select",  // choose one or more options
            "text",          // free-form text answer
            "card_open",     // open a setup card (database, connector, API auth)
            "hybrid",        // choices plus a text fallback
        ],
        requires_resource: None,
        read_only: true,
        requires_approval: false,
        priority: 1,
    },

    ToolRegistryEntry {
        name: "tool_search",
        version: "1.0",
        description: "Search the tool registry for tools matching a query.",
        family: ToolFamily::Meta,
        dsl_step_types: &[DslStepType::FetchRecords],
        operations: &[
            "search", // return tools matching a query string
        ],
        requires_resource: None,
        read_only: true,
        requires_approval: false,
        priority: 1,
    },

    ToolRegistryEntry {
        name: "request_more_tools",
        version: "1.0",
        description: "Request additional tool capabilities for the current step when the default pool is insufficient.",
        family: ToolFamily::Meta,
        dsl_step_types: &[],
        operations: &[
            "request", // request specific tool names for the current step
        ],
        requires_resource: None,
        read_only: true,
        requires_approval: false,
        priority: 2,
    },

    ToolRegistryEntry {
        name: "list_connectors_in_category",
        version: "1.0",
        description: "List available connectors in a given category (crm, support, devtools, etc.).",
        family: ToolFamily::Meta,
        dsl_step_types: &[DslStepType::FetchRecords],
        operations: &[
            "list", // list connectors in a category
        ],
        requires_resource: None,
        read_only: true,
        requires_approval: false,
        priority: 1,
    },

    ToolRegistryEntry {
        name: "suggest_connectors",
        version: "1.0",
        description: "Suggest connectors that might satisfy a described capability need.",
        family: ToolFamily::Meta,
        dsl_step_types: &[],
        operations: &[
            "suggest", // suggest connectors for a described need
        ],
        requires_resource: None,
        read_only: true,
        requires_approval: false,
        priority: 2,
    },

    ToolRegistryEntry {
        name: "model_routing",
        version: "1.0",
        description: "Route a sub-task to a specific LLM model by capability or cost profile.",
        family: ToolFamily::Meta,
        dsl_step_types: &[DslStepType::Compute],
        operations: &[
            "route", // select and invoke a model for a specific task
        ],
        requires_resource: None,
        read_only: true,
        requires_approval: false,
        priority: 3,
    },
];

// ─────────────────────────────────────────────────────────────────────────────
// Binding algorithm
// ─────────────────────────────────────────────────────────────────────────────

#[derive(Debug)]
pub struct DslStep {
    pub id: String,
    pub step_type: DslStepType,
    pub tool: Option<String>,
    pub operation: Option<String>,       // must be present for binding to succeed
    pub resource_id: Option<String>,     // named resource in the workflow's resources block
    pub resource_type: Option<ResourceKind>,
    pub constraints: StepConstraints,
}

#[derive(Debug, Default)]
pub struct StepConstraints {
    pub read_only: bool,
    pub requires_approval: bool,
}

#[derive(Debug)]
pub struct BoundTool {
    pub tool_name: &'static str,
    pub operation: String,
}

#[derive(Debug)]
pub enum BindingError {
    /// The plan named a tool that is not in the registry.
    UnknownTool { step_id: String, tool: String },

    /// No tool in registry serves this DSL step type at all.
    NoToolForStepType(DslStepType),

    /// The explicit tool does not support the declared DSL step type.
    ToolDoesNotSupportStepType { step_id: String, tool: String, step_type: DslStepType },

    /// Tools exist for this step type but all require a resource that is not bound.
    MissingResource {
        step_id: String,
        required_kind: ResourceKind,
        candidate_tools: Vec<&'static str>,
    },

    /// Tools match step type and resource but none support the declared operation.
    NoToolForOperation {
        step_id: String,
        operation: String,
        candidate_tools: Vec<&'static str>,
    },

    /// The step declared no operation at all.
    MissingOperation { step_id: String },

    /// Tools match but all require write access and the step is constrained read-only.
    PolicyViolation { step_id: String, reason: &'static str },
}

pub struct ResourceContext {
    /// Map of resource_id → ResourceKind for all resources bound in this workflow.
    pub bindings: std::collections::HashMap<String, ResourceKind>,
}

impl ResourceContext {
    pub fn has_bound(
        &self,
        kind: ResourceKind,
        resource_id: &Option<String>,
        resource_type: &Option<ResourceKind>,
    ) -> bool {
        match resource_id {
            Some(id) => self.bindings.get(id).map(|k| *k == kind).unwrap_or(false),
            None => resource_type.as_ref().map(|resource_kind| *resource_kind == kind).unwrap_or_else(|| {
                self.bindings.values().any(|resource_kind| *resource_kind == kind)
            }),
        }
    }
}

/// Bind a single DSL step to an exact tool.
/// Runs 4 deterministic filters in order.
/// Returns the winning tool or a typed error describing exactly what is missing.
pub fn bind_step(
    step: &DslStep,
    resources: &ResourceContext,
) -> Result<BoundTool, BindingError> {

    let named_tool_candidates: Vec<&ToolRegistryEntry> = if let Some(tool_name) = step.tool.as_deref() {
        let exact: Vec<&ToolRegistryEntry> = TOOL_REGISTRY.iter().filter(|t| t.name == tool_name).collect();
        if exact.is_empty() {
            return Err(BindingError::UnknownTool { step_id: step.id.clone(), tool: tool_name.to_string() });
        }
        exact
    } else {
        TOOL_REGISTRY.iter().filter(|t| t.dsl_step_types.contains(&step.step_type)).collect()
    };

    // Guard: operation must be declared by plan mode
    let operation = step.operation.as_deref().ok_or_else(|| {
        BindingError::MissingOperation { step_id: step.id.clone() }
    })?;

    // Filter 1 — DSL step type match
    // Narrows the full registry to tools that can serve this kind of step.
    let by_type: Vec<&ToolRegistryEntry> =
        named_tool_candidates.into_iter().filter(|t| t.dsl_step_types.contains(&step.step_type)).collect();

    if by_type.is_empty() {
        if let Some(tool_name) = step.tool.as_deref() {
            return Err(BindingError::ToolDoesNotSupportStepType {
                step_id: step.id.clone(),
                tool: tool_name.to_string(),
                step_type: step.step_type.clone(),
            });
        }
        return Err(BindingError::NoToolForStepType(step.step_type.clone()));
    }

    // Filter 2 — resource availability
    // Eliminates tools that need a resource the workflow hasn't bound.
    let by_resource: Vec<&ToolRegistryEntry> = by_type
        .iter()
        .filter(|t| match &t.requires_resource {
            None => true,
            Some(kind) => resources.has_bound(kind.clone(), &step.resource_id, &step.resource_type),
        })
        .copied()
        .collect();

    if by_resource.is_empty() {
        // Report what resource is needed so the compiler can emit the right ask_user card
        let required_kind = by_type[0].requires_resource.clone().unwrap();
        return Err(BindingError::MissingResource {
            step_id: step.id.clone(),
            required_kind,
            candidate_tools: by_type.iter().map(|t| t.name).collect(),
        });
    }

    // Filter 3 — operation match
    // Eliminates tools that don't support the declared operation.
    let by_operation: Vec<&ToolRegistryEntry> = by_resource
        .iter()
        .filter(|t| t.operations.contains(&operation))
        .copied()
        .collect();

    if by_operation.is_empty() {
        return Err(BindingError::NoToolForOperation {
            step_id: step.id.clone(),
            operation: operation.to_string(),
            candidate_tools: by_resource.iter().map(|t| t.name).collect(),
        });
    }

    // Filter 4 — policy enforcement
    let by_policy: Vec<&ToolRegistryEntry> = by_operation
        .iter()
        .filter(|t| !step.constraints.read_only || t.read_only || !operation_is_mutating(operation))
        .copied()
        .collect();

    if by_policy.is_empty() {
        return Err(BindingError::PolicyViolation {
            step_id: step.id.clone(),
            reason: "step is constrained read-only but all matching tools perform writes",
        });
    }

    // Pick lowest priority number (most preferred)
    let winner = by_policy
        .into_iter()
        .min_by_key(|t| t.priority)
        .unwrap();

    Ok(BoundTool {
        tool_name: winner.name,
        operation: operation.to_string(),
    })
}

fn operation_is_mutating(operation: &str) -> bool {
    matches!(
        operation.to_ascii_lowercase().as_str(),
        "execute"
            | "insert"
            | "update"
            | "delete"
            | "create"
            | "write"
            | "append"
            | "mkdir"
            | "remove"
            | "push"
            | "pull"
            | "commit"
            | "checkout"
            | "merge"
            | "split"
            | "run"
            | "post"
            | "put"
            | "patch"
            | "send"
            | "reply"
            | "draft"
            | "add"
            | "forget"
            | "upsert"
            | "clear_all"
            | "trigger"
            | "acknowledge"
            | "resolve"
            | "scale"
            | "apply"
            | "encrypt"
            | "decrypt"
            | "sign"
            | "verify"
    )
}

/// Inject this into the Phase 2 (DSL generation) LLM prompt.
/// Gives plan mode the exact operation vocabulary so it never invents operations.
pub fn dsl_generation_prompt_fragment() -> &'static str {
    r#"
## DSL Step Rules

Every step you emit MUST include an `operation` field.
The operation must be one of the exact strings listed for the tool family below.
If you cannot determine the correct operation from the user's intent, emit `ask_user` instead of guessing.

### Web family (no resource binding required)
- web_search_tool:  search | discover
- web_fetch:        fetch | fetch_raw
- browser_open:     open | check_reachable
- browser_interact: click | type | select | navigate | extract
- browser_network:  intercept | monitor
- http_request:     get | post | put | patch | delete | head
- screenshot:       screenshot

### Database family (requires Database resource binding)
- external_db:   schema | query | execute | table_preview | explain
- sql_query:     select | insert | update | delete | create
- vector_search: search | query
- vector_store:  store | upsert

### Transform family
- data_engine:    transform_records | filter | map | compute_formula | clean_data | apply_rules | rank_items | aggregate_records | extract_structured_data
- data_extractor: extract | extract_table
- spreadsheet_read:  read | read_sheet | list_sheets
- spreadsheet_write: write | append_rows
- pdf_read:       read | read_page | extract_tables
- pdf_create:     create | merge | split
- image_process:  rotate | flip | crop | resize

### Connector family (requires Connector resource binding)
- connector:salesforce:      query_records | get_record | create_record | update_record | log_note
- connector:hubspot:         search_contacts | create_contact | update_deal | add_note
- connector:zendesk:         list_tickets | get_ticket | create_ticket | update_ticket | add_comment
- connector:intercom:        list_conversations | get_conversation | reply | create_note | search_contacts
- connector:freshdesk:       list_tickets | create_ticket | update_ticket | add_note | get_contact
- connector:github:          get_file | list_issues | create_issue | create_pr | merge_pr | push_commit | run_workflow
- connector:jira:            search_issues | get_issue | create_issue | update_issue | add_comment
- connector:notion:          search_pages | get_page | create_page | append_block | update_props
- connector:asana:           list_tasks | create_task | update_task | add_comment
- connector:linear:          search_issues | get_issue | create_issue | update_issue
- connector:monday:          list_items | get_item | create_item | update_item
- connector:slack:           send_message | list_channels | search_messages | add_reaction | get_thread
- connector:gmail:           search_emails | get_email | send_email | reply | label
- connector:google_calendar: list_events | get_event | create_event | update_event | delete_event | check_free_busy
- connector:outlook:         search_emails | send_email | reply | list_events | create_event
- connector:quickbooks:      query | get_invoice | create_invoice | list_customers | list_expenses
- connector:stripe:          list_charges | get_customer | list_subscriptions | create_refund | list_invoices
- connector:servicenow:      list_incidents | get_incident | create_incident | update_incident | query_cmdb
- connector:pagerduty:       list_incidents | get_incident | acknowledge | resolve | trigger_alert
- connector:greenhouse:      list_jobs | list_candidates | get_application | add_note | update_stage
- connector:docusign:        send_envelope | get_envelope | list_envelopes | download_document
- connector:dbt_cloud:       trigger_job | get_run | list_jobs | get_logs
- connector:twilio:          send_sms | make_call
- connector:teams:           send_message | list_channels | get_messages | create_meeting
- connector:shopify:         list_orders | get_order | list_products | update_product | list_customers
- connector:airtable:        list_records | get_record | create_record | update_record | delete_record
- connector:mailchimp:       list_members | add_member | remove_member | send_campaign | get_campaign_stats

### Notification family
- email:        send | send_html | reply | draft
- notification: send | send_alert
- pushover:     send

### Storage family (requires FileSystem resource binding)
- file_read:     read | read_bytes | list | stat
- file_write:    write | append | mkdir | delete
- file_edit:     replace | insert_line | delete_lines
- glob_search:   search
- content_search: search | grep
- compress:      compress | decompress | list

### Code family
- code_run:       run
- shell:          exec
- git_operations: clone | status | add | commit | push | pull | branch | checkout | diff | log | init
- docker:         run | pull | status | exec | build | logs | stop
- kubernetes:     get | list | delete | scale | rollout | logs | apply
- ssh_exec:       exec | upload_file | download_file
- crypto_tool:    hash | encrypt | decrypt | sign | verify

### Scheduling family
- cron_add:    add
- cron_remove: remove
- cron_update: update
- cron_run:    run
- delegate:    delegate

## Step output schema

Every step must declare `output_schema` with explicit types.
Use the Narayan type system: number | string | boolean | array | object.
Nested object fields must be listed explicitly.

## Resource references

If a step needs a database, connector, or file path:
- declare the resource in the workflow `resources` block with an id
- reference it in the step as `resource_id`
- do NOT inline connection strings in step args

## When to emit ask_user

Emit ask_user (not a step) when:
- the operation cannot be determined from the user's intent
- a required resource is missing and cannot be inferred
- a policy conflict exists that requires user resolution
- after 2 failed repair passes
"#
}
