use super::*;
use crate::providers::ToolCall;

pub(crate) fn is_valid_search_query(query: &str) -> bool {
    let normalized = query.trim();
    normalized.chars().count() >= 2 && normalized.chars().count() <= 180
}

pub(crate) fn content_requests_web_search(question: &str) -> bool {
    let normalized = question.to_lowercase();
    [
        "web search",
        "online search",
        "search the internet",
        "联网搜索",
        "网络搜索",
        "使用网络搜索",
        "上网搜索",
        "可以上网",
        "帮我搜索",
        "搜索一下",
    ]
    .iter()
    .any(|phrase| normalized.contains(phrase))
}

async fn fetch_searxng_response(
    client: &Client,
    base: &str,
    query: &str,
    language: &str,
    engines: Option<&str>,
) -> Result<Value, reqwest::Error> {
    let mut request = client
        .get(format!("{}/search", base.trim_end_matches('/')))
        .query(&[("q", query), ("format", "json"), ("language", language)]);
    if let Some(engines) = engines {
        request = request.query(&[("engines", engines)]);
    }
    request.send().await?.error_for_status()?.json().await
}

fn search_results(response: Value) -> Vec<Value> {
    response.get("results").and_then(Value::as_array).cloned().unwrap_or_default().into_iter()
        .filter(|v| v.get("title").and_then(Value::as_str).is_some_and(|x| !x.trim().is_empty()) && v.get("url").and_then(Value::as_str).is_some_and(|x| x.starts_with("http")))
        .map(|v| json!({"title":v["title"].as_str().unwrap_or_default(),"url":v["url"].as_str().unwrap_or_default(),"content":v["content"].as_str().unwrap_or_default(),"engine":v["engine"].as_str().unwrap_or("SearXNG")}))
        .collect()
}

pub(crate) async fn fetch_search(state: &AppState, query: &str) -> Result<Vec<Value>, ApiError> {
    let base = state
        .searxng_url
        .as_deref()
        .ok_or_else(|| ApiError::bad("Online search is not configured."))?;
    let search_query = query.trim();
    let language = if search_query
        .chars()
        .any(|ch| ('\u{4e00}'..='\u{9fff}').contains(&ch))
    {
        "zh-CN"
    } else {
        "en-US"
    };
    let preferred_engines = state.searxng_preferred_engines.as_deref();
    let mut results = match preferred_engines {
        Some(engines) => {
            match fetch_searxng_response(&state.http, base, search_query, language, Some(engines))
                .await
            {
                Ok(response) => search_results(response),
                Err(error) => {
                    warn!(%engines, error=%error, "preferred web search engines failed; using aggregate search");
                    Vec::new()
                }
            }
        }
        None => Vec::new(),
    };
    if results.len() < MIN_PREFERRED_SEARCH_RESULTS {
        for result in search_results(
            fetch_searxng_response(&state.http, base, search_query, language, None).await?,
        ) {
            if !results
                .iter()
                .any(|existing| existing["url"] == result["url"])
            {
                results.push(result);
            }
        }
    }
    results.truncate(8);
    info!(query_length=query.chars().count(), result_count=results.len(), preferred_engines=?preferred_engines, "search query completed");
    Ok(results)
}

pub(crate) fn web_tool_definitions(kind: &str) -> Value {
    let search_schema = json!({
        "type": "object",
        "properties": {"query": {"type": "string", "description": "A concise, specific search-engine query."}},
        "required": ["query"],
        "additionalProperties": false
    });
    let open_schema = json!({
        "type": "object",
        "properties": {"url": {"type": "string", "description": "An exact http(s) URL returned by web_search."}},
        "required": ["url"],
        "additionalProperties": false
    });
    if kind == "anthropic" {
        json!([
            {"name":"web_search","description":"Search the public web for current, factual information. Use focused queries and refine them when the initial evidence is weak.","input_schema":search_schema},
            {"name":"open_web_page","description":"Read the text of an exact result URL returned by web_search. Use this to verify details before making consequential claims.","input_schema":open_schema}
        ])
    } else {
        json!([
            {"type":"function","function":{"name":"web_search","description":"Search the public web for current, factual information. Use focused queries and refine them when the initial evidence is weak.","parameters":search_schema}},
            {"type":"function","function":{"name":"open_web_page","description":"Read the text of an exact result URL returned by web_search. Use this to verify details before making consequential claims.","parameters":open_schema}}
        ])
    }
}

fn add_search_sources(sources: &mut Vec<Value>, query: &str, results: Vec<Value>) {
    for mut result in results {
        result["query"] = json!(query);
        if !sources
            .iter()
            .any(|existing| existing["url"] == result["url"])
        {
            sources.push(result);
        }
        if sources.len() >= MAX_WEB_SOURCES {
            break;
        }
    }
}

fn tool_search_results(results: &[Value]) -> Value {
    Value::Array(
        results
            .iter()
            .take(5)
            .map(|result| {
                let mut result = result.clone();
                if let Some(content) = result["content"].as_str() {
                    result["content"] = json!(content.chars().take(900).collect::<String>());
                }
                result
            })
            .collect(),
    )
}

pub(crate) fn is_safe_public_url(raw: &str) -> bool {
    let Ok(url) = reqwest::Url::parse(raw) else {
        return false;
    };
    if !matches!(url.scheme(), "http" | "https") {
        return false;
    }
    let Some(host) = url.host_str() else {
        return false;
    };
    let host = host.trim_matches(['[', ']']);
    if host.eq_ignore_ascii_case("localhost") || host.ends_with(".localhost") {
        return false;
    }
    match host.parse::<IpAddr>() {
        Ok(ip) => is_safe_public_ip(ip),
        Err(_) => true,
    }
}

fn is_safe_public_ip(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(ip) => {
            !ip.is_private()
                && !ip.is_loopback()
                && !ip.is_link_local()
                && !ip.is_unspecified()
                && !ip.is_broadcast()
        }
        IpAddr::V6(ip) => {
            !ip.is_loopback()
                && !ip.is_unspecified()
                && !ip.is_unique_local()
                && !ip.is_unicast_link_local()
        }
    }
}

async fn resolves_to_public_addresses(raw: &str) -> bool {
    let Ok(url) = reqwest::Url::parse(raw) else {
        return false;
    };
    let Some(host) = url.host_str() else {
        return false;
    };
    if host.trim_matches(['[', ']']).parse::<IpAddr>().is_ok() {
        return true;
    }
    let port = url.port_or_known_default().unwrap_or(443);
    match tokio::net::lookup_host((host, port)).await {
        Ok(addresses) => {
            let addresses = addresses.collect::<Vec<_>>();
            !addresses.is_empty()
                && addresses
                    .iter()
                    .all(|address| is_safe_public_ip(address.ip()))
        }
        Err(_) => false,
    }
}

fn html_to_text(html: &str) -> String {
    let mut compact = html.to_string();
    for tag in ["script", "style", "noscript", "svg"] {
        let lower = compact.to_ascii_lowercase();
        let mut cursor = 0;
        while let Some(start) = lower[cursor..].find(&format!("<{tag}")) {
            let start = cursor + start;
            let Some(end) = lower[start..].find(&format!("</{tag}>")) else {
                compact.replace_range(start..compact.len(), &" ".repeat(compact.len() - start));
                break;
            };
            let end = start + end + tag.len() + 3;
            compact.replace_range(start..end, &" ".repeat(end - start));
            cursor = end;
        }
    }
    let mut text = String::with_capacity(compact.len().min(24_000));
    let mut in_tag = false;
    for character in compact.chars() {
        match character {
            '<' => {
                in_tag = true;
                text.push(' ');
            }
            '>' => in_tag = false,
            _ if !in_tag => text.push(character),
            _ => {}
        }
    }
    text = text
        .replace("&nbsp;", " ")
        .replace("&amp;", "&")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&#39;", "'");
    text.split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .chars()
        .take(12_000)
        .collect()
}

async fn open_search_result(
    state: &AppState,
    url: &str,
    allowed_urls: &[String],
) -> Result<Value, String> {
    if !allowed_urls.iter().any(|allowed| allowed == url)
        || !is_safe_public_url(url)
        || !resolves_to_public_addresses(url).await
    {
        return Err("The URL must be an exact public result returned by web_search.".into());
    }
    let response = state
        .web_reader
        .get(url)
        .header("Accept", "text/html, text/plain;q=0.9")
        .send()
        .await
        .map_err(|_| "The page could not be retrieved.".to_string())?
        .error_for_status()
        .map_err(|_| "The page returned an error response.".to_string())?;
    let content_type = response
        .headers()
        .get(http::header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .unwrap_or("")
        .to_ascii_lowercase();
    if !content_type.is_empty()
        && !content_type.contains("text/")
        && !content_type.contains("html")
        && !content_type.contains("xml")
    {
        return Err("The result is not a readable text page.".into());
    }
    let mut bytes = Vec::new();
    let mut body = response.bytes_stream();
    while let Some(next) = body.next().await {
        let chunk = next.map_err(|_| "The page could not be read.".to_string())?;
        let remaining = 1_000_000usize.saturating_sub(bytes.len());
        if remaining == 0 {
            break;
        }
        bytes.extend_from_slice(&chunk[..chunk.len().min(remaining)]);
    }
    let raw = String::from_utf8_lossy(&bytes);
    let content = if content_type.contains("html") || raw.contains("<html") {
        html_to_text(&raw)
    } else {
        raw.split_whitespace()
            .collect::<Vec<_>>()
            .join(" ")
            .chars()
            .take(12_000)
            .collect()
    };
    if content.trim().is_empty() {
        return Err("The page did not contain readable text.".into());
    }
    Ok(json!({"url":url,"content":content}))
}

pub(crate) async fn execute_web_tool(
    state: &AppState,
    call: &ToolCall,
    sources: &mut Vec<Value>,
    allowed_urls: &mut Vec<String>,
) -> Value {
    match call.name.as_str() {
        "web_search" => {
            let query = call.input["query"]
                .as_str()
                .unwrap_or("")
                .split_whitespace()
                .collect::<Vec<_>>()
                .join(" ");
            if !is_valid_search_query(&query) {
                return json!({"error":"web_search requires a focused query between 2 and 180 characters."});
            }
            match fetch_search(state, &query).await {
                Ok(results) => {
                    add_search_sources(sources, &query, results);
                    for result in sources.iter() {
                        if result["query"] == query {
                            if let Some(url) = result["url"].as_str() {
                                if is_safe_public_url(url)
                                    && !allowed_urls.iter().any(|allowed| allowed == url)
                                {
                                    allowed_urls.push(url.to_string());
                                }
                            }
                        }
                    }
                    let matching = sources
                        .iter()
                        .filter(|result| result["query"] == query)
                        .cloned()
                        .collect::<Vec<_>>();
                    json!({"query":query,"results":tool_search_results(&matching)})
                }
                Err(error) => json!({"error":error.message}),
            }
        }
        "open_web_page" => {
            let url = call.input["url"].as_str().unwrap_or("");
            match open_search_result(state, url, allowed_urls).await {
                Ok(page) => {
                    if let Some(source) = sources
                        .iter_mut()
                        .find(|source| source["url"] == page["url"])
                    {
                        source["content"] = page["content"].clone();
                    }
                    page
                }
                Err(message) => json!({"error":message}),
            }
        }
        _ => json!({"error":"Unknown tool. Only web_search and open_web_page are available."}),
    }
}
