//! `web_search` tool — DuckDuckGo HTML endpoint, no API key.
//!
//! We deliberately use `html.duckduckgo.com/html/` (the no-JS surface)
//! and parse it with `scraper`. Returns a small list of result snippets.

use rig::completion::ToolDefinition;
use rig::tool::Tool;
use scraper::{Html, Selector};
use serde::{Deserialize, Serialize};

use super::ToolContext;

#[derive(Debug, thiserror::Error)]
#[error("{0}")]
pub struct WebSearchError(String);

impl From<reqwest::Error> for WebSearchError {
    fn from(e: reqwest::Error) -> Self {
        Self(e.to_string())
    }
}

#[derive(Debug, Deserialize)]
pub struct WebSearchArgs {
    pub query: String,
    #[serde(default = "default_k")]
    pub k: usize,
}
fn default_k() -> usize {
    5
}

#[derive(Debug, Serialize, Clone)]
pub struct WebResult {
    pub title: String,
    pub url: String,
    pub snippet: String,
}

#[derive(Clone, Default)]
pub struct WebSearch {
    /// Optional override for testing (e.g. a local mock server).
    pub endpoint: Option<String>,
    /// Optional tool context so calls show up on the event bus.
    pub ctx: Option<ToolContext>,
}

impl WebSearch {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_ctx(mut self, ctx: ToolContext) -> Self {
        self.ctx = Some(ctx);
        self
    }

    fn base_url(&self) -> &str {
        self.endpoint
            .as_deref()
            .unwrap_or("https://html.duckduckgo.com/html/")
    }
}

impl Tool for WebSearch {
    const NAME: &'static str = "web_search";
    type Error = WebSearchError;
    type Args = WebSearchArgs;
    type Output = Vec<WebResult>;

    async fn definition(&self, _prompt: String) -> ToolDefinition {
        ToolDefinition {
            name: Self::NAME.into(),
            description: "Search the web (DuckDuckGo) and return titles, URLs, and snippets."
                .into(),
            parameters: serde_json::json!({
                "type": "object",
                "properties": {
                    "query": {"type": "string"},
                    "k":     {"type": "integer", "minimum": 1, "default": 5}
                },
                "required": ["query"]
            }),
        }
    }

    async fn call(&self, args: Self::Args) -> Result<Self::Output, Self::Error> {
        let guard = self
            .ctx
            .as_ref()
            .map(|c| c.begin_call(Self::NAME, format!("query={:?} k={}", args.query, args.k)));
        let client = match reqwest::Client::builder()
            .user_agent("hal/0.1 (+https://x.ai)")
            .timeout(std::time::Duration::from_secs(10))
            .build()
        {
            Ok(c) => c,
            Err(e) => {
                if let Some(g) = guard {
                    g.finish_err(e.to_string());
                }
                return Err(e.into());
            }
        };
        let res = match client
            .get(self.base_url())
            .query(&[("q", args.query.as_str())])
            .send()
            .await
        {
            Ok(r) => r,
            Err(e) => {
                if let Some(g) = guard {
                    g.finish_err(e.to_string());
                }
                return Err(e.into());
            }
        };
        let body = match res.text().await {
            Ok(b) => b,
            Err(e) => {
                if let Some(g) = guard {
                    g.finish_err(e.to_string());
                }
                return Err(e.into());
            }
        };
        let out = parse_duckduckgo_html(&body, args.k);
        if let Some(g) = guard {
            g.finish_ok(format!("{} result(s)", out.len()));
        }
        Ok(out)
    }
}

pub(crate) fn parse_duckduckgo_html(body: &str, k: usize) -> Vec<WebResult> {
    let doc = Html::parse_document(body);
    let result_sel = Selector::parse("div.result, div.web-result").unwrap();
    let title_sel = Selector::parse("a.result__a, h2 a").unwrap();
    let snippet_sel = Selector::parse("a.result__snippet, .result__snippet").unwrap();

    let mut out = Vec::with_capacity(k);
    for res in doc.select(&result_sel) {
        let title = res
            .select(&title_sel)
            .next()
            .map(|e| e.text().collect::<String>().trim().to_string())
            .unwrap_or_default();
        let url = res
            .select(&title_sel)
            .next()
            .and_then(|e| e.value().attr("href"))
            .unwrap_or("")
            .to_string();
        let snippet = res
            .select(&snippet_sel)
            .next()
            .map(|e| e.text().collect::<String>().trim().to_string())
            .unwrap_or_default();
        if !title.is_empty() && !url.is_empty() {
            out.push(WebResult {
                title,
                url,
                snippet,
            });
        }
        if out.len() >= k {
            break;
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_picks_up_results() {
        let html = r#"
        <html><body>
          <div class="result">
            <a class="result__a" href="https://example.com/a">Alpha</a>
            <a class="result__snippet">about alpha</a>
          </div>
          <div class="result">
            <a class="result__a" href="https://example.com/b">Beta</a>
            <a class="result__snippet">about beta</a>
          </div>
          <div class="result">
            <a class="result__a" href="https://example.com/c">Gamma</a>
          </div>
        </body></html>"#;
        let r = parse_duckduckgo_html(html, 2);
        assert_eq!(r.len(), 2);
        assert_eq!(r[0].title, "Alpha");
        assert_eq!(r[0].url, "https://example.com/a");
        assert!(r[0].snippet.contains("alpha"));
    }

    #[test]
    fn parse_handles_empty_body() {
        let r = parse_duckduckgo_html("<html></html>", 5);
        assert!(r.is_empty());
    }
}
