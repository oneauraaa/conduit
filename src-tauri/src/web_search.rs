//! Keyless web search through DuckDuckGo's public Lite results page.
//!
//! This module deliberately returns links and snippets only. It does not run
//! JavaScript, solve challenges, fetch arbitrary result pages, or turn search
//! into an unrestricted browser. The MCP layer runs [`search`] on a blocking
//! thread so the synchronous HTTP client never occupies a Tokio worker.

use std::time::Duration;

use scraper::{Html, Selector};

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

/// Search DuckDuckGo Lite without an API key.
pub fn search(query: &str, max_results: Option<usize>) -> Result<SearchResponse, String> {
    let (query, limit) = validate(query, max_results)?;
    let url = search_url(&query);
    let client = reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(15))
        .user_agent(concat!("conduit/", env!("CARGO_PKG_VERSION")))
        .build()
        .map_err(|error| format!("DuckDuckGo search failed to start: {error}"))?;

    let response = client
        .get(url)
        .send()
        .map_err(|error| format!("DuckDuckGo search failed: {error}"))?;
    let status = response.status();
    if !status.is_success() {
        return Err(format!(
            "DuckDuckGo search returned HTTP {}",
            status.as_u16()
        ));
    }

    let body = response
        .text()
        .map_err(|error| format!("DuckDuckGo search failed while reading the response: {error}"))?;
    if is_challenge(&body) {
        return Err(
            "DuckDuckGo presented an anti-bot challenge; try the search again later".into(),
        );
    }

    Ok(SearchResponse {
        query,
        results: parse_results(&body, limit),
    })
}

fn validate(query: &str, max_results: Option<usize>) -> Result<(String, usize), String> {
    let query = query.trim();
    if query.is_empty() {
        return Err("DuckDuckGo search needs a non-empty query".into());
    }

    let limit = max_results.unwrap_or(DEFAULT_RESULTS);
    if !(1..=MAX_RESULTS).contains(&limit) {
        return Err(format!(
            "DuckDuckGo search accepts between 1 and {MAX_RESULTS} results"
        ));
    }
    Ok((query.to_string(), limit))
}

fn search_url(query: &str) -> String {
    let mut url = reqwest::Url::parse(ENDPOINT).expect("the DuckDuckGo endpoint is a URL literal");
    url.query_pairs_mut().append_pair("q", query);
    url.to_string()
}

fn destination_url(href: &str) -> Option<String> {
    let href = href.trim();
    let parsed = if href.starts_with("//") {
        reqwest::Url::parse(&format!("https:{href}"))
    } else {
        reqwest::Url::parse(href)
    }
    .ok()?;

    if parsed.host_str().is_some_and(|host| {
        host.eq_ignore_ascii_case("duckduckgo.com")
            || host.eq_ignore_ascii_case("www.duckduckgo.com")
    }) && parsed.path() == "/l/"
    {
        let target = parsed
            .query_pairs()
            .find_map(|(key, value)| (key == "uddg").then(|| value.into_owned()))?;
        let target = reqwest::Url::parse(&target).ok()?;
        return matches!(target.scheme(), "http" | "https").then(|| target.to_string());
    }

    matches!(parsed.scheme(), "http" | "https").then(|| parsed.to_string())
}

fn parse_results(body: &str, limit: usize) -> Vec<SearchResult> {
    let document = Html::parse_document(body);
    let entry_selector = Selector::parse("*").expect("valid result selector");
    let mut results = Vec::new();
    let mut pending: Option<SearchResult> = None;

    for element in document.select(&entry_selector) {
        let classes = element.value().attr("class").unwrap_or_default();
        if element.value().name() == "a" && classes.split_whitespace().any(|c| c == "result-link") {
            if let Some(result) = pending.take() {
                if !result.title.is_empty() && !result.url.is_empty() {
                    results.push(result);
                    if results.len() == limit {
                        break;
                    }
                }
            }

            let title = text_of(element.text());
            let url = element
                .value()
                .attr("href")
                .and_then(destination_url)
                .unwrap_or_default();
            pending = Some(SearchResult {
                title,
                url,
                snippet: String::new(),
            });
        } else if element.value().name() == "td"
            && classes.split_whitespace().any(|c| c == "result-snippet")
        {
            if let Some(result) = pending.as_mut() {
                result.snippet = text_of(element.text());
            }
        }
    }

    if results.len() < limit {
        if let Some(result) = pending {
            if !result.title.is_empty() && !result.url.is_empty() {
                results.push(result);
            }
        }
    }

    results
}

fn text_of<'a>(parts: impl Iterator<Item = &'a str>) -> String {
    parts
        .collect::<Vec<_>>()
        .join(" ")
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

fn is_challenge(body: &str) -> bool {
    let lower = body.to_ascii_lowercase();
    lower.contains("anomaly-modal")
        || lower.contains("challenge-form")
        || lower.contains("image-check_")
}

#[cfg(test)]
mod tests {
    use super::*;

    const PAGE: &str = r#"
        <table>
          <tr><td><a class="result-link" href="//duckduckgo.com/l/?uddg=https%3A%2F%2Fexample.com%2Fone">First &amp; Result</a></td></tr>
          <tr><td class="result-snippet">  A <b>useful</b> snippet. </td></tr>
          <tr><td><a class="result-link" href="https://example.org/two">Second</a></td></tr>
          <tr><td class="result-snippet">Second snippet</td></tr>
          <tr><td><a class="result-link" href="javascript:alert(1)">Ignored</a></td></tr>
          <tr><td class="result-snippet">Bad URL</td></tr>
        </table>
    "#;

    #[test]
    fn validates_query_and_result_bounds() {
        assert_eq!(validate("  rust  ", None).unwrap(), ("rust".into(), 5));
        assert!(validate("   ", None).is_err());
        assert!(validate("rust", Some(0)).is_err());
        assert_eq!(validate("rust", Some(1)).unwrap().1, 1);
        assert_eq!(validate("rust", Some(10)).unwrap().1, 10);
        assert!(validate("rust", Some(11)).is_err());
    }

    #[test]
    fn encodes_queries_in_the_request_url() {
        let url = search_url("rust & wayland");
        assert!(url.contains("q=rust+%26+wayland"), "{url}");
    }

    #[test]
    fn resolves_direct_and_duckduckgo_redirect_links() {
        assert_eq!(
            destination_url("https://example.com/path").as_deref(),
            Some("https://example.com/path")
        );
        assert_eq!(
            destination_url("//duckduckgo.com/l/?uddg=https%3A%2F%2Fexample.com%2Fpath").as_deref(),
            Some("https://example.com/path")
        );
        assert!(destination_url("javascript:alert(1)").is_none());
        assert!(destination_url("//duckduckgo.com/l/?uddg=file%3A%2F%2Fsecret").is_none());
    }

    #[test]
    fn parses_entities_skips_bad_urls_and_limits_results() {
        let results = parse_results(PAGE, 1);
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].title, "First & Result");
        assert_eq!(results[0].url, "https://example.com/one");
        assert_eq!(results[0].snippet, "A useful snippet.");

        let all = parse_results(PAGE, 10);
        assert_eq!(all.len(), 2);
    }

    #[test]
    fn keeps_a_valid_link_when_its_snippet_is_missing() {
        let body = r#"
            <table>
              <tr><td><a class="result-link" href="https://example.com/one">First</a></td></tr>
              <tr><td><a class="result-link" href="https://example.com/two">Second</a></td></tr>
              <tr><td class="result-snippet">Second snippet</td></tr>
            </table>
        "#;
        let results = parse_results(body, 10);
        assert_eq!(results.len(), 2);
        assert_eq!(results[0].title, "First");
        assert!(results[0].snippet.is_empty());
        assert_eq!(results[1].snippet, "Second snippet");
    }

    #[test]
    fn recognizes_challenge_pages() {
        assert!(is_challenge(
            "<form id='challenge-form'><input name='image-check_x'></form>"
        ));
        assert!(!is_challenge("<a class='result-link'>normal</a>"));
    }
}
