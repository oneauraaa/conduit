#!/usr/bin/env python3
"""Exercise DuckDuckGo Lite directly, without starting or calling Conduit's MCP server."""

from html.parser import HTMLParser
from urllib.parse import parse_qs, urlencode, urlparse
from urllib.request import Request, urlopen
import sys


ENDPOINT = "https://lite.duckduckgo.com/lite/"


class Results(HTMLParser):
    def __init__(self):
        super().__init__()
        self.links = []
        self.snippets = []
        self._link = None
        self._snippet = None

    def handle_starttag(self, tag, attrs):
        attrs = dict(attrs)
        classes = set(attrs.get("class", "").split())
        if tag == "a" and "result-link" in classes:
            self._link = [attrs.get("href", ""), []]
        elif tag == "td" and "result-snippet" in classes:
            self._snippet = []

    def handle_data(self, data):
        if self._link is not None:
            self._link[1].append(data)
        if self._snippet is not None:
            self._snippet.append(data)

    def handle_endtag(self, tag):
        if tag == "a" and self._link is not None:
            self.links.append((self._link[0], " ".join(self._link[1]).split()))
            self._link = None
        elif tag == "td" and self._snippet is not None:
            self.snippets.append(" ".join(self._snippet).split())
            self._snippet = None


def destination(href):
    parsed = urlparse(href if not href.startswith("//") else "https:" + href)
    if parsed.netloc.lower() in {"duckduckgo.com", "www.duckduckgo.com"} and parsed.path == "/l/":
        target = parse_qs(parsed.query).get("uddg", [None])[0]
        if target is None:
            return None
        parsed = urlparse(target)
    if parsed.scheme not in {"http", "https"} or not parsed.netloc:
        return None
    return parsed.geturl()


def main():
    query = " ".join(sys.argv[1:]).strip() or "DuckDuckGo Lite"
    url = ENDPOINT + "?" + urlencode({"q": query})
    request = Request(url, headers={"User-Agent": "conduit-duckduckgo-check/1.0"})
    try:
        with urlopen(request, timeout=15) as response:
            body = response.read().decode("utf-8", "replace")
            status = response.status
    except Exception as error:
        print(f"request failed: {error}", file=sys.stderr)
        return 1

    lower = body.lower()
    if "anomaly-modal" in lower or "challenge-form" in lower or "image-check_" in lower:
        print("DuckDuckGo returned an anti-bot challenge", file=sys.stderr)
        return 1

    parsed = Results()
    parsed.feed(body)
    pairs = []
    for index, (href, title_parts) in enumerate(parsed.links):
        link = destination(href)
        if link is None or not title_parts:
            continue
        snippet = " ".join(parsed.snippets[index]) if index < len(parsed.snippets) else ""
        pairs.append((" ".join(title_parts), link, snippet))

    print(f"HTTP {status}; query={query!r}; results={len(pairs)}")
    for title, link, snippet in pairs[:3]:
        print(f"- {title}\n  {link}\n  {snippet}")
    return 0 if status >= 200 and status < 300 and pairs else 1


if __name__ == "__main__":
    raise SystemExit(main())
