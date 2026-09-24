#!/usr/bin/env python3
import glob
import re
import sys

css = open("web/app/src/app.css", encoding="utf-8").read()
defined = set(re.findall(r"\.([A-Za-z][\w-]*)", css))

used: dict[str, set[str]] = {}
for path in sorted(glob.glob("web/app/src/**/*.tsx", recursive=True)):
    text = open(path, encoding="utf-8").read()
    text = re.sub(r"\{/\*.*?\*/\}", "", text, flags=re.S)
    text = re.sub(r"/\*.*?\*/", "", text, flags=re.S)
    for m in re.finditer(r'class="([^"{}]*)"', text):
        for cls in m.group(1).split():
            used.setdefault(cls, set()).add(path.split("src/")[1])

missing = {c: f for c, f in used.items() if c not in defined}
if not missing:
    print(f"verify_web_classes_styled: OK ({len(used)} classes, all styled)")
    sys.exit(0)

print("verify_web_classes_styled: markup uses classes that no CSS rule defines.\n",
      file=sys.stderr)
for cls, files in sorted(missing.items()):
    print(f"  .{cls:24} used in {', '.join(sorted(files))}", file=sys.stderr)
print("\nEach renders as browser defaults on the device while every host test\n"
      "stays green. Add the rule, or use the house class that already has one.",
      file=sys.stderr)
sys.exit(1)
