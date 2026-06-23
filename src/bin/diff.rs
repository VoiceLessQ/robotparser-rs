//! Differential-testing CLI. Each stdin line is one test: tab-separated fields where field 0 is a
//! robots.txt document (its lines joined by U+001F) and the remaining fields are queries. One
//! output line is written per input, the query results joined by U+001E. `difftest.py` feeds
//! identical input to this binary and to `urllib.robotparser`, then compares line by line.
//!
//! A query field is U+001F-separated: `CF<agent><url>` (can_fetch), `CD<agent>` (crawl_delay),
//! `RR<agent>` (request_rate), `SM` (site_maps), `STR` (str). Results: `True`/`False`/`ERR`,
//! an integer or `None`, `requests/seconds` or `None`, sitemaps joined by U+001F or `None`, and
//! `str(rp)` with newlines mapped to U+001F. Corpus text never contains tab/U+001F/U+001E.

use std::io::{self, Read, Write};

use robotparser_rs::RobotFileParser;

const US: char = '\u{1f}';
const RS: char = '\u{1e}';

fn run_query(rp: &RobotFileParser, q: &str) -> String {
    let f: Vec<&str> = q.split(US).collect();
    match f[0] {
        "CF" => match rp.can_fetch(f[1], f[2]) {
            Ok(true) => "True".to_string(),
            Ok(false) => "False".to_string(),
            Err(_) => "ERR".to_string(),
        },
        "CD" => match rp.crawl_delay(f[1]) {
            Some(n) => n.to_string(),
            None => "None".to_string(),
        },
        "RR" => match rp.request_rate(f[1]) {
            Some(r) => format!("{}/{}", r.requests, r.seconds),
            None => "None".to_string(),
        },
        "SM" => match rp.site_maps() {
            Some(s) => s.join(&US.to_string()),
            None => "None".to_string(),
        },
        "STR" => rp.to_string().replace('\n', &US.to_string()),
        other => panic!("unknown query {other:?}"),
    }
}

fn dispatch(line: &str) -> String {
    let fields: Vec<&str> = line.split('\t').collect();
    let doc_lines: Vec<&str> = fields[0].split(US).collect();
    let mut rp = RobotFileParser::new("http://example.com/robots.txt");
    rp.parse(&doc_lines);
    fields[1..]
        .iter()
        .map(|q| run_query(&rp, q))
        .collect::<Vec<_>>()
        .join(&RS.to_string())
}

fn main() {
    let mut input = String::new();
    io::stdin().read_to_string(&mut input).expect("read stdin");

    let stdout = io::stdout();
    let mut out = io::BufWriter::new(stdout.lock());
    for line in input.lines() {
        writeln!(out, "{}", dispatch(line)).expect("write");
    }
}
