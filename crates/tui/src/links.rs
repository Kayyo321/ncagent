/*
 * Ko, (Baczynski-Bruce) 2026; Apache 2.0
*/
//! Links in the transcript (PROJECT.md, D76, D77): which addresses are accepted, and when the
//! address stays visible after its label.
//!
//! A link's address comes from model output, which is untrusted, and it goes into an escape
//! sequence (OSC 8) and, once the user confirms, into the argument of the system URL opener. So
//! an address is accepted only when it is `http` or `https` (any case), at most
//! `MaxUrlBytes` bytes, and every byte is printable ASCII other than a space or a backslash.
//! Anything else, a non-ASCII character included, must be percent-encoded, or the link is
//! refused. A refused link renders as plain text with its address after it, as before.

/// The longest address accepted, in bytes (D77).
pub const MaxUrlBytes: usize = 2000;

/// The address when it may be a link, else `None` (see the module note).
pub fn safe_url(url: &str) -> Option<String> {
    if url.is_empty() || url.len() > MaxUrlBytes {
        return None;
    }
    if !url
        .bytes()
        .all(|b| (0x21..=0x7e).contains(&b) && b != b'\\')
    {
        return None;
    }
    let lower = url.to_ascii_lowercase();
    let rest = lower
        .strip_prefix("https://")
        .or_else(|| lower.strip_prefix("http://"))?;
    if host_of_rest(rest).is_empty() {
        return None;
    }
    Some(url.to_string())
}

/// The host of an `http` or `https` address, in lowercase: after the scheme, before the path,
/// the query, the fragment and the port, and after any `user@`. Empty when there is none.
pub fn host_of(url: &str) -> String {
    let lower = url.to_ascii_lowercase();
    match lower
        .strip_prefix("https://")
        .or_else(|| lower.strip_prefix("http://"))
    {
        Some(rest) => host_of_rest(rest),
        None => String::new(),
    }
}

fn host_of_rest(rest: &str) -> String {
    let authority = rest.split(['/', '?', '#']).next().unwrap_or("");
    let host = authority.rsplit('@').next().unwrap_or("");
    let host = if host.starts_with('[') {
        // An IPv6 literal keeps its colons.
        host.split(']')
            .next()
            .map(|h| format!("{h}]"))
            .unwrap_or_default()
    } else {
        host.split(':').next().unwrap_or("").to_string()
    };
    host.trim_end_matches('.').to_string()
}

/// True when a label looks like an address itself: it holds `http://`, `https://` or starts
/// with `www.`.
pub fn label_looks_like_url(label: &str) -> bool {
    let lower = label.trim().to_ascii_lowercase();
    lower.contains("http://") || lower.contains("https://") || lower.starts_with("www.")
}

/// The domain-like words of a label: dot-separated runs of letters, digits and `-` whose last
/// part is two or more letters, such as `example.com`, in lowercase.
pub fn domain_tokens(label: &str) -> Vec<String> {
    label
        .split(|c: char| !(c.is_ascii_alphanumeric() || c == '.' || c == '-'))
        .map(|w| w.trim_matches('.').to_ascii_lowercase())
        .filter(|w| {
            let parts: Vec<&str> = w.split('.').collect();
            parts.len() >= 2
                && parts.iter().all(|p| !p.is_empty())
                && parts.last().is_some_and(|tld| {
                    tld.len() >= 2 && tld.chars().all(|c| c.is_ascii_alphabetic())
                })
        })
        .collect()
}

/// True when the label names a host other than the address's: a domain-like word that is
/// neither the host nor a parent of it (`example.com` for `docs.example.com` is fine).
pub fn deceptive_label(label: &str, url: &str) -> bool {
    let host = host_of(url);
    domain_tokens(label)
        .iter()
        .any(|t| *t != host && !host.ends_with(&format!(".{t}")))
}

/// Whether the address is shown after a link's label (D77): when the link is refused, when the
/// label looks like an address or names another host, or when the user asked to see every
/// address. Never when the label is the address itself.
pub fn show_suffix(label: &str, url: &str, accepted: bool, always: bool) -> bool {
    if url.is_empty() || label.trim() == url.trim() {
        return false;
    }
    !accepted || always || label_looks_like_url(label) || deceptive_label(label, url)
}

/// `text` cut in the middle to at most `max` columns, with `…` where it was cut, so both the
/// host at the start and the end of the path stay visible.
pub fn middle_truncate(text: &str, max: usize) -> String {
    let chars: Vec<char> = text.chars().collect();
    if chars.len() <= max {
        return text.to_string();
    }
    if max <= 1 {
        return "…".chars().take(max).collect();
    }
    let keep = max - 1;
    let head = keep.div_ceil(2);
    let tail = keep - head;
    let mut out: String = chars.iter().take(head).collect();
    out.push('…');
    out.extend(chars.iter().skip(chars.len() - tail));
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_clean_http_and_https_addresses_are_accepted() {
        for good in [
            "https://arxiv.org/abs/2610.14162",
            "HTTP://Example.COM",
            "https://example.com/a%20b?q=%E2%9C%93#frag",
            "https://user@host.example:8080/x",
            "https://[::1]:8080/",
        ] {
            assert_eq!(safe_url(good).as_deref(), Some(good), "{good}");
        }
        let long = format!("https://example.com/{}", "a".repeat(MaxUrlBytes));
        for bad in [
            "",
            "https://exa\u{1b}mple.com",
            "https://example.com/\u{7}",
            "https://example.com/\u{1b}\\",
            "https://example.com/\u{1b}]8;;evil\u{1b}\\",
            "https://example.com/a\nb",
            "https://example.com/a\rb",
            "https://example.com/a b",
            "https://example.com/a\tb",
            "https://example.com/\u{7f}",
            "https://example.com/\\x",
            "https://exämple.com/",
            "https://example.com/\u{9c}",
            "javascript:alert(1)",
            "JavaScript:alert(1)",
            "file:///etc/passwd",
            "data:text/html,x",
            "ftp://example.com",
            "//example.com",
            "https://",
            "https:///path",
            long.as_str(),
        ] {
            assert_eq!(safe_url(bad), None, "{bad:?}");
        }
        assert!(safe_url(&format!("https://e.co/{}", "a".repeat(MaxUrlBytes - 13))).is_some());
    }

    #[test]
    fn hosts_skip_user_info_and_ports() {
        assert_eq!(host_of("https://Arxiv.org/abs/1"), "arxiv.org");
        assert_eq!(host_of("https://google.com@evil.example/x"), "evil.example");
        assert_eq!(host_of("http://host:80?x"), "host");
        assert_eq!(host_of("https://[::1]:8080/"), "[::1]");
        assert_eq!(host_of("ftp://x"), "");
    }

    #[test]
    fn the_suffix_stays_for_refused_lookalike_and_deceptive_labels() {
        let url = "https://arxiv.org/abs/2610.14162";
        assert!(
            !show_suffix("NCM paper", url, true, false),
            "a plain label hides it"
        );
        assert!(
            show_suffix("NCM paper", url, true, true),
            "the user asked for it"
        );
        assert!(
            show_suffix("NCM paper", url, false, false),
            "a refused link keeps it"
        );
        assert!(
            !show_suffix(url, url, true, false),
            "the label is the address"
        );
        assert!(!show_suffix("arxiv.org", url, true, false), "the real host");
        assert!(!show_suffix("the arXiv page", url, true, false));
        assert!(!show_suffix(
            "export.arxiv.org",
            "https://export.arxiv.org/x",
            true,
            false
        ));
        assert!(
            !show_suffix("arxiv.org", "https://www.arxiv.org/x", true, false),
            "a parent"
        );
        for lie in [
            "https://google.com",
            "http://arxiv.org/abs/1",
            "www.arxiv.org",
            "google.com",
            "see paypal.com now",
            "arxiv.org.evil.example",
        ] {
            assert!(show_suffix(lie, url, true, false), "{lie}");
        }
        assert!(
            show_suffix("arxiv.org", "https://arxiv.org.evil.example/", true, false),
            "a host that only starts like the label"
        );
        assert_eq!(
            domain_tokens("e.g. v1.2 and docs.rs"),
            vec!["docs.rs".to_string()]
        );
    }

    #[test]
    fn middle_truncation_keeps_both_ends() {
        assert_eq!(middle_truncate("abcdef", 10), "abcdef");
        assert_eq!(middle_truncate("abcdefghij", 5), "ab…ij");
        assert_eq!(middle_truncate("abcdefghij", 6), "abc…ij");
        assert_eq!(middle_truncate("abc", 1), "…");
        assert_eq!(middle_truncate("abc", 0), "");
    }
}
