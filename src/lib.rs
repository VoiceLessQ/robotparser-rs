//! A faithful Rust port of Python's [`urllib.robotparser`](https://docs.python.org/3/library/urllib.robotparser.html).
//!
//! Port target: the standard library `urllib/robotparser.py` (CPython 3.13). Behaviour is verified
//! against the reference module itself via differential testing.
//!
//! Unlike the existing `robotparser` crate (which follows the 1996 `norobots-rfc` draft), this
//! mirrors what CPython actually does today: a **prefix-match, first-match** algorithm (no `*`/`$`
//! wildcards, no longest-match precedence), with CPython's exact path normalization, parse state
//! machine, and user-agent substring matching.
//!
//! The network half of `RobotFileParser` (`read()` / `urlopen`) is out of scope — fetch the bytes
//! yourself and feed the lines to [`RobotFileParser::parse`], exactly as CPython's `read()` does.
//!
//! ```
//! use robotparser_rs::RobotFileParser;
//!
//! let mut rp = RobotFileParser::new("http://example.com/robots.txt");
//! rp.parse(&["User-agent: *", "Disallow: /private", "Allow: /private/ok"]);
//! assert_eq!(rp.can_fetch("MyBot", "http://example.com/public"), Ok(true));
//! assert_eq!(rp.can_fetch("MyBot", "http://example.com/private/x"), Ok(false));
//! ```

use std::fmt;

use urlparse_rs::{quote_from_bytes, unquote_to_bytes, urlsplit, urlunsplit, UrlError};

/// A `Request-rate` directive: `requests` per `seconds`. Port of the `RequestRate` namedtuple.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RequestRate {
    pub requests: u64,
    pub seconds: u64,
}

/// `quote(unquote(path, surrogateescape), surrogateescape)` — canonicalize percent-encoding. Done
/// on the raw decoded bytes (equivalent to CPython's surrogateescape round-trip), so any byte
/// sequence is preserved exactly. Port of `normalize`.
fn normalize(path: &str) -> String {
    quote_from_bytes(&unquote_to_bytes(path), "/")
}

/// Normalize the path, and within a `?query` normalize each maximal run of non-`=&` characters
/// (leaving the `=`/`&` separators). Port of `normalize_path`.
fn normalize_path(path: &str) -> String {
    match path.split_once('?') {
        Some((path, query)) => {
            let mut out = normalize(path);
            out.push('?');
            // Port of `re.sub(r'[^=&]+', lambda m: normalize(m[0]), query)`.
            let mut run = String::new();
            for ch in query.chars() {
                if ch == '=' || ch == '&' {
                    if !run.is_empty() {
                        out.push_str(&normalize(&run));
                        run.clear();
                    }
                    out.push(ch);
                } else {
                    run.push(ch);
                }
            }
            if !run.is_empty() {
                out.push_str(&normalize(&run));
            }
            out
        }
        None => normalize(path),
    }
}

/// A single `Allow:`/`Disallow:` rule. Port of `RuleLine`.
#[derive(Debug, Clone)]
struct RuleLine {
    path: String,
    allowance: bool,
}

impl RuleLine {
    fn new(path: &str, allowance: bool) -> Self {
        // An empty `Disallow:` value means allow all.
        let allowance = if path.is_empty() && !allowance { true } else { allowance };
        RuleLine { path: normalize_path(path), allowance }
    }

    fn applies_to(&self, filename: &str) -> bool {
        self.path == "*" || filename.starts_with(&self.path)
    }
}

impl fmt::Display for RuleLine {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}: {}", if self.allowance { "Allow" } else { "Disallow" }, self.path)
    }
}

/// One or more user-agents and their rules/delays. Port of `Entry`.
#[derive(Debug, Clone, Default)]
struct Entry {
    useragents: Vec<String>,
    rulelines: Vec<RuleLine>,
    delay: Option<i64>,
    req_rate: Option<RequestRate>,
}

impl Entry {
    /// Port of `Entry.applies_to`: the agent token (before the first `/`, lowercased) contains one
    /// of this entry's agents as a substring, or the entry has the catch-all `*`.
    fn applies_to(&self, useragent: &str) -> bool {
        let useragent = useragent.split('/').next().unwrap_or("").to_lowercase();
        for agent in &self.useragents {
            if agent == "*" {
                return true;
            }
            if useragent.contains(&agent.to_lowercase()) {
                return true;
            }
        }
        false
    }

    /// Port of `Entry.allowance`: the first rule whose path matches decides; default allow.
    fn allowance(&self, filename: &str) -> bool {
        for line in &self.rulelines {
            if line.applies_to(filename) {
                return line.allowance;
            }
        }
        true
    }
}

impl fmt::Display for Entry {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut parts: Vec<String> = Vec::new();
        for agent in &self.useragents {
            parts.push(format!("User-agent: {agent}"));
        }
        if let Some(delay) = self.delay {
            parts.push(format!("Crawl-delay: {delay}"));
        }
        if let Some(rate) = self.req_rate {
            parts.push(format!("Request-rate: {}/{}", rate.requests, rate.seconds));
        }
        for line in &self.rulelines {
            parts.push(line.to_string());
        }
        f.write_str(&parts.join("\n"))
    }
}

/// Reads, parses and answers questions about a single robots.txt file. Port of `RobotFileParser`.
#[derive(Debug, Clone, Default)]
pub struct RobotFileParser {
    entries: Vec<Entry>,
    sitemaps: Vec<String>,
    default_entry: Option<Entry>,
    disallow_all: bool,
    allow_all: bool,
    url: String,
    host: String,
    path: String,
    // CPython tracks `last_checked` as a timestamp and only ever tests its truthiness; we model
    // that as "has parse()/modified() been called?".
    last_checked: bool,
}

impl RobotFileParser {
    /// Create a parser for the robots.txt at `url` (pass `""` for none). Port of `__init__`.
    pub fn new(url: &str) -> Self {
        let mut rp = RobotFileParser::default();
        rp.set_url(url);
        rp
    }

    /// Whether the file has been parsed yet. Port of `mtime` (truthiness only — no real clock).
    pub fn mtime(&self) -> bool {
        self.last_checked
    }

    /// Mark the file as just parsed. Port of `modified`.
    pub fn modified(&mut self) {
        self.last_checked = true;
    }

    /// Set the robots.txt URL, caching its host and path. Port of `set_url`. A malformed authority
    /// leaves host/path empty rather than propagating (CPython would raise here).
    pub fn set_url(&mut self, url: &str) {
        self.url = url.to_string();
        if let Ok(s) = urlsplit(url, "", true) {
            self.host = s.netloc;
            self.path = s.path;
        } else {
            self.host = String::new();
            self.path = String::new();
        }
    }

    /// The robots.txt URL set via [`new`](Self::new) / [`set_url`](Self::set_url).
    pub fn url(&self) -> &str {
        &self.url
    }

    /// The host of the robots.txt URL.
    pub fn host(&self) -> &str {
        &self.host
    }

    /// The path of the robots.txt URL.
    pub fn path(&self) -> &str {
        &self.path
    }

    fn add_entry(&mut self, entry: Entry) {
        if entry.useragents.iter().any(|a| a == "*") {
            // The first catch-all entry becomes the default and is considered last.
            if self.default_entry.is_none() {
                self.default_entry = Some(entry);
            }
        } else {
            self.entries.push(entry);
        }
    }

    /// Parse the lines of a robots.txt file (already split into lines, no trailing newlines). Port
    /// of `parse`.
    pub fn parse<S: AsRef<str>>(&mut self, lines: &[S]) {
        // states: 0 start, 1 saw user-agent, 2 saw allow/disallow
        let mut state = 0u8;
        let mut entry = Entry::default();
        self.modified();
        for raw in lines {
            let raw = raw.as_ref();
            if raw.is_empty() {
                match state {
                    1 => {
                        entry = Entry::default();
                        state = 0;
                    }
                    2 => {
                        self.add_entry(std::mem::take(&mut entry));
                        state = 0;
                    }
                    _ => {}
                }
            }
            // strip an optional comment, then surrounding whitespace
            let line = match raw.find('#') {
                Some(i) => &raw[..i],
                None => raw,
            };
            let line = line.trim();
            if line.is_empty() {
                continue;
            }
            let (key, value) = match line.split_once(':') {
                Some((k, v)) => (k.trim().to_lowercase(), v.trim()),
                None => continue, // no ':' — CPython's len(line)==2 guard skips it
            };
            match key.as_str() {
                "user-agent" => {
                    if state == 2 {
                        self.add_entry(std::mem::take(&mut entry));
                    }
                    entry.useragents.push(value.to_string());
                    state = 1;
                }
                "disallow" => {
                    if state != 0 {
                        entry.rulelines.push(RuleLine::new(value, false));
                        state = 2;
                    }
                }
                "allow" => {
                    if state != 0 {
                        entry.rulelines.push(RuleLine::new(value, true));
                        state = 2;
                    }
                }
                "crawl-delay" => {
                    if state != 0 {
                        if is_ascii_digits(value)
                            && let Ok(n) = value.parse::<i64>()
                        {
                            entry.delay = Some(n);
                        }
                        state = 2;
                    }
                }
                "request-rate" => {
                    if state != 0 {
                        let nums: Vec<&str> = value.split('/').collect();
                        if nums.len() == 2
                            && is_ascii_digits(nums[0].trim())
                            && is_ascii_digits(nums[1].trim())
                            && let (Ok(r), Ok(s)) =
                                (nums[0].trim().parse::<u64>(), nums[1].trim().parse::<u64>())
                        {
                            entry.req_rate = Some(RequestRate { requests: r, seconds: s });
                        }
                        state = 2;
                    }
                }
                "sitemap" => {
                    // Independent of user-agent, so the state is left unchanged.
                    self.sitemaps.push(value.to_string());
                }
                _ => {}
            }
        }
        if state == 2 {
            self.add_entry(entry);
        }
    }

    /// Decide whether `useragent` may fetch `url`, per the parsed robots.txt. Port of `can_fetch`.
    ///
    /// Returns `Err` only when the URL's authority is malformed (CPython raises `ValueError` from
    /// the internal `urlsplit`).
    pub fn can_fetch(&self, useragent: &str, url: &str) -> Result<bool, UrlError> {
        if self.disallow_all {
            return Ok(false);
        }
        if self.allow_all {
            return Ok(true);
        }
        // Until the file has been parsed, assume nothing is allowed.
        if !self.last_checked {
            return Ok(false);
        }
        let parsed = urlsplit(url, "", true)?;
        let url = urlunsplit("", "", &parsed.path, &parsed.query, &parsed.fragment);
        let url = normalize_path(&url);
        let url = if url.is_empty() { "/".to_string() } else { url };
        for entry in &self.entries {
            if entry.applies_to(useragent) {
                return Ok(entry.allowance(&url));
            }
        }
        if let Some(default) = &self.default_entry {
            return Ok(default.allowance(&url));
        }
        // agent not found ==> access granted
        Ok(true)
    }

    /// The `Crawl-delay` for `useragent`, or `None`. Port of `crawl_delay`.
    pub fn crawl_delay(&self, useragent: &str) -> Option<i64> {
        if !self.last_checked {
            return None;
        }
        for entry in &self.entries {
            if entry.applies_to(useragent) {
                return entry.delay;
            }
        }
        self.default_entry.as_ref().and_then(|e| e.delay)
    }

    /// The `Request-rate` for `useragent`, or `None`. Port of `request_rate`.
    pub fn request_rate(&self, useragent: &str) -> Option<RequestRate> {
        if !self.last_checked {
            return None;
        }
        for entry in &self.entries {
            if entry.applies_to(useragent) {
                return entry.req_rate;
            }
        }
        self.default_entry.as_ref().and_then(|e| e.req_rate)
    }

    /// The `Sitemap` URLs, or `None` if there were none. Port of `site_maps`.
    pub fn site_maps(&self) -> Option<&[String]> {
        if self.sitemaps.is_empty() {
            None
        } else {
            Some(&self.sitemaps)
        }
    }
}

impl fmt::Display for RobotFileParser {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut entries: Vec<&Entry> = self.entries.iter().collect();
        if let Some(default) = &self.default_entry {
            entries.push(default);
        }
        let joined: Vec<String> = entries.iter().map(|e| e.to_string()).collect();
        f.write_str(&joined.join("\n\n"))
    }
}

/// Port of CPython's `str.isdigit()` for the cases the parser exercises: a non-empty run of ASCII
/// digits. (CPython also accepts some Unicode digit code points; those are not modelled.)
fn is_ascii_digits(s: &str) -> bool {
    !s.is_empty() && s.bytes().all(|b| b.is_ascii_digit())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parsed(text: &str) -> RobotFileParser {
        let mut rp = RobotFileParser::new("http://example.com/robots.txt");
        let lines: Vec<&str> = text.split('\n').collect();
        rp.parse(&lines);
        rp
    }

    #[test]
    fn basic_allow_disallow() {
        let rp = parsed("User-agent: *\nDisallow: /private\nAllow: /private/ok");
        assert_eq!(rp.can_fetch("Bot", "http://example.com/public"), Ok(true));
        assert_eq!(rp.can_fetch("Bot", "http://example.com/private/x"), Ok(false));
        // First-match wins: /private matches before /private/ok is considered.
        assert_eq!(rp.can_fetch("Bot", "http://example.com/private/ok"), Ok(false));
    }

    #[test]
    fn empty_disallow_allows_all() {
        let rp = parsed("User-agent: *\nDisallow:");
        assert_eq!(rp.can_fetch("Bot", "http://example.com/anything"), Ok(true));
    }

    #[test]
    fn not_parsed_denies() {
        let rp = RobotFileParser::new("http://example.com/robots.txt");
        assert_eq!(rp.can_fetch("Bot", "http://example.com/"), Ok(false));
    }

    #[test]
    fn agent_specific_overrides_default() {
        let rp = parsed("User-agent: BadBot\nDisallow: /\n\nUser-agent: *\nDisallow: /admin");
        assert_eq!(rp.can_fetch("BadBot", "http://example.com/x"), Ok(false));
        assert_eq!(rp.can_fetch("GoodBot", "http://example.com/x"), Ok(true));
        assert_eq!(rp.can_fetch("GoodBot", "http://example.com/admin"), Ok(false));
    }

    #[test]
    fn crawl_delay_and_request_rate() {
        let rp = parsed("User-agent: *\nCrawl-delay: 10\nRequest-rate: 1/5");
        assert_eq!(rp.crawl_delay("Bot"), Some(10));
        assert_eq!(rp.request_rate("Bot"), Some(RequestRate { requests: 1, seconds: 5 }));
    }

    #[test]
    fn sitemaps() {
        let rp = parsed("Sitemap: http://example.com/sitemap.xml\nUser-agent: *\nDisallow:");
        assert_eq!(rp.site_maps(), Some(&["http://example.com/sitemap.xml".to_string()][..]));
    }
}
