# DuckDuckGo Search Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans (recommended). Steps use checkbox (`- [x]`) syntax for tracking.

**Goal:** Add a keyless `web_search` MCP tool backed by DuckDuckGo Lite HTML results.

**Architecture:** Keep MCP policy and presentation in `mcp/tools.rs` and `mcp/catalog.rs`; put the blocking HTTP request, URL decoding, HTML parsing, validation, and serializable result types in a focused `web_search.rs` module. The handler uses `spawn_blocking` so the synchronous HTTP client never blocks the Tokio worker. The frontend catalog and standalone preview mirror the new system tool.

**Tech Stack:** Rust 2024, rmcp, reqwest with rustls, scraper, Tokio, DuckDuckGo Lite HTML endpoint, React/TypeScript catalog data.

---

### Task 1: Add the search dependencies and parser module

**Files:**
- Modify: `src-tauri/Cargo.toml`
- Modify: `src-tauri/src/lib.rs` or the platform-independent module declaration location
- Create: `src-tauri/src/web_search.rs`
- Test: `src-tauri/src/web_search.rs`

- [x] **Step 1: Add the HTTP and HTML dependencies.**

Run:

```bash
cd src-tauri
cargo add reqwest@0.12 --no-default-features --features blocking,rustls-tls,charset
cargo add scraper@0.22
```

The HTTP client must use rustls so Linux, macOS, and Windows builds do not require platform-specific OpenSSL setup.

- [x] **Step 2: Define the search data types and constants.**

Create `web_search.rs` with:

```rust
const ENDPOINT: &str = "https://lite.duckduckgo.com/lite/";
const DEFAULT_RESULTS: usize = 5;
const MAX_RESULTS: usize = 10;

#[derive(Debug, Clone, serde::Serialize)]
pub struct SearchResult {
    pub title: String,
    pub url: String,
    pub snippet: String,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct SearchResponse {
    pub query: String,
    pub results: Vec<SearchResult>,
}
```

Add the module declaration in the crate root used by the existing MCP modules.

- [x] **Step 3: Add deterministic validation and URL helpers.**

Implement and unit-test:

```rust
fn validate(query: &str, max_results: Option<usize>) -> Result<(String, usize), String>;
fn search_url(query: &str) -> String;
fn destination_url(href: &str) -> Option<String>;
```

Trim the query and reject it when empty. Use five results by default and reject values outside 1–10. Build the query with `url::form_urlencoded::Serializer` or an equivalent percent-encoding helper; never interpolate raw query text into the URL. For DuckDuckGo Lite links, accept absolute `http`/`https` URLs and decode `//duckduckgo.com/l/?uddg=<encoded-url>` into the destination URL. Reject other schemes and malformed redirects.

- [x] **Step 4: Parse DuckDuckGo Lite result cards.**

Use `scraper::Html` with selectors `.result-link` and `.result-snippet`. Pair each result link with the next snippet in document order, decode HTML entities through the parsed text, trim whitespace, and skip entries with no title or destination URL. Return at most the requested count. A valid page with no cards returns an empty result list.

- [x] **Step 5: Verify parser tests.**

Add tests for empty queries, bounds 1 and 10, rejection of 0 and 11, encoded queries, direct links, DuckDuckGo redirect links, HTML entities, malformed links, missing snippets, and result truncation. Run:

```bash
cargo test --lib web_search
```

Expected: all new parser tests pass without network access.

### Task 2: Implement the blocking search operation

**Files:**
- Modify: `src-tauri/src/web_search.rs`
- Test: `src-tauri/src/web_search.rs`

- [x] **Step 1: Build the HTTP client with bounded behavior.**

Implement:

```rust
pub fn search(query: &str, max_results: Option<usize>) -> Result<SearchResponse, String>;
```

Validate before creating the request. Build a `reqwest::blocking::Client` with a 15-second timeout and a clear `User-Agent` such as `conduit/<version>`. `GET` the Lite endpoint, append the encoded `q` parameter, accept any 2xx response including 202, and map non-2xx status codes to an error naming DuckDuckGo and the numeric status. Read the body only after a successful status and parse it with the pure parser from Task 1.

- [x] **Step 2: Keep transport errors actionable.**

Convert timeout, DNS, TLS, and body-read errors into `DuckDuckGo search failed: ...` messages without returning the response body. Detect known challenge markers before parsing and return `DuckDuckGo presented an anti-bot challenge; try again later`; do not retry, solve challenges, or execute page content. Never treat challenge text as a search result.

- [x] **Step 3: Run the module tests and build.**

Run:

```bash
cargo test --lib web_search
cargo check
```

Expected: parser and validation tests pass and the crate compiles on the current Linux target.

### Task 3: Expose `web_search` through MCP and the catalog

**Files:**
- Modify: `src-tauri/src/mcp/tools.rs`
- Modify: `src-tauri/src/mcp/catalog.rs`
- Test: `src-tauri/src/mcp/tools.rs` and existing catalog tests if present

- [x] **Step 1: Add the MCP argument shape.**

Add:

```rust
#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct WebSearchArgs {
    /// The words to search for.
    pub query: String,
    /// Number of links to return, from 1 to 10. Defaults to 5.
    #[serde(default)]
    pub max_results: Option<usize>,
}
```

- [x] **Step 2: Add the gated async handler.**

Add a `#[tool]` method named `web_search` in the `system` section. It should call `gate::run` with tool name `web_search`, preserve the agent identity, run `crate::web_search::search` in `tokio::task::spawn_blocking`, map join errors with `fail`, and serialize the `SearchResponse` with `json_ok`. The tool is read-only and must not be marked risky.

- [x] **Step 3: Add catalog metadata.**

Add `tool("web_search", System, "search the web with DuckDuckGo", false)` to `CATALOG`. Add action label `searching DuckDuckGo` and approval-summary phrasing `search the web` in `action_label` and `approval_summary`.

- [x] **Step 4: Add handler-level validation coverage.**

Test the pure request validation through the module tests and verify the generated tool catalog includes exactly one `web_search` entry with `risky: false`. Keep network tests out of the MCP handler tests.

### Task 4: Update the standalone catalog and validate the live endpoint

**Files:**
- Modify: `src/lib/standalone.ts`
- Create: `scripts/check-duckduckgo-search.py`
- Test: `scripts/check-duckduckgo-search.py` as a manual verification script

- [x] **Step 1: Mirror the catalog entry.**

Add the same system catalog object to `src/lib/standalone.ts` so the preview can render the tool even without Tauri.

- [x] **Step 2: Add the requested standalone search check.**

Create a Python script that directly requests `https://lite.duckduckgo.com/lite/?q=...` with a user agent, checks for a 2xx response, detects a challenge page, parses `.result-link` anchors with Python's standard-library `html.parser`, prints the query/status/result count and the first three title/URL pairs, and exits nonzero on transport, HTTP, or challenge failure. It must not call Conduit's MCP endpoint or mutate the user's clipboard/input.

- [x] **Step 3: Run full verification.**

Run:

```bash
cd src-tauri && cargo test --lib && cargo build
cd .. && pnpm build
python3 scripts/check-duckduckgo-search.py "Rust programming language"
git diff --check
```

Expected: Rust tests pass, the Tauri crate builds, the frontend build passes, the standalone script prints a successful 2xx response and at least one result in an environment where DuckDuckGo is reachable, and the diff is clean. If DuckDuckGo presents a challenge, record that live network limitation without weakening parser/error tests.

- [x] **Step 4: Commit the feature.**

```bash
git add src-tauri/Cargo.toml src-tauri/Cargo.lock src-tauri/src/web_search.rs src-tauri/src/mcp/tools.rs src-tauri/src/mcp/catalog.rs src/lib/standalone.ts scripts/check-duckduckgo-search.py docs/superpowers/specs/2026-09-07-duckduckgo-search-design.md docs/superpowers/plans/2026-09-07-duckduckgo-search.md
git commit -m "feat: add DuckDuckGo web search tool"
```
