#!/usr/bin/env python3
import pathlib, re, subprocess, sys

build = sys.argv[1] if len(sys.argv) > 1 else ""
ver = sys.argv[2] if len(sys.argv) > 2 else "0.7.5"
out_dir = pathlib.Path("GiroCrabApp/TestFlight")
out_dir.mkdir(parents=True, exist_ok=True)

readme = pathlib.Path("README.md").read_text() if pathlib.Path("README.md").exists() else ""

def extract_section(heading_prefix: str) -> list[str]:
    m = re.search(rf"^{re.escape(heading_prefix)}[^\n]*\n+(.*?)(?=\n### |\Z)", readme, re.M | re.S)
    if not m:
        return []
    bullets = []
    for line in m.group(1).splitlines():
        line = line.strip()
        if line.startswith("- "):
            clean = re.sub(r"\*\*([^*]+)\*\*", r"\1", line[2:])
            clean = re.sub(r"(?:,\s*|\s*\()#\d+(?:,\s*#\d+)*\)?", lambda m: ")" if m.group(0).startswith(",") and m.group(0).endswith(")") else "", clean)
            bullets.append(f"• {clean}")
    return bullets

de_bullets = extract_section("### ✨ Neu in v")
en_bullets = extract_section("### ✨ What's new in v")

if not de_bullets:
    commits = subprocess.check_output(
        ["git", "log", "-n", "12", "--no-merges", "--pretty=format:%s"], text=True
    ).splitlines()
    de_bullets = [f"• {c}" for c in commits if not c.startswith("chore:")]
    en_bullets = de_bullets

header_de = f"Neu in Build {build} (v{ver}):\n\n" if build else f"Neu in v{ver}:\n\n"
header_en = f"What's new in Build {build} (v{ver}):\n\n" if build else f"What's new in v{ver}:\n\n"

(out_dir / "WhatToTest.de-DE.txt").write_text(header_de + "\n".join(de_bullets) + "\n")
(out_dir / "WhatToTest.en-US.txt").write_text(header_en + "\n".join(en_bullets) + "\n")
print(f"==> Generated TestFlight WhatToTest notes (Build {build}, v{ver})")
