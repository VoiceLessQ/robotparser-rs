# robotparser-rs

[![CI](https://github.com/VoiceLessQ/robotparser-rs/actions/workflows/ci.yml/badge.svg)](https://github.com/VoiceLessQ/robotparser-rs/actions/workflows/ci.yml)

A Rust port of Python's
[`urllib.robotparser`](https://docs.python.org/3/library/urllib.robotparser.html). Parses a
`robots.txt` file and answers `can_fetch` / `crawl_delay` / `request_rate` / `site_maps`. Behaviour
is verified against the reference `urllib.robotparser` module itself across thousands of queries.

The goal is **CPython parity**: mirroring what CPython actually does today. That means a **prefix-match, first-match** algorithm (no `*`/`$` wildcards,
no longest-match precedence), with CPython's exact path normalization, parse state machine, and
user-agent substring matching. This is the right choice when porting Python code that needs identical
decisions; a WHATWG/Google-spec parser behaves differently.

## Usage

```rust
use robotparser_rs::RobotFileParser;

let mut rp = RobotFileParser::new("http://example.com/robots.txt");
rp.parse(&[
    "User-agent: *",
    "Disallow: /private",
    "Allow: /private/ok",
    "Crawl-delay: 5",
]);

assert_eq!(rp.can_fetch("MyBot", "http://example.com/public"), Ok(true));
assert_eq!(rp.can_fetch("MyBot", "http://example.com/private/x"), Ok(false));
assert_eq!(rp.crawl_delay("MyBot"), Some(5));
```

Fetching the file is left to you (CPython's `read()` is a thin `urlopen` wrapper): retrieve the
bytes however you like, then feed the lines to [`parse`](RobotFileParser::parse), exactly as
CPython's `read()` does internally.

## Installation

```sh
cargo add robotparser-rs
```

Requires a Rust toolchain with 2024-edition support (Rust 1.85 or newer).

## How it matches CPython

- **Matching** is prefix-based and first-match: a rule applies when the request path
  `starts_with` the rule's path, and the first applicable `Allow`/`Disallow` decides. An empty
  `Disallow:` means allow-all.
- **Path normalization** round-trips percent-encoding (`quote(unquote(path))`) and, within a
  `?query`, normalizes each run of non-`=&` characters, implemented byte-exactly via the
  [`urlparse-rs`](https://crates.io/crates/urlparse-rs) primitives this crate is built on.
- **User-agent matching** lowercases the agent token (up to the first `/`) and tests each entry's
  agents as substrings; `*` is the catch-all, used only when no specific entry applies.
- **`crawl-delay`** is read only when the value is all digits; **`request-rate`** only for an
  `int/int` value; **`sitemap`** lines are collected independently of user-agent.

The deliberate non-goals match CPython's own: no Google `*`/`$` wildcard expansion, and the
network `read()` path is left to the caller. (A few Unicode corner cases of `str.isdigit()` and
`str.lower()` are not modelled, matching only ASCII there.)

## Verification

Correctness is checked on every push. CI exercises the crate over a corpus of robots.txt documents
and queries and compares **every result against Python's own `urllib.robotparser`** (CPython 3.13);
the build fails on any divergence. The green badge above means the port currently matches CPython
exactly across the whole corpus.

Reproduce it locally (needs Python 3.13 on `PATH`):

```sh
cargo build
python difftest.py     # prints "ALL MATCH - N documents, M queries agree ..." on success
```

[`src/bin/diff.rs`](src/bin/diff.rs) exposes the crate over a small stdin protocol;
[`difftest.py`](difftest.py) drives both it and the reference module and diffs the output line by
line. The current corpus is 21 documents / 4,578 query comparisons.

Beyond that corpus, the crate is cross-checked against the scenarios in CPython's own upstream
[`test_robotparser.py`](https://github.com/python/cpython/blob/v3.13.13/Lib/test/test_robotparser.py),
the same tests CPython ships: all 16 scenarios / 125
`can_fetch`/`crawl_delay`/`request_rate`/`site_maps` checks agree, and CPython 3.13.13 itself agrees
with the suite's asserted `Allow`/`Disallow` labels.

## License

Licensed under the [MIT License](LICENSE-MIT).
