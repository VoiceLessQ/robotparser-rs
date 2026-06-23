r"""Differential test: robotparser-rs vs Python's `urllib.robotparser`.

Feeds identical (robots.txt document, queries) tests to the `diff` binary and to a real
`RobotFileParser`, checking every result agrees. Run from the robotparser-rs/ folder after
`cargo build`:
    ..\..\python\python\python.exe difftest.py
"""

import os
import subprocess
import sys
import urllib.robotparser as rp

HERE = os.path.dirname(os.path.abspath(__file__))
RUST_BIN = os.path.join(HERE, "target", "debug", "diff.exe" if os.name == "nt" else "diff")

US = "\x1f"  # field/line/list separator within a record
RS = "\x1e"  # separator between query results

# robots.txt documents, each a list of lines. Covers: basic allow/disallow, empty-disallow,
# agent-specific vs default, multiple agents per entry, key case-insensitivity, comments and odd
# spacing, rules before any user-agent, crawl-delay/request-rate (valid + junk), sitemaps,
# percent-encoded and query-string paths, a literal "*" path, and a couple of real-world shapes.
ROBOTS = [
    ["User-agent: *", "Disallow: /private", "Allow: /private/ok"],
    ["User-agent: *", "Disallow:"],
    ["User-agent: *", "Disallow: /"],
    ["User-agent: BadBot", "Disallow: /", "", "User-agent: *", "Disallow: /admin"],
    ["User-agent: A", "User-agent: B", "Disallow: /ab"],
    ["USER-AGENT: *", "DISALLOW: /X", "ALLOW: /X/y"],
    ["# comment", "User-agent: *   # inline", "Disallow:   /sp aces  ", "", "", "Disallow: /b"],
    ["Disallow: /tooEarly", "User-agent: *", "Disallow: /late"],
    ["User-agent: *", "Crawl-delay: 10", "Request-rate: 1/5", "Disallow: /q"],
    ["User-agent: *", "Crawl-delay: notanumber", "Request-rate: 1/x", "Disallow: /q"],
    ["User-agent: *", "Crawl-delay: 3", "Request-rate: 10/60"],
    ["Sitemap: http://example.com/sitemap.xml", "User-agent: *", "Disallow:",
     "Sitemap: http://example.com/s2.xml"],
    ["User-agent: *", "Disallow: /a%2Fb", "Allow: /caf%C3%A9"],
    ["User-agent: *", "Disallow: /search?q=secret&x=1", "Allow: /search?q=ok"],
    ["User-agent: *", "Disallow: *"],
    ["User-agent: *", "disallow: /p", "allow: /p/ok", "crawl-delay: 2"],
    ["User-agent: Googlebot", "Disallow: /no", "", "User-agent: *", "Disallow:"],
    ["random line without colon", "User-agent: *", "Disallow: /x"],
    ["User-agent:*", "Disallow:/tight"],
    [],
    ["User-agent: *", "Disallow: /a", "Disallow: /b", "Allow: /a/keep"],
]

# (agent, path-or-url) pairs to probe with can_fetch.
AGENTS = ["*", "MyBot", "BadBot", "Googlebot/2.1", "A", "B", "googlebot", "CafeBot"]
PATHS = [
    "http://example.com/", "http://example.com/private", "http://example.com/private/ok",
    "http://example.com/private/ok/deep", "http://example.com/admin", "http://example.com/ab",
    "http://example.com/X/y", "http://example.com/sp%20aces", "http://example.com/b",
    "http://example.com/late", "http://example.com/tooEarly", "http://example.com/q",
    "http://example.com/a%2Fb", "http://example.com/café", "http://example.com/caf%C3%A9",
    "http://example.com/search?q=secret&x=1", "http://example.com/search?q=ok",
    "http://example.com/p/ok", "http://example.com/tight", "http://example.com/a/keep",
    "/relative/path", "http://example.com/no", "", "http://[bad/ipv6", "http://example.com/x",
]


def build_commands():
    cmds = []
    for doc in ROBOTS:
        fields = [US.join(doc)]
        for ua in AGENTS:
            for path in PATHS:
                fields.append(US.join(("CF", ua, path)))
            fields.append(US.join(("CD", ua)))
            fields.append(US.join(("RR", ua)))
        fields.append("SM")
        fields.append("STR")
        cmds.append("\t".join(fields))
    return cmds


def query(parser, q):
    f = q.split(US)
    t = f[0]
    if t == "CF":
        try:
            return "True" if parser.can_fetch(f[1], f[2]) else "False"
        except ValueError:
            return "ERR"
    if t == "CD":
        d = parser.crawl_delay(f[1])
        return "None" if d is None else str(d)
    if t == "RR":
        r = parser.request_rate(f[1])
        return "None" if r is None else f"{r.requests}/{r.seconds}"
    if t == "SM":
        s = parser.site_maps()
        return "None" if s is None else US.join(s)
    if t == "STR":
        return str(parser).replace("\n", US)
    raise AssertionError(t)


def expected(line):
    fields = line.split("\t")
    parser = rp.RobotFileParser("http://example.com/robots.txt")
    parser.parse(fields[0].split(US))
    return RS.join(query(parser, q) for q in fields[1:])


def main():
    if not os.path.exists(RUST_BIN):
        sys.exit(f"missing {RUST_BIN} - run `cargo build` first")

    cmds = build_commands()
    proc = subprocess.run(
        [RUST_BIN], input="\n".join(cmds), capture_output=True, text=True, encoding="utf-8",
    )
    if proc.returncode != 0:
        sys.exit(f"rust diff binary failed:\n{proc.stderr}")

    # Split on '\n' only: results legitimately contain U+001E/U+001F, which str.splitlines() would
    # wrongly treat as line boundaries.
    rust = proc.stdout.split("\n")
    if rust and rust[-1] == "":
        rust.pop()
    if len(rust) != len(cmds):
        sys.exit(f"line count mismatch: {len(rust)} rust vs {len(cmds)} commands")

    mismatches = []
    for cmd, got in zip(cmds, rust):
        exp = expected(cmd)
        if exp != got:
            mismatches.append((cmd, exp, got))

    if mismatches:
        print(f"{len(mismatches)} mismatches (of {len(cmds)}):")
        for cmd, exp, got in mismatches[:20]:
            print(f"  doc={cmd.split(chr(9))[0]!r}\n    python={exp!r}\n    rust  ={got!r}")
        sys.exit("\nMISMATCHES FOUND.")

    # Count total query comparisons for a meaningful number.
    total_queries = sum(line.count("\t") for line in cmds)
    print(f"ALL MATCH - {len(cmds)} documents, {total_queries} queries agree with urllib.robotparser.")


if __name__ == "__main__":
    main()
