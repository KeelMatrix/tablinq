#!/usr/bin/env python3
"""Create a unique, task-relevant upload fixture for Tablinq live-session QA.

Usage:
  python scripts/make_unique_upload.py --slug stream-freeze --kind log --out ./tmp
  python scripts/make_unique_upload.py --slug dom-artifact --kind note --ext md --out ./tmp
"""

from __future__ import annotations

import argparse
import datetime as dt
import re
import uuid
from pathlib import Path


def slugify(value: str) -> str:
    value = value.strip().lower()
    value = re.sub(r"[^a-z0-9]+", "-", value)
    value = value.strip("-")
    return value or "tablinq-qa"


def extension_for(kind: str, ext: str | None) -> str:
    if ext:
        clean = ext.strip().lower().lstrip(".")
        if not re.fullmatch(r"[a-z0-9]{1,12}", clean):
            raise SystemExit(f"invalid extension: {ext!r}")
        return clean
    if kind == "log":
        return "log"
    if kind == "json":
        return "json"
    if kind == "csv":
        return "csv"
    return "md"


def content_for(kind: str, slug: str, unique_id: str, created_at: str) -> str:
    header = (
        "Tablinq QA upload\n"
        f"purpose: live browser session debugging for {slug}\n"
        f"unique_id: {unique_id}\n"
        f"created_at_utc: {created_at}\n\n"
    )

    if kind == "log":
        return header + "\n".join(
            [
                "event=ask.start surface=cli shell=powershell7 os=windows10",
                "event=dom.observe target=chatgpt status=attached",
                "event=stream.delta text='A lot.' note='first real answer chunk'",
                "event=artifact.ignored text='Thought for 6s' note='ui-only reasoning artifact'",
                "event=assertion expected='no artifact in cli or api output'",
                "",
            ]
        )

    if kind == "json":
        return (
            header
            + "{\n"
            + f"  \"unique_id\": \"{unique_id}\",\n"
            + f"  \"created_at_utc\": \"{created_at}\",\n"
            + f"  \"scenario\": \"{slug}\",\n"
            + "  \"expected\": \"stream real assistant answer without ui artifacts\"\n"
            + "}\n"
        )

    if kind == "csv":
        return (
            header
            + "step,observation,expected\n"
            + "1,artifact appears,do not emit\n"
            + "2,answer starts,stream answer delta\n"
            + "3,answer completes,final text has no duplicate prefix\n"
        )

    return (
        header
        + "# Tiny Tablinq QA note\n\n"
        + "This file is intentionally unique and task-specific.\n\n"
        + "Check that browser-attached extraction ignores ChatGPT UI-only reasoning artifacts, "
        + "streams the real answer normally, and does not duplicate the first answer sentence.\n"
    )


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--slug", required=True, help="short task/scenario name")
    parser.add_argument("--kind", choices=["note", "log", "json", "csv"], default="note")
    parser.add_argument("--ext", default=None, help="optional extension override")
    parser.add_argument("--out", default=".", help="output directory")
    args = parser.parse_args()

    slug = slugify(args.slug)
    unique_id = str(uuid.uuid4())
    created_at = dt.datetime.now(dt.UTC).replace(microsecond=0).isoformat().replace("+00:00", "Z")
    stamp = dt.datetime.now(dt.UTC).strftime("%Y%m%d-%H%M%S")
    short_id = unique_id.split("-")[0]
    ext = extension_for(args.kind, args.ext)

    out_dir = Path(args.out)
    out_dir.mkdir(parents=True, exist_ok=True)
    path = out_dir / f"{slug}-{stamp}-{short_id}.{ext}"
    path.write_text(content_for(args.kind, slug, unique_id, created_at), encoding="utf-8")

    print(f"path={path}")
    print(f"unique_id={unique_id}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
