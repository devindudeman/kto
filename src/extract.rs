use scraper::{Html, Selector};

use crate::error::{KtoError, Result};
use crate::fetch::PageContent;
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
}
