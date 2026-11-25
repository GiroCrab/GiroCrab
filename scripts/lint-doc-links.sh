#!/usr/bin/env bash
# Relative links in the tracked Markdown files point to something that exists (#1875):
#   * `[…](path)`, `![…](path)`, `[ref]: path` and HTML `href`/`src` lead to an existing file or directory;
#   * an anchor (`file.md#section`, `#section`) matches a heading of the target file, slugged the
#     way GitHub does it (lowercase; umlauts and `_` stay; punctuation, `&`, en dashes and emoji
#     are dropped; spaces become `-`; repeats get `-1`, `-2`), or an `id`/`name` attribute.
# Not checked: external URLs (network), line anchors (`#L42`), links that leave the repository
# (`../../releases` works only on github.com), setext headings (`Title` over `===`), fenced code,
# inline code, front matter, the ADR template and the legacy cashcrab directories.
# Runs a self-test first (milliseconds).
set -euo pipefail
cd "$(dirname "$0")/.."
python3 -I - <<'PY'
import html
import re
import subprocess
import sys
import tempfile
import unicodedata
from pathlib import Path
from urllib.parse import unquote

SKIP = re.compile(r"^(cashcrab-core|CashCrabApp)/|(^|/)0000-template\.md$")
FENCE = re.compile(r"^ {0,3}(`{3,}|~{3,})")
ATX = re.compile(r"^ {0,3}#{1,6}[ \t]+(.*?)(?:[ \t]+#+)?[ \t]*$")
# `](target "title")`: the end of every inline link or image, also of an image inside a link.
INLINE = re.compile(r"\]\(\s*(<[^>]*>|[^\s)]+(?:\([^\s)]*\)[^\s)]*)*)(?:\s+(?:\"[^\"]*\"|'[^']*'))?\s*\)")
REFDEF = re.compile(r"^ {0,3}\[(?!\^)[^\]]+\]:\s*(<[^>]*>|\S+)")  # not footnotes `[^1]:`
HTML_LINK = re.compile(r"""<[a-zA-Z][^>]*?\s(?:href|src)\s*=\s*["']([^"']+)["']""")
HTML_ID = re.compile(r"""<[a-zA-Z][^>]*?\s(?:id|name)\s*=\s*["']([^"']+)["']""")
SCHEME = re.compile(r"^[a-zA-Z][a-zA-Z0-9+.-]*:")
LINE_ANCHOR = re.compile(r"^L\d+(C\d+)?(-L\d+(C\d+)?)?$")
CODE_SPAN = re.compile(r"(`+)(.+?)\1")
UNDERSCORE_EMPHASIS = re.compile(r"(?<!\w)(_+)(?=\S)(.+?)(?<=\S)\1(?!\w)")  # not inside words


def lines(text):
    """(line number, line) outside front matter and fenced code blocks."""
    rows = text.split("\n")
    start = 0
    if rows and rows[0].strip() == "---":
        start = next((i + 1 for i in range(1, len(rows)) if rows[i].strip() in ("---", "...")), 0)
    fence = None
    for i in range(start, len(rows)):
        m = FENCE.match(rows[i])
        if fence:
            closing = rows[i].strip()
            if m and set(closing) == {fence[0]} and len(closing) >= len(fence):
                fence = None
        elif m:
            fence = m.group(1)
        else:
            yield i + 1, rows[i]


def slug(heading):
    """Base anchor GitHub gives a heading: its rendered text, then github-slugger."""
    parts = CODE_SPAN.split(heading)  # text, ticks, code, text, …: code spans stay literal
    for i in range(0, len(parts), 3):
        s = re.sub(r"!?\[([^\]]*)\]\([^)]*\)", r"\1", parts[i])  # links and images: their text
        s = re.sub(r"<[^>]+>", "", s)  # HTML tags
        parts[i] = html.unescape(UNDERSCORE_EMPHASIS.sub(r"\2", s))
    text = "".join(p for i, p in enumerate(parts) if i % 3 != 1)
    keep = (c for c in text.lower() if c in " -_" or unicodedata.category(c)[0] in "LMN")
    return "".join(keep).replace(" ", "-")


def anchors(text):
    used, out = {}, set()
    for _, line in lines(text):
        m = ATX.match(line)
        if m:
            a = base = slug(m.group(1))
            while a in used:  # github-slugger: count up until the anchor is free
                used[base] += 1
                a = f"{base}-{used[base]}"
            used[a] = 0
            out.add(a)
        out.update(HTML_ID.findall(line))
    return out


def targets(text):
    for n, line in lines(text):
        line = CODE_SPAN.sub("", line)
        for t in INLINE.findall(line) + HTML_LINK.findall(line):
            yield n, t
        m = REFDEF.match(line)
        if m:
            yield n, m.group(1)


def read(path):
    try:
        return path.read_text(encoding="utf-8")
    except UnicodeDecodeError as e:
        sys.exit(f"{path}: not UTF-8 ({e.reason})")


def check(root, files):
    """-> [(file, line, target, problem)]"""
    root = root.resolve()
    errors, cache = [], {}
    for f in files:
        if not (root / f).exists():
            continue  # tracked, but deleted in the working tree
        for n, t in targets(read(root / f)):
            t = html.unescape(t[1:-1] if t.startswith("<") else t)
            if SCHEME.match(t) or t.startswith("//"):
                continue
            path, _, frag = t.partition("#")
            path = unquote(path.partition("?")[0])
            base = root if path.startswith("/") else (root / f).parent
            dest = (base / path.lstrip("/")).resolve() if path else root / f
            if not dest.is_relative_to(root):
                continue  # leaves the repository
            rel = dest.relative_to(root)
            if not dest.exists():
                errors.append((f, n, t, f"{rel} does not exist"))
            elif frag and dest.suffix.lower() == ".md" and not LINE_ANCHOR.match(frag):
                if dest not in cache:
                    cache[dest] = anchors(read(dest))
                frag = unquote(frag)
                if frag not in cache[dest] and frag.lower() not in cache[dest]:
                    errors.append((f, n, t, f"no heading or id #{frag} in {rel}"))
    return errors


def self_test():
    # Expected anchors as rendered by GitHub (Markdown API, November 2026).
    for heading, want in [
        ("0003 – Rollen & Regeln", "0003--rollen--regeln"),
        ("Übersicht: Konten (DE/EN)", "übersicht-konten-deen"),
        ("`sync_docs.py` und **fett** und [Link](x.md)", "sync_docspy-und-fett-und-link"),
        ("✨ Neu in v0.9.0", "-neu-in-v090"),
        ("Fragen? Antworten! 100% <br> ok", "fragen-antworten-100--ok"),
        ("Einmalige Einrichtung: `WEBSITE_DEPLOY_TOKEN`", "einmalige-einrichtung-website_deploy_token"),
        ("Typ `Vec<T>` und `a_b_c` und `**x**`", "typ-vect-und-a_b_c-und-x"),
        ("snake_case_name und _betont_ und __fett__ und *stern*", "snake_case_name-und-betont-und-fett-und-stern"),
        ("Tom &amp; Jerry &lt;3", "tom--jerry-3"),
        ("a*b*c und x__y__z", "abc-und-x__y__z"),
        ("<code>raw_html</code> und <b>fett</b>", "raw_html-und-fett"),
    ]:
        assert slug(heading) == want, (heading, slug(heading), want)
    assert anchors("# Foo\n# Foo\n# Foo 1\n") == {"foo", "foo-1", "foo-1-1"}
    with tempfile.TemporaryDirectory() as d:
        root = Path(d)
        (root / "docs").mkdir()
        (root / "docs/b.md").write_text("# Dup\n## Dup\n## Trailing ##\n<a id=\"Own\"></a>\n"
                                        "```\n# Not a heading\n```\n````md\n```js\n# Nor this\n````\n")
        (root / "a.md").write_text(
            "---\nx: [y](nope.md)\n---\n# A\n"
            "[ok](docs/b.md#dup-1) [ok](docs/b.md#trailing) [ok](docs/b.md#Own) [ok](#a) [ok](docs/)\n"
            "[ok](https://x.invalid/nope.md) [ok](docs/b.md#L3) [ok](../../releases) `[ok](nope.md)`\n"
            "[![ok](docs/b.md)](docs/) <img src=\"docs/b.md\">\n[^1]: footnote\n"
            "```md\n[ok](nope.md)\n```\n"
            "[bad](nope.md)\n[bad](docs/b.md#dup-2)\n[bad](docs/b.md#not-a-heading)\n"
            "[bad](docs/b.md#nor-this)\n<a href=\"docs/gone.png\">x</a>\n[ref]: docs/missing.md\n")
        got = [e[2] for e in check(root, ["a.md"])]
        want = ["nope.md", "docs/b.md#dup-2", "docs/b.md#not-a-heading", "docs/b.md#nor-this",
                "docs/gone.png", "docs/missing.md"]
        assert got == want, got


self_test()
root = Path.cwd()
listed = subprocess.run(["git", "ls-files", "-z", "*.md"], stdout=subprocess.PIPE, text=True, check=True).stdout
errors = check(root, [f for f in listed.split("\0") if f and not SKIP.search(f)])
if errors:
    print("Broken relative links in Markdown files:", file=sys.stderr)
    for f, n, t, problem in errors:
        print(f"  {f}:{n}: ({t}) {problem}", file=sys.stderr)
    print("Fix the path or heading (anchors follow GitHub: lowercase, spaces to '-', punctuation dropped).",
          file=sys.stderr)
    sys.exit(1)
PY
