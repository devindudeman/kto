use scraper::{Html, Selector};

use crate::error::{KtoError, Result};
use crate::fetch::PageContent;
use crate::transforms::Intent;
use crate::validate;
use crate::watch::Extraction;

/// Extract content from a page based on the extraction strategy
pub fn extract(content: &PageContent, strategy: &Extraction) -> Result<String> {
    match strategy {
        Extraction::Auto => extract_auto(content),
        Extraction::Selector { selector } => extract_selector(&content.html, selector),
        Extraction::Full => extract_full(content),
        Extraction::Meta { tags } => extract_meta(&content.html, tags),
        Extraction::Rss => extract_rss(content),
        Extraction::JsonLd { types } => extract_jsonld(&content.html, types.as_ref()),
    }
}

/// Extract RSS feed content (use pre-formatted text from fetch)
fn extract_rss(content: &PageContent) -> Result<String> {
    content.text.clone().ok_or_else(|| {
        KtoError::ExtractionError("No RSS content available - feed may be empty or malformed".into())
    })
}

/// Minimum characters for readability output to be considered useful.
/// If readability returns less than this, fall back to full body extraction.
/// This prevents over-aggressive extraction that misses important content like stock buttons.
const MIN_READABILITY_CHARS: usize = 50;

/// Auto-detect main content using readability
fn extract_auto(content: &PageContent) -> Result<String> {
    // Try readability-js first
    if let Ok(readability) = readability_js::Readability::new() {
        if let Ok(article) = readability.parse(&content.html) {
            // Use text_content which is the cleaned plain text
            let text = article.text_content.trim();
            // Only use readability result if it's substantial enough.
            // Very short results often mean readability stripped too much content
            // (e.g., product pages where it keeps only the price).
            if text.len() >= MIN_READABILITY_CHARS {
                return Ok(article.text_content);
            }
        }
    }

    // Fallback: if we have text from Playwright, use that
    if let Some(ref text) = content.text {
        if !text.trim().is_empty() {
            return Ok(text.clone());
        }
    }

    // Fallback: extract text from body ourselves (more complete than readability)
    let document = Html::parse_document(&content.html);
    let body_selector =
        Selector::parse("body").map_err(|e| KtoError::ExtractionError(format!("{:?}", e)))?;

    if let Some(body) = document.select(&body_selector).next() {
        let text: String = body.text().collect::<Vec<_>>().join(" ");
        if !text.trim().is_empty() {
            return Ok(text);
        }
    }

    Err(KtoError::ExtractionError(
        "Could not extract any content from page".into(),
    ))
}

/// Extract content using a CSS selector
fn extract_selector(html: &str, selector_str: &str) -> Result<String> {
    let document = Html::parse_document(html);
    let selector = Selector::parse(selector_str)
        .map_err(|e| KtoError::ExtractionError(format!("Invalid selector: {:?}", e)))?;

    let texts: Vec<String> = document
        .select(&selector)
        .map(|el| el.text().collect::<Vec<_>>().join(" "))
        .collect();

    if texts.is_empty() {
        return Err(KtoError::ExtractionError(format!(
            "Selector '{}' matched no elements",
            selector_str
        )));
    }

    Ok(texts.join("\n"))
}

/// Extract full page body text
fn extract_full(content: &PageContent) -> Result<String> {
    // If we have Playwright text, use it
    if let Some(ref text) = content.text {
        return Ok(text.clone());
    }

    // Otherwise parse HTML
    let document = Html::parse_document(&content.html);
    let body_selector =
        Selector::parse("body").map_err(|e| KtoError::ExtractionError(format!("{:?}", e)))?;

    if let Some(body) = document.select(&body_selector).next() {
        let text: String = body.text().collect::<Vec<_>>().join(" ");
        return Ok(text);
    }

    Err(KtoError::ExtractionError(
        "Could not find body element".into(),
    ))
}

/// Extract specific meta tags
fn extract_meta(html: &str, tags: &[String]) -> Result<String> {
    let document = Html::parse_document(html);
    let mut values = Vec::new();

    for tag in tags {
        // Try meta[name="..."]
        let name_selector = Selector::parse(&format!("meta[name=\"{}\"]", tag))
            .map_err(|e| KtoError::ExtractionError(format!("{:?}", e)))?;

        for el in document.select(&name_selector) {
            if let Some(content) = el.value().attr("content") {
                values.push(content.to_string());
            }
        }

        // Try meta[property="..."] (for og: tags)
        let prop_selector = Selector::parse(&format!("meta[property=\"{}\"]", tag))
            .map_err(|e| KtoError::ExtractionError(format!("{:?}", e)))?;

        for el in document.select(&prop_selector) {
            if let Some(content) = el.value().attr("content") {
                values.push(content.to_string());
            }
        }
    }

    if values.is_empty() {
        return Err(KtoError::ExtractionError(format!(
            "No meta tags found for: {:?}",
            tags
        )));
    }

    Ok(values.join("\n"))
}

/// Extract JSON-LD structured data from the page
fn extract_jsonld(html: &str, type_filter: Option<&Vec<String>>) -> Result<String> {
    let document = Html::parse_document(html);
    let jsonld_selector = Selector::parse(r#"script[type="application/ld+json"]"#)
        .map_err(|e| KtoError::ExtractionError(format!("{:?}", e)))?;

    let mut extracted_items = Vec::new();

    for script in document.select(&jsonld_selector) {
        let text: String = script.text().collect();
        if let Ok(json) = serde_json::from_str::<serde_json::Value>(&text) {
            // Extract items from JSON-LD (handles @graph, arrays, and single objects)
            let items = flatten_jsonld(&json, type_filter);
            extracted_items.extend(items);
        }
    }

    if extracted_items.is_empty() {
        return Err(KtoError::ExtractionError(
            "No JSON-LD structured data found".into(),
        ));
    }

    // Format as human-readable text
    let formatted = extracted_items
        .into_iter()
        .map(format_jsonld_item)
        .collect::<Vec<_>>()
        .join("\n\n---\n\n");

    Ok(formatted)
}

/// Flatten JSON-LD structure into individual items, optionally filtering by type
fn flatten_jsonld(
    json: &serde_json::Value,
    type_filter: Option<&Vec<String>>,
) -> Vec<serde_json::Value> {
    let mut items = Vec::new();

    match json {
        serde_json::Value::Object(map) => {
            // Check for @graph (array of items)
            if let Some(serde_json::Value::Array(graph)) = map.get("@graph") {
                for item in graph {
                    items.extend(flatten_jsonld(item, type_filter));
                }
            } else {
                // Single object - check if it matches type filter
                if matches_type_filter(json, type_filter) {
                    items.push(json.clone());
                }
            }
        }
        serde_json::Value::Array(arr) => {
            for item in arr {
                items.extend(flatten_jsonld(item, type_filter));
            }
        }
        _ => {}
    }

    items
}

/// Check if a JSON-LD item matches the type filter
/// Handles namespaced types (e.g., "schema:Product" matches "Product")
fn matches_type_filter(json: &serde_json::Value, filter: Option<&Vec<String>>) -> bool {
    let filter = match filter {
        Some(f) if !f.is_empty() => f,
        _ => return true, // No filter means match all
    };

    if let Some(type_value) = json.get("@type") {
        match type_value {
            serde_json::Value::String(s) => type_matches_filter(s, filter),
            serde_json::Value::Array(arr) => arr.iter().any(|t| {
                if let serde_json::Value::String(s) = t {
                    type_matches_filter(s, filter)
                } else {
                    false
                }
            }),
            _ => false,
        }
    } else {
        false
    }
}

/// Check if a type string matches any filter, handling namespaced types
fn type_matches_filter(type_str: &str, filter: &[String]) -> bool {
    // Strip namespace prefix if present (e.g., "schema:Product" -> "Product")
    let clean_type = type_str
        .rsplit_once(':')
        .map(|(_, t)| t)
        .unwrap_or(type_str);

    filter.iter().any(|f| {
        // Also strip namespace from filter if present
        let clean_filter = f.rsplit_once(':').map(|(_, t)| t).unwrap_or(f);

        // Check for exact match (case insensitive)
        clean_type.eq_ignore_ascii_case(clean_filter)
            // Also check if type contains filter (e.g., "NewsArticle" contains "Article")
            || clean_type.to_lowercase().contains(&clean_filter.to_lowercase())
    })
}

/// Format a JSON-LD item as human-readable text
fn format_jsonld_item(json: serde_json::Value) -> String {
    let mut lines = Vec::new();

    // Get the type
    if let Some(t) = json.get("@type") {
        let type_str = match t {
            serde_json::Value::String(s) => s.clone(),
            serde_json::Value::Array(arr) => arr
                .iter()
                .filter_map(|v| v.as_str())
                .collect::<Vec<_>>()
                .join(", "),
            _ => "Unknown".to_string(),
        };
        lines.push(format!("[{}]", type_str));
    }

    // Extract common fields based on type
    let type_hint = json.get("@type").and_then(|t| t.as_str()).unwrap_or("");

    // Product fields
    if type_hint.eq_ignore_ascii_case("product") {
        if let Some(name) = json.get("name").and_then(|v| v.as_str()) {
            lines.push(format!("Name: {}", name));
        }
        if let Some(offers) = json.get("offers") {
            extract_offer_info(&mut lines, offers);
        }
        if let Some(sku) = json.get("sku").and_then(|v| v.as_str()) {
            lines.push(format!("SKU: {}", sku));
        }
        if let Some(brand) = extract_nested_name(&json, "brand") {
            lines.push(format!("Brand: {}", brand));
        }
    }
    // Article fields
    else if type_hint.eq_ignore_ascii_case("article")
        || type_hint.eq_ignore_ascii_case("newsarticle")
        || type_hint.eq_ignore_ascii_case("blogposting")
    {
        if let Some(headline) = json.get("headline").and_then(|v| v.as_str()) {
            lines.push(format!("Headline: {}", headline));
        }
        if let Some(author) = extract_nested_name(&json, "author") {
            lines.push(format!("Author: {}", author));
        }
        if let Some(date) = json.get("datePublished").and_then(|v| v.as_str()) {
            lines.push(format!("Published: {}", date));
        }
        if let Some(date) = json.get("dateModified").and_then(|v| v.as_str()) {
            lines.push(format!("Modified: {}", date));
        }
    }
    // Event fields
    else if type_hint.eq_ignore_ascii_case("event") {
        if let Some(name) = json.get("name").and_then(|v| v.as_str()) {
            lines.push(format!("Event: {}", name));
        }
        if let Some(date) = json.get("startDate").and_then(|v| v.as_str()) {
            lines.push(format!("Start: {}", date));
        }
        if let Some(date) = json.get("endDate").and_then(|v| v.as_str()) {
            lines.push(format!("End: {}", date));
        }
        if let Some(loc) = extract_nested_name(&json, "location") {
            lines.push(format!("Location: {}", loc));
        }
    }
    // Generic fallback - just show name and description
    else {
        if let Some(name) = json.get("name").and_then(|v| v.as_str()) {
            lines.push(format!("Name: {}", name));
        }
        if let Some(desc) = json.get("description").and_then(|v| v.as_str()) {
            let truncated: String = desc.chars().take(200).collect();
            lines.push(format!("Description: {}...", truncated));
        }
    }

    if lines.len() <= 1 {
        // Just the type - add a generic summary
        format!("{}\n{}", lines.join("\n"), json.to_string())
    } else {
        lines.join("\n")
    }
}

/// Extract offer information (price, availability) from JSON-LD
/// Handles both single offers and arrays of offers
fn extract_offer_info(lines: &mut Vec<String>, offers: &serde_json::Value) {
    let offer_list: Vec<&serde_json::Value> = match offers {
        serde_json::Value::Array(arr) => arr.iter().collect(),
        obj @ serde_json::Value::Object(_) => vec![obj],
        _ => return,
    };

    // Limit to first 5 offers to avoid overwhelming output
    let max_offers = 5;
    let show_count = offer_list.len() > max_offers;

    for (i, offer) in offer_list.iter().take(max_offers).enumerate() {
        let mut offer_parts = Vec::new();

        // Extract seller if present
        if let Some(seller) = offer.get("seller").and_then(|s| {
            s.get("name").and_then(|n| n.as_str())
                .or_else(|| s.as_str())
        }) {
            offer_parts.push(seller.to_string());
        }

        // Extract price
        if let Some(price) = offer.get("price") {
            let currency = offer
                .get("priceCurrency")
                .and_then(|c| c.as_str())
                .unwrap_or("USD");
            let price_str = match price {
                serde_json::Value::String(s) => s.clone(),
                serde_json::Value::Number(n) => n.to_string(),
                _ => price.to_string(),
            };
            offer_parts.push(format!("{} {}", price_str, currency));
        }

        // Extract availability
        if let Some(avail) = offer.get("availability").and_then(|v| v.as_str()) {
            let avail_clean = avail
                .replace("https://schema.org/", "")
                .replace("http://schema.org/", "");
            offer_parts.push(avail_clean);
        }

        // Format offer line
        if !offer_parts.is_empty() {
            if offer_list.len() > 1 {
                lines.push(format!("Offer {}: {}", i + 1, offer_parts.join(" - ")));
            } else {
                // Single offer - just show the parts
                if let Some(price_part) = offer_parts.iter().find(|p| p.contains("USD") || p.contains("EUR") || p.contains("GBP") || p.chars().next().map(|c| c.is_ascii_digit()).unwrap_or(false)) {
                    lines.push(format!("Price: {}", price_part));
                }
                if let Some(avail_part) = offer_parts.iter().find(|p| p.contains("Stock") || p.contains("Available")) {
                    lines.push(format!("Availability: {}", avail_part));
                }
            }
        }
    }

    if show_count {
        lines.push(format!("... and {} more offers", offer_list.len() - max_offers));
    }
}

/// Extract name from a nested object (e.g., brand.name, author.name)
fn extract_nested_name(json: &serde_json::Value, field: &str) -> Option<String> {
    let value = json.get(field)?;
    match value {
        serde_json::Value::String(s) => Some(s.clone()),
        serde_json::Value::Object(_) => value.get("name").and_then(|n| n.as_str().map(String::from)),
        serde_json::Value::Array(arr) => arr.first().and_then(|v| match v {
            serde_json::Value::String(s) => Some(s.clone()),
            serde_json::Value::Object(_) => v.get("name").and_then(|n| n.as_str().map(String::from)),
            _ => None,
        }),
        _ => None,
    }
}

// ---------------------------------------------------------------------------
// Multi-strategy extraction comparison
// ---------------------------------------------------------------------------

/// A candidate extraction strategy with its extracted content
#[derive(Debug, Clone)]
pub struct ExtractionCandidate {
    pub strategy: Extraction,
    pub content: String,
}

/// Score for an extraction candidate
#[derive(Debug, Clone)]
pub struct ExtractionScore {
    pub total: f32,
    pub signals: Vec<(String, f32)>,
}

/// Detected page type based on HTML signals (no site-specific code)
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PageType {
    FinanceData,
    ECommerce,
    Article,
    Dashboard,
    SPA,
    Generic,
}

/// Detect page type from HTML signals
pub fn detect_page_type(html: &str) -> PageType {
    let lower = html.to_lowercase();

    // Finance: ticker, exchange, bid/ask, market cap, 52-week
    let finance_signals = [
        "ticker", "exchange", "bid", "ask", "market cap", "52-week",
        "stock price", "quote", "trading", "portfolio",
    ];
    let finance_hits = finance_signals.iter().filter(|s| lower.contains(*s)).count();
    if finance_hits >= 2 {
        return PageType::FinanceData;
    }

    // E-commerce: add to cart, product-detail, JSON-LD Product
    let ecom_signals = [
        "add to cart", "add-to-cart", "addtocart", "product-detail",
        "product_detail", "buy now", "\"@type\":\"product\"",
        "\"@type\": \"product\"", "shopify", "woocommerce",
    ];
    let ecom_hits = ecom_signals.iter().filter(|s| lower.contains(*s)).count();
    if ecom_hits >= 1 {
        return PageType::ECommerce;
    }

    // Dashboard/SPA detection
    let spa_signals = [
        "__next_data__", "__nuxt__", "data-reactroot", "data-reactid",
        "ng-app", "ng-controller", "__vue__",
    ];
    let spa_hits = spa_signals.iter().filter(|s| lower.contains(*s)).count();
    if spa_hits >= 1 {
        // Dashboard: SPA + data-heavy indicators
        let dashboard_signals = [
            "dashboard", "widget", "metric", "chart", "graph", "analytics",
        ];
        let dashboard_hits = dashboard_signals.iter().filter(|s| lower.contains(*s)).count();
        if dashboard_hits >= 1 {
            return PageType::Dashboard;
        }
        return PageType::SPA;
    }

    // Article: <article> tags, long paragraph text
    if lower.contains("<article") || lower.contains("class=\"article") {
        return PageType::Article;
    }

    PageType::Generic
}

/// Run Auto, Full, and JsonLd extraction strategies on the same already-fetched HTML.
/// No extra fetches — all strategies parse the HTML we already have.
pub fn extract_all_strategies(content: &PageContent) -> Vec<ExtractionCandidate> {
    let strategies = vec![
        Extraction::Auto,
        Extraction::Full,
        Extraction::JsonLd { types: None },
    ];

    strategies
        .into_iter()
        .filter_map(|strategy| {
            match extract(content, &strategy) {
                Ok(extracted) if !extracted.trim().is_empty() => {
                    Some(ExtractionCandidate {
                        strategy,
                        content: extracted,
                    })
                }
                _ => None,
            }
        })
        .collect()
}

/// Score an extraction candidate against the user's intent.
/// Returns a score between 0.0 and 1.0 with signal breakdown.
pub fn score_candidate(
    content: &str,
    strategy: &Extraction,
    intent: Intent,
    page_type: PageType,
) -> ExtractionScore {
    let mut signals = Vec::new();
    let mut total: f32 = 0.0;

    // 1. Specificity (0.30): ratio of intent-relevant tokens to total tokens
    let specificity = compute_specificity(content, intent);
    signals.push(("specificity".to_string(), specificity));
    total += specificity * 0.30;

    // 2. Focus (0.20): penalize too short or too long, sweet spot 200–5000 chars
    let focus = compute_focus(content.len());
    signals.push(("focus".to_string(), focus));
    total += focus * 0.20;

    // 3. Intent alignment (0.25): does content contain expected patterns?
    let alignment = if validate::check_expected_type(content, intent) {
        1.0
    } else {
        0.2
    };
    signals.push(("intent_alignment".to_string(), alignment));
    total += alignment * 0.25;

    // 4. Extraction stability (0.15): structured data > unstructured
    let stability = validate::stability_for_extraction(strategy);
    signals.push(("stability".to_string(), stability));
    total += stability * 0.15;

    // 5. Not boilerplate (0.10): penalize sidebar/about/Wikipedia text
    let boilerplate = is_boilerplate(content);
    let not_boilerplate = if boilerplate { 0.0 } else { 1.0 };
    signals.push(("not_boilerplate".to_string(), not_boilerplate));
    total += not_boilerplate * 0.10;

    // Page-type adjustments (Phase 4)
    let page_adj = page_type_adjustment(strategy, page_type);
    if page_adj.abs() > f32::EPSILON {
        signals.push(("page_type_adj".to_string(), page_adj));
        total = (total + page_adj).clamp(0.0, 1.0);
    }

    ExtractionScore {
        total: total.clamp(0.0, 1.0),
        signals,
    }
}

/// Pick the best extraction strategy for a page given the user's intent.
/// Returns `(Extraction, extracted_content)`. Falls back to Auto if all strategies fail.
pub fn pick_best_extraction(
    content: &PageContent,
    intent: Intent,
) -> (Extraction, String) {
    let candidates = extract_all_strategies(content);
    if candidates.is_empty() {
        // Absolute fallback
        let fallback = extract(content, &Extraction::Auto)
            .unwrap_or_default();
        return (Extraction::Auto, fallback);
    }

    let page_type = detect_page_type(&content.html);

    let mut best_idx = 0;
    let mut best_score: f32 = -1.0;
    let mut best_score_obj: Option<ExtractionScore> = None;

    for (i, candidate) in candidates.iter().enumerate() {
        let score = score_candidate(
            &candidate.content,
            &candidate.strategy,
            intent,
            page_type,
        );
        if score.total > best_score {
            best_score = score.total;
            best_idx = i;
            best_score_obj = Some(score);
        }
    }

    let winner = &candidates[best_idx];

    // Log what was picked and why (only if non-Auto was chosen)
    if !matches!(winner.strategy, Extraction::Auto) {
        let strategy_label = extraction_label(&winner.strategy);
        let auto_score = candidates.iter()
            .find(|c| matches!(c.strategy, Extraction::Auto))
            .map(|c| score_candidate(&c.content, &c.strategy, intent, page_type).total);

        if let Some(auto_s) = auto_score {
            eprintln!(
                "  Extraction: {} (scored {:.2} — readability scored {:.2})",
                strategy_label, best_score, auto_s
            );
        } else {
            eprintln!(
                "  Extraction: {} (scored {:.2})",
                strategy_label, best_score
            );
        }
    } else if let Some(ref score_obj) = best_score_obj {
        // Even for Auto, log the score if it's low
        if score_obj.total < 0.5 {
            eprintln!(
                "  Warning: extraction scored low ({:.2}) — content may not match intent",
                score_obj.total
            );
        }
    }

    (winner.strategy.clone(), winner.content.clone())
}

/// Human-readable label for an extraction strategy
fn extraction_label(extraction: &Extraction) -> &'static str {
    match extraction {
        Extraction::Auto => "auto",
        Extraction::Full => "full",
        Extraction::JsonLd { .. } => "json-ld",
        Extraction::Selector { .. } => "selector",
        Extraction::Meta { .. } => "meta",
        Extraction::Rss => "rss",
    }
}

/// Compute specificity: ratio of intent-relevant tokens to total tokens
fn compute_specificity(content: &str, intent: Intent) -> f32 {
    let words: Vec<&str> = content.split_whitespace().collect();
    if words.is_empty() {
        return 0.0;
    }

    let relevant_count = match intent {
        Intent::Price => {
            let price_terms = ["$", "€", "£", "¥", "price", "cost", "usd", "eur", "gbp"];
            words.iter().filter(|w| {
                let lower = w.to_lowercase();
                price_terms.iter().any(|t| lower.contains(t))
                    || w.chars().any(|c| c.is_ascii_digit())
                        && w.contains('.')
            }).count()
        }
        Intent::Release => {
            let release_terms = ["version", "release", "changelog", "update", "v1", "v2", "v3", "v4", "v5"];
            words.iter().filter(|w| {
                let lower = w.to_lowercase();
                release_terms.iter().any(|t| lower.contains(t))
                    || (lower.starts_with('v') && lower.len() > 1 && lower.chars().nth(1).map(|c| c.is_ascii_digit()).unwrap_or(false))
            }).count()
        }
        Intent::Stock => {
            let stock_terms = ["stock", "available", "availability", "sold", "cart", "buy", "notify", "restock"];
            words.iter().filter(|w| {
                let lower = w.to_lowercase();
                stock_terms.iter().any(|t| lower.contains(t))
            }).count()
        }
        Intent::Jobs => {
            let job_terms = ["job", "position", "apply", "hiring", "career", "salary", "engineer", "developer"];
            words.iter().filter(|w| {
                let lower = w.to_lowercase();
                job_terms.iter().any(|t| lower.contains(t))
            }).count()
        }
        Intent::News | Intent::Generic => {
            // Generic: any meaningful content is relevant
            return 0.5;
        }
    };

    let ratio = relevant_count as f32 / words.len() as f32;
    // Scale: even 5% relevant tokens is a decent signal
    (ratio * 10.0).min(1.0)
}

/// Compute focus score based on content length.
/// Sweet spot: 200–5000 chars gets 1.0. Too short or too long is penalized.
fn compute_focus(len: usize) -> f32 {
    if len < 50 {
        0.1
    } else if len < 100 {
        0.3
    } else if len < 200 {
        0.6
    } else if len <= 5000 {
        1.0
    } else if len <= 10000 {
        0.8
    } else {
        0.5
    }
}

/// Check if content looks like boilerplate (sidebar, "About" section, Wikipedia text).
/// This catches cases like Google Finance picking the "About USD" sidebar.
fn is_boilerplate(content: &str) -> bool {
    let trimmed = content.trim();
    if trimmed.is_empty() {
        return true;
    }

    let lower = trimmed.to_lowercase();

    // Check for "About" section pattern
    let about_patterns = [
        "about ", "from wikipedia", "description:", "overview:",
        "this article", "the united states dollar",
    ];
    let about_hits = about_patterns.iter().filter(|p| lower.starts_with(*p)).count();
    if about_hits > 0 && trimmed.len() < 2000 {
        // Looks like an encyclopedia/about sidebar
        return true;
    }

    // Check for high ratio of long words (encyclopedia-like text) with no numbers/prices
    let words: Vec<&str> = trimmed.split_whitespace().collect();
    if words.len() > 20 {
        let has_numbers = words.iter().any(|w| w.chars().any(|c| c.is_ascii_digit()));
        let has_prices = lower.contains('$') || lower.contains('€') || lower.contains('£');
        let long_words = words.iter().filter(|w| w.len() > 8).count();
        let long_ratio = long_words as f32 / words.len() as f32;

        // Encyclopedia-like text: lots of long words, no numbers or prices
        if !has_numbers && !has_prices && long_ratio > 0.3 {
            return true;
        }
    }

    false
}

/// Apply page-type-based scoring adjustments.
/// Returns a positive or negative adjustment to the total score.
fn page_type_adjustment(strategy: &Extraction, page_type: PageType) -> f32 {
    match (page_type, strategy) {
        // Finance/Dashboard pages: readability misses tabular data
        (PageType::FinanceData | PageType::Dashboard, Extraction::Auto) => -0.15,
        (PageType::FinanceData | PageType::Dashboard, Extraction::Full) => 0.10,

        // E-commerce: JSON-LD has structured pricing data
        (PageType::ECommerce, Extraction::JsonLd { .. }) => 0.10,
        // E-commerce: readability may miss price elements
        (PageType::ECommerce, Extraction::Auto) => -0.05,

        // Articles: readability is designed for this
        (PageType::Article, Extraction::Auto) => 0.05,
        (PageType::Article, Extraction::Full) => -0.05,

        _ => 0.0,
    }
}

/// Get the page title from HTML
pub fn extract_title(html: &str) -> Option<String> {
    let document = Html::parse_document(html);
    let title_selector = Selector::parse("title").ok()?;
    document
        .select(&title_selector)
        .next()
        .map(|el| el.text().collect::<Vec<_>>().join(""))
}

/// Extract raw JSON-LD data from HTML as a string (for research context)
/// Returns the raw JSON-LD content, not formatted for display
pub fn extract_raw_jsonld(html: &str) -> Option<String> {
    let document = Html::parse_document(html);
    let jsonld_selector = Selector::parse(r#"script[type="application/ld+json"]"#).ok()?;

    let mut all_jsonld = Vec::new();

    for script in document.select(&jsonld_selector) {
        let text: String = script.text().collect();
        if let Ok(json) = serde_json::from_str::<serde_json::Value>(&text) {
            all_jsonld.push(json);
        }
    }

    if all_jsonld.is_empty() {
        return None;
    }

    // Return as formatted JSON string
    if all_jsonld.len() == 1 {
        serde_json::to_string_pretty(&all_jsonld[0]).ok()
    } else {
        serde_json::to_string_pretty(&all_jsonld).ok()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_extract_selector() {
        let html = r#"<html><body><div class="content">Hello World</div></body></html>"#;
        let result = extract_selector(html, ".content").unwrap();
        assert_eq!(result.trim(), "Hello World");
    }

    #[test]
    fn test_extract_meta() {
        let html = r#"
            <html>
            <head>
                <meta name="description" content="Test description">
                <meta property="og:title" content="OG Title">
            </head>
            <body></body>
            </html>
        "#;
        let result = extract_meta(html, &["description".into(), "og:title".into()]).unwrap();
        assert!(result.contains("Test description"));
        assert!(result.contains("OG Title"));
    }

    #[test]
    fn test_extract_title() {
        let html = r#"<html><head><title>My Page Title</title></head><body></body></html>"#;
        let title = extract_title(html);
        assert_eq!(title, Some("My Page Title".to_string()));
    }

    // JSON-LD extraction tests

    #[test]
    fn test_jsonld_product_single_offer() {
        let html = r#"
            <html><head>
            <script type="application/ld+json">
            {
                "@type": "Product",
                "name": "Test Product",
                "offers": {
                    "price": "99.99",
                    "priceCurrency": "USD",
                    "availability": "https://schema.org/InStock"
                }
            }
            </script>
            </head><body></body></html>
        "#;
        let result = extract_jsonld(html, None).unwrap();
        assert!(result.contains("[Product]"));
        assert!(result.contains("Test Product"));
        assert!(result.contains("99.99 USD"));
        assert!(result.contains("InStock"));
    }

    #[test]
    fn test_jsonld_product_multiple_offers() {
        let html = r#"
            <html><head>
            <script type="application/ld+json">
            {
                "@type": "Product",
                "name": "Multi-Seller Product",
                "offers": [
                    {"seller": {"name": "Seller A"}, "price": "50.00", "priceCurrency": "USD"},
                    {"seller": {"name": "Seller B"}, "price": "55.00", "priceCurrency": "USD"},
                    {"seller": {"name": "Seller C"}, "price": "52.00", "priceCurrency": "USD"}
                ]
            }
            </script>
            </head><body></body></html>
        "#;
        let result = extract_jsonld(html, None).unwrap();
        assert!(result.contains("Offer 1:"));
        assert!(result.contains("Offer 2:"));
        assert!(result.contains("Offer 3:"));
        assert!(result.contains("Seller A"));
        assert!(result.contains("Seller B"));
        assert!(result.contains("50.00 USD"));
    }

    #[test]
    fn test_jsonld_namespaced_type() {
        let html = r#"
            <html><head>
            <script type="application/ld+json">
            {
                "@type": "schema:Product",
                "name": "Namespaced Product"
            }
            </script>
            </head><body></body></html>
        "#;
        // Should match "Product" filter even with "schema:" prefix
        let result = extract_jsonld(html, Some(&vec!["Product".to_string()])).unwrap();
        assert!(result.contains("Namespaced Product"));
    }

    #[test]
    fn test_jsonld_type_filter() {
        let html = r#"
            <html><head>
            <script type="application/ld+json">
            [
                {"@type": "Product", "name": "Product Item"},
                {"@type": "Organization", "name": "Org Item"}
            ]
            </script>
            </head><body></body></html>
        "#;
        // Filter to only Product
        let result = extract_jsonld(html, Some(&vec!["Product".to_string()])).unwrap();
        assert!(result.contains("Product Item"));
        assert!(!result.contains("Org Item"));
    }

    #[test]
    fn test_jsonld_graph_structure() {
        let html = r#"
            <html><head>
            <script type="application/ld+json">
            {
                "@graph": [
                    {"@type": "Article", "headline": "Test Article"},
                    {"@type": "Organization", "name": "Test Org"}
                ]
            }
            </script>
            </head><body></body></html>
        "#;
        let result = extract_jsonld(html, None).unwrap();
        assert!(result.contains("Test Article"));
        assert!(result.contains("Test Org"));
    }

    #[test]
    fn test_jsonld_article() {
        let html = r#"
            <html><head>
            <script type="application/ld+json">
            {
                "@type": "NewsArticle",
                "headline": "Breaking News",
                "author": {"name": "John Doe"},
                "datePublished": "2024-01-15"
            }
            </script>
            </head><body></body></html>
        "#;
        let result = extract_jsonld(html, None).unwrap();
        assert!(result.contains("Breaking News"));
        assert!(result.contains("John Doe"));
        assert!(result.contains("2024-01-15"));
    }

    #[test]
    fn test_jsonld_event() {
        let html = r#"
            <html><head>
            <script type="application/ld+json">
            {
                "@type": "Event",
                "name": "Concert",
                "startDate": "2024-06-01",
                "location": {"name": "Arena"}
            }
            </script>
            </head><body></body></html>
        "#;
        let result = extract_jsonld(html, None).unwrap();
        assert!(result.contains("Concert"));
        assert!(result.contains("2024-06-01"));
        assert!(result.contains("Arena"));
    }

    #[test]
    fn test_jsonld_no_data() {
        let html = r#"<html><head></head><body></body></html>"#;
        let result = extract_jsonld(html, None);
        assert!(result.is_err());
    }

    #[test]
    fn test_type_matches_filter_basic() {
        assert!(type_matches_filter("Product", &["Product".to_string()]));
        assert!(type_matches_filter("product", &["Product".to_string()])); // case insensitive
        assert!(!type_matches_filter("Article", &["Product".to_string()]));
    }

    #[test]
    fn test_type_matches_filter_namespaced() {
        assert!(type_matches_filter("schema:Product", &["Product".to_string()]));
        assert!(type_matches_filter("http://schema.org/Product", &["Product".to_string()]));
    }

    #[test]
    fn test_type_matches_filter_partial() {
        // "NewsArticle" should match "Article" filter (partial match)
        assert!(type_matches_filter("NewsArticle", &["Article".to_string()]));
        assert!(type_matches_filter("BlogPosting", &["Blog".to_string()]));
    }

    #[test]
    fn test_offers_limit() {
        // Test that more than 5 offers shows "... and N more offers"
        let html = r#"
            <html><head>
            <script type="application/ld+json">
            {
                "@type": "Product",
                "name": "Many Offers Product",
                "offers": [
                    {"price": "10", "priceCurrency": "USD"},
                    {"price": "11", "priceCurrency": "USD"},
                    {"price": "12", "priceCurrency": "USD"},
                    {"price": "13", "priceCurrency": "USD"},
                    {"price": "14", "priceCurrency": "USD"},
                    {"price": "15", "priceCurrency": "USD"},
                    {"price": "16", "priceCurrency": "USD"}
                ]
            }
            </script>
            </head><body></body></html>
        "#;
        let result = extract_jsonld(html, None).unwrap();
        assert!(result.contains("Offer 5:"));
        assert!(result.contains("... and 2 more offers"));
        // Should NOT have Offer 6 or 7
        assert!(!result.contains("Offer 6:"));
    }

    #[test]
    fn test_extract_raw_jsonld() {
        let html = r#"
            <html><head>
            <script type="application/ld+json">
            {"@type": "Product", "name": "Test Product", "sku": "12345"}
            </script>
            </head><body></body></html>
        "#;
        let result = extract_raw_jsonld(html).unwrap();
        assert!(result.contains("\"@type\": \"Product\""));
        assert!(result.contains("\"name\": \"Test Product\""));
        assert!(result.contains("\"sku\": \"12345\""));
    }

    #[test]
    fn test_extract_raw_jsonld_none() {
        let html = r#"<html><head></head><body></body></html>"#;
        let result = extract_raw_jsonld(html);
        assert!(result.is_none());
    }

    // Multi-strategy extraction scoring tests

    #[test]
    fn test_detect_page_type_finance() {
        let html = r#"<html><body>
            <div class="ticker">ETH/USD</div>
            <span>Exchange: Coinbase</span>
            <span>Market Cap: $200B</span>
        </body></html>"#;
        assert_eq!(detect_page_type(html), PageType::FinanceData);
    }

    #[test]
    fn test_detect_page_type_ecommerce() {
        let html = r#"<html><body>
            <div class="product-detail">
                <button>Add to Cart</button>
                <span>$99.99</span>
            </div>
        </body></html>"#;
        assert_eq!(detect_page_type(html), PageType::ECommerce);
    }

    #[test]
    fn test_detect_page_type_article() {
        let html = r#"<html><body>
            <article>
                <h1>Breaking News</h1>
                <p>Long paragraph text here...</p>
            </article>
        </body></html>"#;
        assert_eq!(detect_page_type(html), PageType::Article);
    }

    #[test]
    fn test_detect_page_type_generic() {
        let html = r#"<html><body><div>Hello World</div></body></html>"#;
        assert_eq!(detect_page_type(html), PageType::Generic);
    }

    #[test]
    fn test_is_boilerplate_about_text() {
        assert!(is_boilerplate("About the United States Dollar. The dollar is the currency of the United States."));
        assert!(is_boilerplate("From Wikipedia, the free encyclopedia. This article discusses..."));
    }

    #[test]
    fn test_is_not_boilerplate_price_data() {
        assert!(!is_boilerplate("ETH/USD $2,345.67 +5.2% Market Cap: $280B Volume: $12.5B"));
        assert!(!is_boilerplate("Nike Air Max 90 - $149.99 - In Stock - Free Shipping"));
    }

    #[test]
    fn test_compute_focus() {
        assert!(compute_focus(10) < 0.3);     // too short
        assert!(compute_focus(500) > 0.9);     // sweet spot
        assert!(compute_focus(3000) > 0.9);    // sweet spot
        assert!(compute_focus(15000) < 0.7);   // too long
    }

    #[test]
    fn test_score_candidate_price_intent() {
        let price_content = "ETH/USD $2,345.67 +5.2% 24h Volume: $12.5B Market Cap: $280B";
        let generic_content = "About the United States Dollar. The dollar is the official currency of the United States and several other countries. It is divided into 100 smaller units called cents.";

        let price_score = score_candidate(price_content, &Extraction::Full, Intent::Price, PageType::FinanceData);
        let generic_score = score_candidate(generic_content, &Extraction::Auto, Intent::Price, PageType::FinanceData);

        assert!(price_score.total > generic_score.total,
            "Price content ({:.2}) should score higher than generic sidebar ({:.2}) for Price intent",
            price_score.total, generic_score.total);
    }

    #[test]
    fn test_score_candidate_ecommerce_jsonld() {
        let jsonld_content = "[Product]\nName: Nike Air Max\nPrice: 149.99 USD\nAvailability: InStock";
        let auto_content = "Nike Air Max 90. The Nike Air Max 90 stays true to its OG running roots.";

        let jsonld_score = score_candidate(jsonld_content, &Extraction::JsonLd { types: None }, Intent::Price, PageType::ECommerce);
        let auto_score = score_candidate(auto_content, &Extraction::Auto, Intent::Price, PageType::ECommerce);

        assert!(jsonld_score.total > auto_score.total,
            "JSON-LD ({:.2}) should score higher than Auto ({:.2}) for Price intent on e-commerce",
            jsonld_score.total, auto_score.total);
    }

    #[test]
    fn test_extract_all_strategies() {
        let html = r#"
            <html><head>
            <script type="application/ld+json">
            {"@type": "Product", "name": "Test", "offers": {"price": "99.99", "priceCurrency": "USD"}}
            </script>
            </head>
            <body>
                <article><p>This is a product page with lots of content about the product and its features.</p></article>
                <div class="sidebar">About this product category</div>
            </body></html>
        "#;
        let content = PageContent {
            url: "https://example.com".to_string(),
            title: None,
            html: html.to_string(),
            text: None,
        };

        let candidates = extract_all_strategies(&content);
        // Should have at least Auto and JsonLd (Full depends on body content)
        assert!(candidates.len() >= 2, "Expected at least 2 candidates, got {}", candidates.len());

        // Check that we got different strategies
        let strategies: Vec<_> = candidates.iter().map(|c| extraction_label(&c.strategy)).collect();
        assert!(strategies.contains(&"json-ld"), "Should include json-ld strategy");
    }

    #[test]
    fn test_page_type_adjustment() {
        // Finance + Auto should be penalized
        assert!(page_type_adjustment(&Extraction::Auto, PageType::FinanceData) < 0.0);
        // Finance + Full should get a bonus
        assert!(page_type_adjustment(&Extraction::Full, PageType::FinanceData) > 0.0);
        // E-commerce + JsonLd should get a bonus
        assert!(page_type_adjustment(&Extraction::JsonLd { types: None }, PageType::ECommerce) > 0.0);
        // Article + Auto should get a bonus
        assert!(page_type_adjustment(&Extraction::Auto, PageType::Article) > 0.0);
    }
}
