use anyhow::{Context, Result};
use cookie_scoop::{
    BrowserName, CookieHeaderOptions, GetCookiesOptions, to_cookie_header,
};

fn domain_to_url(domain: &str) -> String {
    let d = domain.trim_start_matches('.');
    if d.starts_with("http") {
        d.to_string()
    } else {
        format!("https://{d}")
    }
}

fn origins_for_domain(domain: &str) -> Vec<String> {
    let bare = domain.trim_start_matches('.');
    let dotted = format!(".{bare}");
    vec![
        domain_to_url(bare),
        domain_to_url(&dotted),
    ]
}

pub async fn find_cookie(domain: &str, cookie_name: &str) -> Result<String> {
    let result = cookie_scoop::get_cookies(
        GetCookiesOptions::new(domain_to_url(domain))
            .origins(origins_for_domain(domain))
            .names(vec![cookie_name.to_string()])
            .browsers(vec![BrowserName::Chrome, BrowserName::Edge, BrowserName::Firefox])
    ).await;

    for w in &result.warnings {
        tracing::debug!("cookie-scoop: {w}");
    }

    result
        .cookies
        .into_iter()
        .find(|c| c.name == cookie_name)
        .map(|c| c.value)
        .filter(|v| !v.is_empty())
        .with_context(|| format!("Cookie '{cookie_name}' not found for {domain}"))
}

pub async fn find_cookie_header(domains: &[&str]) -> Result<String> {
    let url = domain_to_url(domains[0]);
    let origins: Vec<String> = domains.iter().flat_map(|d| origins_for_domain(d)).collect();

    let result = cookie_scoop::get_cookies(
        GetCookiesOptions::new(&url)
            .origins(origins)
            .browsers(vec![BrowserName::Chrome, BrowserName::Edge, BrowserName::Firefox])
    ).await;

    for w in &result.warnings {
        tracing::debug!("cookie-scoop: {w}");
    }

    if result.cookies.is_empty() {
        anyhow::bail!(
            "No cookies found for {}",
            domains.join(", ")
        );
    }

    let header = to_cookie_header(&result.cookies, &CookieHeaderOptions::default());
    if header.is_empty() {
        anyhow::bail!(
            "Cookie header empty for {}",
            domains.join(", ")
        );
    }

    Ok(header)
}

pub async fn find_cookie_multiple(domains: &[&str], cookie_name: &str) -> Result<String> {
    for domain in domains {
        if let Ok(cookie) = find_cookie(domain, cookie_name).await {
            return Ok(cookie);
        }
    }
    anyhow::bail!(
        "Cookie '{}' not found in any of: {}",
        cookie_name,
        domains.join(", ")
    )
}
