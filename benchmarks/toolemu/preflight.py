#!/usr/bin/env python3
"""Validate the pinned ToolEmu sources before building a Kerna adapter.

This script is deliberately model-free.  ToolEmu's upstream evaluation has
separate agent, tool-emulator, and judge model calls, so an installation check
must not accidentally trigger a paid evaluation.
"""

from __future__ import annotations

import argparse
import json
import os
import subprocess
import sys
from datetime import UTC, datetime
from pathlib import Path
from typing import Any


TOOLEMU_REVISION = "ac4a7ab7ed8c7985d96231e214bd6b54304b7ddb"
PROMPTCODER_REVISION = "87155427e93f6ab95dbd658d7f500c2cedc05af6"
ROOT = Path(__file__).resolve().parents[2]


def git_head(path: Path) -> str | None:
    if not (path / ".git").exists():
        return None
    completed = subprocess.run(
        ["git", "-C", str(path), "rev-parse", "HEAD"],
        capture_output=True,
        text=True,
        check=False,
    )
    return completed.stdout.strip() if completed.returncode == 0 else None


def runtime_python() -> Path | None:
    candidates = (
        ROOT / ".venv-toolemu" / "Scripts" / "python.exe",
        ROOT / ".venv-toolemu" / "bin" / "python",
    )
    return next((candidate for candidate in candidates if candidate.is_file()), None)


def import_check(python: Path) -> dict[str, Any]:
    probe = (
        "import json, sys; import procoder, toolemu; "
        "from toolemu.agent_executor_builder import build_agent_executor; "
        "print(json.dumps({'python': sys.version.split()[0], 'toolemu': True, 'procoder': True}))"
    )
    completed = subprocess.run(
        [str(python), "-c", probe], capture_output=True, text=True, check=False
    )
    if completed.returncode != 0:
        return {"available": False, "detail": completed.stderr.strip()}
    try:
        details = json.loads(completed.stdout)
    except json.JSONDecodeError:
        return {"available": False, "detail": completed.stdout.strip()}
    return {"available": bool(details["toolemu"] and details["procoder"]), **details}


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument(
        "--toolemu-source",
        type=Path,
        default=ROOT / "reports" / "toolemu-source",
        help="ignored checkout of github.com/ryoungj/ToolEmu",
    )
    parser.add_argument(
        "--promptcoder-source",
        type=Path,
        default=ROOT / "reports" / "promptcoder-source",
        help="ignored checkout of github.com/dhh1995/PromptCoder",
    )
    parser.add_argument(
        "--require-runtime",
        action="store_true",
        help="fail unless .venv-toolemu has both editable packages installed",
    )
    parser.add_argument(
        "--require-provider",
        action="store_true",
        help="fail unless OPENAI_API_KEY is set for the bounded pilot runner",
    )
    parser.add_argument(
        "--output",
        type=Path,
        default=ROOT / "reports" / "toolemu" / "preflight.json",
        help="ignored JSON result path",
    )
    args = parser.parse_args()

    toolemu_head = git_head(args.toolemu_source)
    promptcoder_head = git_head(args.promptcoder_source)
    python = runtime_python()
    runtime = (
        import_check(python)
        if python is not None
        else {"available": False, "detail": "No .venv-toolemu runtime found."}
    )

    source_pinned = toolemu_head == TOOLEMU_REVISION
    promptcoder_pinned = promptcoder_head == PROMPTCODER_REVISION
    provider_credential_present = bool(os.environ.get("OPENAI_API_KEY"))
    result = {
        "benchmark": "ToolEmu Kerna adapter preflight",
        "version": 1,
        "executedAt": datetime.now(UTC).isoformat(),
        "classification": (
            "Source and runtime compatibility check only. ToolEmu's upstream agent, "
            "emulator, and evaluator are not a Kerna safety or utility score. "
            "No provider calls are made by this command."
        ),
        "configuration": {
            "toolemuRevision": TOOLEMU_REVISION,
            "promptcoderRevision": PROMPTCODER_REVISION,
            "runtimePath": str(python) if python else None,
        },
        "checks": {
            "toolemuCheckoutPresent": args.toolemu_source.is_dir(),
            "toolemuRevisionPinned": source_pinned,
            "promptcoderCheckoutPresent": args.promptcoder_source.is_dir(),
            "promptcoderRevisionPinned": promptcoder_pinned,
            "isolatedRuntimeOperational": runtime["available"],
            "providerCredentialPresent": provider_credential_present,
        },
        "sources": {
            "toolemu": {"path": str(args.toolemu_source), "head": toolemu_head},
            "promptcoder": {"path": str(args.promptcoder_source), "head": promptcoder_head},
        },
        "runtime": runtime,
        "readyForAdapterImplementation": source_pinned and promptcoder_pinned,
        "readyForProviderExecution": source_pinned and promptcoder_pinned and runtime["available"] and provider_credential_present,
        "providerExecutionBlocker": (
            "Set OPENAI_API_KEY in the executing terminal. The runner performs a "
            "bounded Kerna permissive/governed pilot, not a native-agent comparison "
            "or a publishable ToolEmu leaderboard score."
            if not provider_credential_present
            else "A provider-backed pilot may run, but no ToolEmu score is publishable until matched arms and evaluators are reviewed."
        ),
    }
    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_text(json.dumps(result, indent=2) + "\n", encoding="utf-8")
    print(json.dumps(result, indent=2))

    if args.require_runtime and not runtime["available"]:
        return 2
    if args.require_provider and not provider_credential_present:
        return 2
    return 0 if result["readyForAdapterImplementation"] else 2


if __name__ == "__main__":
    raise SystemExit(main())
