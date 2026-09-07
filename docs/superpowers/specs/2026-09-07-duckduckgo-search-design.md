# DuckDuckGo Search Design

**Date:** 2026-09-07  
**Status:** Approved for implementation by the user's request to continue with the recommended keyless HTML search route

## Goal

Expose a free, keyless DuckDuckGo web-search tool through Conduit's MCP endpoint so an agent can search the web without opening a browser or configuring a paid API key.

## Scope

### In scope

- One read-only MCP tool named `web_search`.
- A required query string and an optional bounded result count.
- DuckDuckGo's public HTML results endpoint, requested with a normal user agent.
- Structured results containing title, URL, and snippet text.
- Clear errors for empty queries, invalid result counts, network failures, non-success HTTP responses, malformed result pages, and rate limiting.
- Tool catalog, action labels, approval policy, standalone preview data, and unit tests updated with the new tool.

### Out of scope

- DuckDuckGo Instant Answers as the primary search source; it does not reliably return ordinary web results.
- Paid search APIs or API keys.
- Browser automation, JavaScript execution, CAPTCHA solving, or bypassing DuckDuckGo rate limits.
- Fetching and summarizing result pages; the tool returns links and snippets so an agent can decide whether to open a result through another mechanism.
- Search history, caching, or background polling.

## Architecture

The MCP handler remains thin and runs through the existing `gate::run` policy so the search appears in the server log and obeys the user's tool toggle. A small `web_search` platform-independent module owns the network request and HTML parsing; it returns a serializable `SearchResult` list and maps failures to user-readable strings.

The handler executes the blocking HTTP/parser work with `tokio::task::spawn_blocking`, keeping the MCP runtime responsive. The request URL is built with a URL-encoding helper, uses a short timeout, and asks for at most ten results. The parser extracts DuckDuckGo result anchors and snippets from the public HTML structure, normalizes relative result URLs, decodes entities, and skips malformed entries rather than returning invented data.

## Tool contract

```text
web_search(query: string, max_results?: integer) ->
  {
    query: string,
    results: [{ title: string, url: string, snippet: string }]
  }
```

`max_results` defaults to five and must be between 1 and 10 when supplied; values outside that range are rejected. An empty or whitespace-only query is rejected. Zero valid results is a successful response with an empty `results` array, allowing the agent to distinguish “nothing matched” from a network error.

## Error handling

- HTTP 403, 202, 429, or other non-success responses become a concise error naming DuckDuckGo and the status; no HTML body is returned.
- A transport or timeout error reports that DuckDuckGo could not be reached and preserves the original cause for logs.
- A page with no parseable result entries is treated as an empty result set unless the response is non-success; this avoids turning a normal no-results page into a false failure.
- The tool never retries automatically, never follows arbitrary redirects beyond the HTTP client's normal policy, and never executes returned content.

## UI/catalog

The tool belongs to the `system` group because it is a read-only external information operation rather than a screen or input action. It is non-risky, gets a “searching DuckDuckGo” action label, and appears in the standalone catalog. No new permission or settings control is needed.

## Testing

- Unit tests for query validation, result-count bounds, URL encoding, relative-link normalization, HTML entity decoding, and malformed/empty result entries.
- A standalone script using the same public HTML endpoint to verify a live query, print status/result count, and avoid invoking Conduit's MCP tool itself.
- Full Rust tests, Rust build, frontend typecheck/build, and `git diff --check`.

## User-visible result

An agent can call `web_search` on any supported Conduit platform without an API key. The result is a compact list of DuckDuckGo links/snippets; if DuckDuckGo is unreachable or rate-limits the request, the agent receives an explicit error and can decide whether to retry later.
