#!/usr/bin/env python3
"""Generate .commitlintrc.json from commit-types.json (single source of truth)."""

import json
import sys
from pathlib import Path


def main():
    root = Path(__file__).parent.parent
    config_path = root / "commit-types.json"
    output_path = root / ".commitlintrc.json"

    with open(config_path) as f:
        config = json.load(f)

    types = list(config["types"].keys())

    commitlint_config = {
        "extends": ["@commitlint/config-conventional"],
        "rules": {
            "type-enum": [2, "always", types],
            **config["commitlint_rules"],
        },
    }

    output = json.dumps(commitlint_config, indent=2) + "\n"

    if "--dry-run" in sys.argv:
        print(output)
    else:
        output_path.write_text(output)
        print(f"Wrote {output_path}")


if __name__ == "__main__":
    main()
