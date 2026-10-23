// SPDX-License-Identifier: GPL-3.0-only
// Copyright (c) 2026 LibreTypist contributors
//! Exact host matching only. Paths, page titles and browsing history are not retained.
use url::Url;

pub fn canonical_host(input: &str) -> Result<String, String> {
    let input = input.trim();
    if input.is_empty()
        || input.len() > 253
        || input
            .chars()
            .any(|c| c.is_whitespace() || c.is_control() || "/?#@\\*%".contains(c))
    {
        return Err(
            "Enter an exact domain, such as mail.google.com, without a URL, path or wildcard."
                .into(),
        );
    }
    if input.contains(':') && !(input.starts_with('[') && input.ends_with(']')) {
        return Err("Enter the domain without a port or login details.".into());
    }
    let url = Url::parse(&format!("https://{input}"))
        .map_err(|_| "Enter a valid website domain.".to_owned())?;
    if url.port().is_some() || !url.username().is_empty() || url.password().is_some() {
        return Err("Enter the domain without a port or login details.".into());
    }
    let host = url
        .host_str()
        .ok_or("Enter a valid website domain.")?
        .trim_end_matches('.')
        .to_owned();
    if host.is_empty() || host.contains("..") || host.starts_with('.') {
        return Err("Enter a valid website domain.".into());
    }
    Ok(host)
}

pub fn host_from_url(input: &str) -> Option<String> {
    if input.chars().any(char::is_control) {
        return None;
    }
    let url = Url::parse(input).ok()?;
    if !matches!(url.scheme(), "http" | "https")
        || !url.username().is_empty()
        || url.password().is_some()
    {
        return None;
    }
    canonical_host(url.host_str()?).ok()
}

pub fn is_browser(app: &str) -> bool {
    matches!(
        app,
        "com.google.Chrome"
            | "com.google.Chrome.canary"
            | "com.apple.Safari"
            | "com.apple.SafariTechnologyPreview"
            | "org.mozilla.firefox"
            | "com.microsoft.edgemac"
            | "com.brave.Browser"
            | "company.thebrowser.Browser"
            | "company.thebrowser.dia"
            | "com.vivaldi.Vivaldi"
            | "com.operasoftware.Opera"
            | "com.kagi.kagimacOS"
    )
}

/// Ancestors arrive from the focused frame outward. An unknown outer document
/// must replace a known inner document, rather than leaking an iframe's rule.
#[derive(Default)]
pub struct DocumentScope {
    seen: bool,
    host: Option<String>,
}
impl DocumentScope {
    pub fn observe(&mut self, url: Option<&str>) {
        self.seen = true;
        self.host = url.and_then(host_from_url);
    }
    pub fn finish(self) -> Result<Option<String>, &'static str> {
        if self.seen && self.host.is_none() {
            Err("Website identity unavailable")
        } else {
            Ok(self.host)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn outer_document_controls_scope_and_unknown_identity_never_reuses_inner_scope() {
        let mut scope = DocumentScope::default();
        scope.observe(Some("https://embedded.example/"));
        scope.observe(Some("https://mail.google.com/"));
        assert_eq!(scope.finish(), Ok(Some("mail.google.com".into())));
        let mut scope = DocumentScope::default();
        scope.observe(Some("https://mail.google.com/"));
        scope.observe(None);
        assert!(scope.finish().is_err());
        assert_eq!(DocumentScope::default().finish(), Ok(None));
        let mut scope = DocumentScope::default();
        scope.observe(Some("about:blank"));
        scope.observe(Some("https://parent.example/"));
        assert_eq!(scope.finish(), Ok(Some("parent.example".into())));
    }
    #[test]
    fn exact_domains_and_urls_have_unambiguous_hosts() {
        assert_eq!(
            canonical_host(" MAIL.Google.COM. ").unwrap(),
            "mail.google.com"
        );
        assert_eq!(
            canonical_host("bücher.example").unwrap(),
            "xn--bcher-kva.example"
        );
        assert_eq!(
            host_from_url("https://mail.google.com/mail/u/0/?secret=x#draft").as_deref(),
            Some("mail.google.com")
        );
        assert_eq!(
            host_from_url("http://127.0.0.1:8080/test").as_deref(),
            Some("127.0.0.1")
        );
        assert_eq!(
            host_from_url("https://mail.google.com.evil.example/").as_deref(),
            Some("mail.google.com.evil.example")
        );
        for input in [
            "",
            "https://example.com",
            "*.example.com",
            "example.com/path",
            "example.com?x",
            "example.com#x",
            "a@b.com",
            "example.com:8080",
            "example.com:443",
            "foo bar",
            "a..b",
            "%65xample.com",
        ] {
            assert!(canonical_host(input).is_err(), "{input}");
        }
        for input in [
            "about:blank",
            "file:///tmp/test.html",
            "chrome://settings",
            "https://a:b@example.com",
            "https://example.com/\nsecret",
        ] {
            assert!(host_from_url(input).is_none(), "{input}");
        }
    }
}
