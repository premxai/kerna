"""Would this edit have worked? (P1.2-E)

The last missing piece. `evidence.py` evaluates a gate over `check_verdict`, and nothing
produces one, so every arm reports *not measured*. This applies a proposed edit to a
throwaway copy of the tree and runs the tests, which is the only kind of check that has
ever discriminated in this project.

## Two checks, deliberately not merged

  * **form** — does the edit apply at all? The `old_string` appears exactly once, the
    file exists, the patch is well-formed.
  * **behaviour** — having applied, does the affected test suite pass?

Keeping them apart is the whole experiment. Form checks have failed three times here
(citations 33.3%, derivability 34.8%, prompt-only prediction at chance) and behaviour
checks have worked every time, and a single "checked" verdict would merge the two and
destroy the comparison before it is run.

## The threat model is not the one execution.py has

`validators/execution.py` runs a standalone program written by a model and protects the
**host** from it — Docker, `--network none`, non-root, read-only root, stdin delivery.

This is a different shape. The test command is the **customer's own**, and their agent
already runs it every few turns. What is untrusted here is the **edit**, and the thing
being protected is the **working tree**: a speculative change from a small local model
must never reach the files a developer is working in.

That difference argues for a lighter design, and it is not sufficient, because the edit
lands in a file the test suite then imports and executes. That is unreviewed
model-generated code running on a developer's machine — nobody sees these edits, which
is exactly what makes them different from the agent's. So the same rule applies as
everywhere else in this repository: **it refuses to run without a sandbox rather than
falling back to the host.** `allow_host_execution` exists, is off, and says what it costs.

## Copying is the expensive part

A naive tree copy pulls in `.git`, `.venv` and `node_modules` and turns a two-second
check into a two-minute one — which on the idle budget means the queue never drains.
The exclusions below are not an optimisation; without them this component does not work.
"""

from __future__ import annotations

import hashlib
import shutil
import subprocess
import tempfile
from dataclasses import dataclass
from enum import Enum
from pathlib import Path
from typing import Any

from .oracle import Action

DEFAULT_TIMEOUT_S = 120
IMAGE = "python:3.13-slim"

# Without these a check takes minutes and the idle queue never drains.
SKIP_DIRS = frozenset({
    ".git", ".venv", "venv", "node_modules", "__pycache__", ".pytest_cache",
    ".mypy_cache", ".ruff_cache", "dist", "build", ".tox", "target", ".next",
})
MAX_TREE_FILES = 20_000        # a repo larger than this is not a laptop-scale check


class Verdict(str, Enum):
    NOT_APPLICABLE = "not_applicable"        # not an edit; nothing to apply
    FORM_FAIL = "form_fail"                  # the edit does not apply
    FORM_PASS = "form_pass"                  # applies, but nothing was run
    BEHAVIOUR_PASS = "behaviour_pass"        # applied and the tests passed
    BEHAVIOUR_FAIL = "behaviour_fail"        # applied and the tests failed
    NO_SANDBOX = "no_sandbox"                # refused: infrastructure, not a result
    INFRASTRUCTURE_ERROR = "infrastructure_error"


@dataclass
class CheckResult:
    verdict: Verdict
    detail: str
    elapsed_ms: float = 0.0

    @property
    def is_infrastructure(self) -> bool:
        """Ours, not the model's. Must never be counted as a failed check.

        The gate treats runner errors as blocking for exactly this reason: a broken
        environment that reads as a bad model turns one afternoon's debugging into a
        product decision.
        """
        return self.verdict in (Verdict.NO_SANDBOX, Verdict.INFRASTRUCTURE_ERROR)


# ------------------------------------------------------------- the form check


def apply_edit(action: Action, tree: Path) -> tuple[bool, str]:
    """Apply one edit inside `tree`. Returns (applied, reason). Never escapes the tree.

    The path containment check is not paranoia about a hostile model so much as about a
    confused one: `../../etc/hosts` is a plausible thing for a small model to emit when
    it has lost track of the working directory, and the failure would be silent and
    outside the copy.
    """
    if action.tool not in ("Edit", "Write", "MultiEdit", "NotebookEdit"):
        return False, f"not an edit tool: {action.tool}"

    target = action.args.get("file_path") or action.args.get("path") or action.target
    if not target:
        return False, "no target path in the action"

    try:
        path = (tree / str(target)).resolve()
        path.relative_to(tree.resolve())
    except (ValueError, OSError):
        return False, f"path escapes the scratch tree: {target}"

    new = action.args.get("new_string")
    old = action.args.get("old_string")
    content = action.args.get("content")

    if content is not None and old is None:                     # Write
        try:
            path.parent.mkdir(parents=True, exist_ok=True)
            path.write_text(str(content), encoding="utf-8")
        except OSError as exc:
            return False, f"write failed: {exc}"
        return True, "written"

    if old is None or new is None:
        return False, "edit carries neither content nor an old/new pair"
    if not path.exists():
        return False, f"target does not exist: {target}"

    try:
        body = path.read_text(encoding="utf-8")
    except (OSError, UnicodeDecodeError) as exc:
        return False, f"unreadable: {exc}"

    occurrences = body.count(str(old))
    if occurrences == 0:
        return False, "old_string not found"
    if occurrences > 1:
        # Ambiguity is a form failure, not a coin flip. Picking one would make the
        # behaviour result depend on which we guessed.
        return False, f"old_string is ambiguous ({occurrences} matches)"

    try:
        path.write_text(body.replace(str(old), str(new), 1), encoding="utf-8")
    except OSError as exc:
        return False, f"write failed: {exc}"
    return True, "applied"


# -------------------------------------------------------------- the sandbox


def docker_available() -> bool:
    try:
        done = subprocess.run(["docker", "version", "--format", "{{.Server.Version}}"],
                              capture_output=True, timeout=20)
        return done.returncode == 0
    except (OSError, subprocess.SubprocessError):
        return False


def copy_tree(source: Path, dest: Path) -> int:
    """Copy a working tree, skipping everything that makes it slow. Returns file count.

    Refuses a destination inside the source. `rglob` would otherwise pick up the files it
    had just written and copy them again, building `working/working/working/...` until
    the path stops being representable -- observed as WinError 534 filling a temp
    directory Windows then could not delete. Production passes a tempdir so this has
    never fired there, but a `--repo` pointing at a parent of the scratch directory would
    do it on a customer's disk.
    """
    source, dest = Path(source).resolve(), Path(dest).resolve()
    if dest == source or source in dest.parents:
        raise ValueError(
            f"refusing to copy {source} into itself at {dest}: the copy would find its "
            f"own output and recurse")

    copied = 0
    for path in source.rglob("*"):
        parts = set(path.relative_to(source).parts)
        if parts & SKIP_DIRS:
            continue
        if path.is_dir():
            continue
        copied += 1
        if copied > MAX_TREE_FILES:
            raise RuntimeError(f"tree exceeds {MAX_TREE_FILES} files; not a laptop check")
        target = dest / path.relative_to(source)
        target.parent.mkdir(parents=True, exist_ok=True)
        try:
            shutil.copy2(path, target)
        except (OSError, shutil.Error):
            copied -= 1          # unreadable file: skip it, do not fail the whole check
    return copied


def project_dependencies(repo: Path) -> list[str]:
    """Runtime dependencies declared by the project under test."""
    import re

    config = Path(repo) / "evals" / "pyproject.toml"
    if not config.is_file():
        config = Path(repo) / "pyproject.toml"
    if not config.is_file():
        return []

    block = re.search(r"^dependencies = \[(.*?)\]", config.read_text(encoding="utf-8"),
                      re.S | re.M)
    if not block:
        return []
    return [line.strip().strip('",') for line in block.group(1).strip().splitlines()
            if line.strip().strip('",')]


def ensure_sandbox_image(repo: Path, *, timeout_s: int = 600) -> str:
    """An image that can actually run the project's suite, built once and reused.

    `python:3.13-slim` has neither pytest nor any of the project's dependencies, so a
    suite run inside it fails at import for every task -- an environment problem that
    would have read as "the model could not repair anything" across a whole cohort.

    The tag is derived from the dependency list, so changing the project's requirements
    builds a new image and leaves the old one alone. A run that reused a stale image
    would be testing against libraries the code no longer declares.

    The build needs network. The *test run* does not, and still gets `--network none`:
    that is the step executing code a model wrote.
    """
    deps = ["pytest"] + project_dependencies(repo)
    tag = "kerna-check:" + hashlib.sha256(
        " ".join(sorted(deps)).encode()).hexdigest()[:12]

    present = subprocess.run(["docker", "image", "inspect", tag],
                             capture_output=True, timeout=60)
    if present.returncode == 0:
        return tag

    dockerfile = "\n".join([
        f"FROM {IMAGE}",
        f"RUN pip install --no-cache-dir {' '.join(deps)}",
        "",
    ])
    build = subprocess.run(
        ["docker", "build", "-t", tag, "-"],
        input=dockerfile.encode(), capture_output=True, timeout=timeout_s,
    )
    if build.returncode != 0:
        raise RuntimeError(
            "could not build the sandbox image: "
            + (build.stderr or b"").decode("utf-8", "replace")[-300:])
    return tag


def verify_sandbox(repo: Path, command: list[str], *, timeout_s: int = 120) -> str:
    """Run the real command in the real image, and return what it printed.

    Raises with the reason if it cannot run.

    An import check is not enough. The previous preflight confirmed pytest and the
    project dependencies were importable and still missed the failure that mattered: a
    Windows `sys.executable` path was passed into a Linux container, every task exited
    127, and five of them were recorded as the model failing to repair code.

    The only check that catches that is executing the command that will actually be
    executed, in the container it will actually run in.
    """
    if not docker_available():
        raise RuntimeError("docker is not available")

    image = ensure_sandbox_image(repo)
    probe = [c for c in command]
    # `--version` rather than the suite: it exercises the interpreter and the runner
    # without needing the tree, so a failure here is unambiguously the environment.
    if probe and probe[-1].startswith("evals"):
        probe = probe[:-1]
    probe = probe + ["--version"]

    done = subprocess.run(
        ["docker", "run", "--rm", "--network", "none", image, *probe],
        capture_output=True, timeout=timeout_s,
    )
    output = ((done.stdout or b"") + (done.stderr or b"")).decode("utf-8", "replace")
    if done.returncode != 0:
        raise RuntimeError(
            f"the sandbox cannot run `{' '.join(probe)}` (exit {done.returncode}): "
            f"{output.strip()[-200:]}")
    return output.strip()


def _mount(tree: Path) -> str:
    """A volume argument Docker accepts on every host.

    Windows paths arrive with backslashes and a drive-letter colon, which is exactly the
    shape Docker's `--volume` parser is worst at. Forward slashes work on all three
    platforms.
    """
    return f"{Path(tree).resolve().as_posix()}:/w"


def run_tests(tree: Path, command: list[str], *, timeout_s: int = DEFAULT_TIMEOUT_S,
              use_docker: bool = True, image: str | None = None) -> tuple[bool, str]:
    """Run the suite inside `tree`. Returns (passed, detail)."""
    if use_docker:
        args = [
            "docker", "run", "--rm",
            "--network", "none",                 # no egress from an unreviewed edit
            "--memory", "1g", "--pids-limit", "256",
            "--volume", _mount(tree), "--workdir", "/w",
            image or IMAGE, *command,
        ]
    else:
        args = command

    try:
        done = subprocess.run(args, capture_output=True, timeout=timeout_s,
                              cwd=None if use_docker else str(tree))
    except subprocess.TimeoutExpired as exc:
        # Not a failed test. A timeout is ambiguous -- our command could be too broad, or
        # the model's edit could have hung -- and this project's entire history is
        # infrastructure being read as a model result. Erring toward ERROR undercounts
        # model failures, which is the safe direction; erring the other way produced a
        # cohort of five "failed repairs" that were all a 120s budget.
        raise RuntimeError(f"the suite did not finish within {timeout_s}s") from exc
    except (OSError, subprocess.SubprocessError) as exc:
        raise RuntimeError(f"could not run the suite: {exc}") from exc

    tail = (done.stdout or b"").decode("utf-8", "replace")[-400:]

    # 125/126/127 are the container failing to run the command, not the command failing.
    # Measured the hard way: a Windows `sys.executable` path was passed into a Linux
    # container, every task exited 127, and five of them were recorded as the model
    # failing to repair code. A broken sandbox must never be able to masquerade as a
    # terrible model.
    if use_docker and done.returncode in (125, 126, 127):
        raise RuntimeError(
            f"the sandbox could not run the command (exit {done.returncode}): "
            f"{(done.stderr or b'').decode('utf-8', 'replace')[-200:] or tail[-200:]}")

    return done.returncode == 0, tail.strip() or f"exit {done.returncode}"


# ------------------------------------------------------------ the whole check


def check_action(action: Action | None, repo_root: Path, test_command: list[str], *,
                 timeout_s: int = DEFAULT_TIMEOUT_S,
                 allow_host_execution: bool = False,
                 clock=None) -> CheckResult:
    """Apply a proposed edit to a throwaway copy and run the tests.

    `allow_host_execution` runs the suite directly instead of in a container. It is off by
    default and should stay off: the edit being tested was written by a model and reviewed
    by nobody, and the suite imports it.
    """
    import time
    clock = clock or time.monotonic
    started = clock()

    def done(verdict: Verdict, detail: str) -> CheckResult:
        return CheckResult(verdict, detail, round((clock() - started) * 1000.0, 1))

    if action is None:
        return done(Verdict.NOT_APPLICABLE, "no action to check")
    if action.tool not in ("Edit", "Write", "MultiEdit", "NotebookEdit"):
        return done(Verdict.NOT_APPLICABLE, f"{action.tool} produces nothing to test")

    use_docker = not allow_host_execution
    if use_docker and not docker_available():
        # Refused, not failed. An environment problem must never read as a bad model.
        return done(Verdict.NO_SANDBOX,
                    "docker is not available; refusing to run an unreviewed edit on the host")

    scratch: str | None = None
    try:
        scratch = tempfile.mkdtemp(prefix="m0-check-")
        tree = Path(scratch)
        copy_tree(repo_root, tree)

        applied, reason = apply_edit(action, tree)
        if not applied:
            return done(Verdict.FORM_FAIL, reason)

        passed, detail = run_tests(tree, test_command, timeout_s=timeout_s,
                                   use_docker=use_docker)
        return done(Verdict.BEHAVIOUR_PASS if passed else Verdict.BEHAVIOUR_FAIL, detail)
    except Exception as exc:  # noqa: BLE001
        return done(Verdict.INFRASTRUCTURE_ERROR, f"{type(exc).__name__}: {exc}")
    finally:
        if scratch:
            shutil.rmtree(scratch, ignore_errors=True)


def form_only(action: Action | None, repo_root: Path) -> CheckResult:
    """The cheap arm: does it apply? Nothing is executed, so no sandbox is needed.

    This is the arm predicted to fail. It is run anyway, for the same reason Arm B was
    run in Phase 0 — a boundary claimed without its control is an argument, not a finding.
    """
    import time
    started = time.monotonic()

    def done(verdict: Verdict, detail: str) -> CheckResult:
        return CheckResult(verdict, detail, round((time.monotonic() - started) * 1000.0, 1))

    if action is None or action.tool not in ("Edit", "Write", "MultiEdit", "NotebookEdit"):
        return done(Verdict.NOT_APPLICABLE, "no edit to apply")

    scratch: str | None = None
    try:
        scratch = tempfile.mkdtemp(prefix="m0-form-")
        tree = Path(scratch)
        copy_tree(repo_root, tree)
        applied, reason = apply_edit(action, tree)
        return done(Verdict.FORM_PASS if applied else Verdict.FORM_FAIL, reason)
    except Exception as exc:  # noqa: BLE001
        return done(Verdict.INFRASTRUCTURE_ERROR, f"{type(exc).__name__}: {exc}")
    finally:
        if scratch:
            shutil.rmtree(scratch, ignore_errors=True)


def discover_test_command(repo_root: Path) -> list[str] | None:
    """The customer's own gate, found rather than configured.

    Inheriting the existing gate is the cascade's premise (034), so asking a customer to
    declare their test command would already be a small failure of it. Returns None when
    it cannot be found, which is honest and leaves the behaviour arm unmeasured for that
    repository rather than guessed at.
    """
    if (repo_root / "pyproject.toml").exists() or (repo_root / "pytest.ini").exists():
        return ["python", "-m", "pytest", "-q"]
    if (repo_root / "tests").is_dir() or (repo_root / "test").is_dir():
        return ["python", "-m", "pytest", "-q"]
    if (repo_root / "package.json").exists():
        return ["npm", "test", "--silent"]
    if (repo_root / "Cargo.toml").exists():
        return ["cargo", "test", "--quiet"]
    if (repo_root / "go.mod").exists():
        return ["go", "test", "./..."]
    return None
