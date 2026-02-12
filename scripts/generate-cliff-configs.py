#!/usr/bin/env python3
"""Generate cliff.toml configs from commit-types.json (single source of truth)."""

import json
import sys
from pathlib import Path

CHANGELOG_TEMPLATE = '''# git-cliff config for {name}
# Auto-generated from commit-types.json - edit that file instead

[changelog]
header = """# Changelog

All notable changes to this project will be documented in this file.
"""
body = """
{{% if version %}}\\
## [{{{{ version | trim_start_matches(pat="v") }}}}] - {{{{ timestamp | date(format="%Y-%m-%d") }}}}
{{% else %}}\\
## [unreleased]
{{% endif %}}\\
{{% for group, group_commits in commits | group_by(attribute="group") %}}\\
{{% if group != "_ignored" %}}
### {{{{ group | upper_first }}}}
{{% for commit in group_commits %}}
- {{{{ commit.message | upper_first }}}}
{{% endfor %}}
{{% endif %}}\\
{{% endfor %}}\\
"""
trim = true

[git]
conventional_commits = true
filter_unconventional = true
tag_pattern = "{tag_pattern}"
commit_parsers = [
{commit_parsers}
]
'''


def generate_commit_parsers(types: dict) -> str:
    """Generate commit_parsers array entries from types config."""
    lines = []
    for type_name, type_config in types.items():
        group = type_config.get("changelog_group")
        if group:
            lines.append(f'    {{ message = "^{type_name}", group = "{group}" }},')
    lines.append('    { message = ".*", group = "_ignored" },')
    return "\n".join(lines)


def main():
    root = Path(__file__).parent.parent
    config_path = root / "commit-types.json"

    with open(config_path) as f:
        config = json.load(f)

    types = config["types"]
    commit_parsers = generate_commit_parsers(types)
    dry_run = "--dry-run" in sys.argv

    output_path = root / "cliff.toml"
    content = CHANGELOG_TEMPLATE.format(
        name="repoverlay",
        tag_pattern="v[0-9].*",
        commit_parsers=commit_parsers,
    )

    if dry_run:
        print(content)
    else:
        output_path.write_text(content)
        print(f"Wrote {output_path}")


if __name__ == "__main__":
    main()
