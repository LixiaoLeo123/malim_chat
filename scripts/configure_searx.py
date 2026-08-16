#!/usr/bin/env python3
"""Configure SearXNG for malim_chat's local search service."""

import secrets
import sys
from pathlib import Path

import yaml


def main() -> None:
    source, target = map(Path, sys.argv[1:])
    settings = yaml.safe_load(source.read_text(encoding="utf-8"))
    settings["general"]["instance_name"] = "malim_chat search"
    settings["server"]["bind_address"] = "127.0.0.1"
    settings["server"]["port"] = 8888
    settings["server"]["secret_key"] = secrets.token_urlsafe(48)
    settings["search"]["default_lang"] = "zh-CN"
    target.write_text(yaml.safe_dump(settings, sort_keys=False, allow_unicode=True), encoding="utf-8")


if __name__ == "__main__":
    main()
