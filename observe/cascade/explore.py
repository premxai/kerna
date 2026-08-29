"""Attempt locally off the critical path, for evidence only (P1.2-B).

`Route.EXPLORE` has existed since the dispatcher was written and has never explored
anything. It labels the path `explore->cloud` and forwards, so `attempt_local` is never
called and no evidence is gathered. This is the half that makes it a measurement.

Decision 037: the first MVP **serves nothing locally.** Every request is answered by the
cloud, exactly as today. Afterwards, on the idle budget, eligible turns are attempted
locally and the result is compared against the cloud's own answer. The customer cannot be
hurt because their answer never came from us, which is the property that gets this
installed somewhere.

## Everything here is designed around one rule

**A served request must be untouchable.** Not "unlikely to be affected" — structurally
incapable of being affected. Three consequences, and each is a thing this module refuses
to do rather than a thing it does carefully:

  * **`submit` never blocks.** A full queue drops, and counts the drop. A bounded queue
    that blocks is a latency bug with a slow fuse; an unbounded one is a memory leak on
    somebody's laptop.
  * **The worker cannot raise into anything.** It catches everything, records it, and
    continues. A local model server that dies mid-attempt is an ordinary Tuesday.
  * **Work happens only when the developer is not working.** Decision 034: evidence is
    paid for with electrons, never with anyone's time.

## What "idle" means, and why it is not a system probe

Idle is **no request served in the last `idle_after_s` seconds**, which we know for free
because every request passes through us. Probing CPU or GPU would be more precise and
would be measuring the wrong thing: a developer reading a diff registers as an idle
machine, and the moment their agent resumes we are competing with it for the GPU it needs.
The request stream is the only signal that tracks the human rather than the hardware.

## Staleness

An item that has waited past its TTL is dropped and counted. It is not wrong to explore a
stale prompt — the comparison against the cloud answer is still valid — but a queue
quietly working through an hour-old backlog is spending a laptop's power on evidence
nobody is waiting for, and the drop count is more honest than the result would be.
"""

from __future__ import annotations

import queue
import threading
import time
from dataclasses import dataclass, field
from pathlib import Path
from typing import Any, Callable

from .check import CheckResult, Verdict, check_action, form_only
from .repetition import repetition_row
from .semantic import compare_semantic
from .oracle import (
    Agreement,
    compare,
    extract_action,
    ABSTAINED,
    extract_local_action,
    local_response_diagnostics,
)
from .tool_policy import (
    cloud_action_available,
    offered_tool_names,
    tool_metrics,
)

DEFAULT_QUEUE = 32
DEFAULT_TTL_S = 900.0          # 15 minutes
DEFAULT_IDLE_AFTER_S = 20.0


class ContextIneligible(Exception):
    """The full customer turn cannot fit this local model's context window.

    This is machine/model eligibility evidence, not an inference failure and not broken
    infrastructure. The model never attempted the task.
    """


@dataclass
class ExploreItem:
    """One turn to attempt locally, with the cloud's answer to compare against."""

    payload: dict[str, Any]
    cloud_answer: dict[str, Any] | None
    task_class: str
    queued_at: float
    edits_since_check: int = 0
    turn: str | None = None

    @property
    def cloud_action(self):
        """The cloud's proposed action, or None when it answered in prose.

        Read lazily rather than stored, so a queued item holds the response it was given
        and nothing derived from it -- one representation of the customer's output in
        memory, not two.
        """
        return extract_action(self.cloud_answer)


@dataclass
class ExploreStats:
    submitted: int = 0
    dropped_full: int = 0
    dropped_stale: int = 0
    attempted: int = 0
    converged: int = 0
    context_ineligible: int = 0
    action_space_ineligible: int = 0
    # The attempt completed and the comparison is still invalid: the dialect
    # translation could not carry part of what the cloud saw, so a disagreement
    # cannot be attributed to the model rather than to us.
    translation_ineligible: int = 0
    failed: int = 0
    agreement: dict[str, int] = field(
        default_factory=lambda: {a.value: 0 for a in Agreement})

    def as_dict(self) -> dict[str, Any]:
        return {
            "submitted": self.submitted,
            "dropped_full": self.dropped_full,
            "dropped_stale": self.dropped_stale,
            "attempted": self.attempted,
            "converged": self.converged,
            "context_ineligible": self.context_ineligible,
            "action_space_ineligible": self.action_space_ineligible,
            "translation_ineligible": self.translation_ineligible,
            "failed": self.failed,
            # Named `agreement` at every layer. There is no key in this product called
            # `accuracy`, because nothing here measures one.
            "agreement": dict(self.agreement),
        }


@dataclass
class Explorer:
    """A bounded queue and one worker, neither of which can reach a served request.

    `attempt_local` returns the local answer, or `None` when it did not converge.
    `record` receives one metadata row per attempt and is the only output — this class
    deliberately has no return value anyone can wait on.
    """

    attempt_local: Callable[[dict[str, Any]], dict[str, Any] | None]
    record: Callable[[dict[str, Any]], None] = lambda row: None
    tool_policy: str = "full"
    local_tool_choice: str = "auto"
    local_max_tokens: int | None = None
    local_tool_prompt: str = "none"
    # Constrained decoding is a cohort axis, not a setting. A rate earned with prose made
    # unrepresentable is a different measurement from one earned without, and pooling them
    # would repeat the error `class_version` exists to prevent.
    local_decoding: str = "none"
    # How the local model is shown its tools. A separate axis from the decoding
    # because it changes what the model was offered, not how it answers -- and
    # because grammar-v1 is only reachable through textual-v1 at all, hiding it
    # inside the decoding label would make an infrastructure workaround
    # indistinguishable from the constraint whose value it is meant to measure.
    local_tool_transport: str = "native"
    # Which model produced these rows. Every local request says `model: "local"` because
    # the server has one loaded, so without this two models' rows are indistinguishable
    # and pool -- the unlabelled-cohort error, applied to the thing the experiment is
    # about.
    local_model: str = "unknown"
    local_model_source: str = "unknown"
    # Record the arguments of each proposed action, not just the tool names. Off by
    # default because a target is usually a path into the customer's repository; tool
    # names are identifiers from a menu we chose and are always recorded.
    record_action_args: bool = False
    # Both arms of the check experiment, or neither. `repo_root` is what makes this a
    # measurement rather than a comparison against the cloud alone -- without it the rows
    # carry `agreement` and no `check_verdict`, and evidence.py correctly reports the
    # check arms as unmeasured.
    repo_root: Path | None = None
    test_command: list[str] | None = None
    allow_host_execution: bool = False
    queue_size: int = DEFAULT_QUEUE
    ttl_s: float = DEFAULT_TTL_S
    idle_after_s: float = DEFAULT_IDLE_AFTER_S
    clock: Callable[[], float] = time.monotonic

    _queue: queue.Queue = field(init=False)
    _stats: ExploreStats = field(default_factory=ExploreStats)
    _last_request_at: float = field(init=False, default=0.0)
    _lock: threading.Lock = field(default_factory=threading.Lock)
    _worker: threading.Thread | None = field(init=False, default=None)
    _stopping: threading.Event = field(default_factory=threading.Event)

    def __post_init__(self) -> None:
        self._queue = queue.Queue(maxsize=self.queue_size)
        self._last_request_at = self.clock()

    # ------------------------------------------------------------ the signal

    def note_request(self) -> None:
        """Called on every served request. The only thing that defines 'busy'."""
        with self._lock:
            self._last_request_at = self.clock()

    def is_idle(self) -> bool:
        with self._lock:
            return (self.clock() - self._last_request_at) >= self.idle_after_s

    # ------------------------------------------------------------- the queue

    def submit(self, item: ExploreItem) -> bool:
        """Queue one turn. Never blocks; returns whether it was accepted.

        Called from the request path, so the only acceptable behaviours are "accept
        instantly" and "drop instantly". `put_nowait` gives exactly those two.
        """
        try:
            self._queue.put_nowait(item)
        except queue.Full:
            self._stats.dropped_full += 1
            return False
        self._stats.submitted += 1
        return True

    @property
    def stats(self) -> ExploreStats:
        return self._stats

    @property
    def depth(self) -> int:
        return self._queue.qsize()

    # ------------------------------------------------------------ the worker

    def drain_once(self, *, block_s: float = 0.0) -> bool:
        """Process at most one item. Returns whether anything was attempted.

        Separated from the thread so the whole policy — staleness, the idle gate, failure
        handling — is testable without timing or concurrency. A background worker whose
        behaviour can only be observed by waiting is a background worker nobody verifies.
        """
        try:
            item = self._queue.get(timeout=block_s) if block_s else self._queue.get_nowait()
        except queue.Empty:
            return False

        try:
            if (self.clock() - item.queued_at) > self.ttl_s:
                self._stats.dropped_stale += 1
                return False
            if not self.is_idle():
                # Put it back and stop. Attempting now would take the GPU from the agent
                # that is actively using it, which is the one cost this must never impose.
                self.submit_front(item)
                return False
            self._attempt(item)
            return True
        finally:
            self._queue.task_done()

    def submit_front(self, item: ExploreItem) -> None:
        """Return an item to the queue, dropping it if that is now full.

        Dropping a deferred item is correct: the queue being full means newer evidence is
        already waiting, and newer evidence is better evidence.
        """
        try:
            self._queue.put_nowait(item)
        except queue.Full:
            self._stats.dropped_full += 1

    def _attempt(self, item: ExploreItem) -> None:
        """Run one local attempt. Cannot raise ? that is the whole contract."""
        started = self.clock()
        row: dict[str, Any] = {
            "record": "explore",
            "turn": item.turn,
            "task_class": item.task_class,
            "tool_policy": self.tool_policy,
            "local_tool_choice": self.local_tool_choice,
            "local_max_tokens": self.local_max_tokens,
            "local_tool_prompt": self.local_tool_prompt,
            "local_decoding": self.local_decoding,
            "local_tool_transport": self.local_tool_transport,
            "local_model": self.local_model,
            "local_model_source": self.local_model_source,
            "edits_since_check": item.edits_since_check,
            # How deep into the conversation this turn sits. Recorded because
            # `translation_ineligible` is not randomly distributed: thinking blocks
            # accumulate as a session runs, so lossy turns cluster late and the clean
            # cohort skews early -- toward opening reads and searches and away from the
            # edits that follow several observations. That is a sampling bias in favour
            # of the local model, and this is the field that makes it checkable rather
            # than assumed absent.
            "n_messages": len(item.payload.get("messages") or []),
            "waited_ms": round((started - item.queued_at) * 1000.0, 1),
        }

        cloud_action = item.cloud_action
        cloud_tool = cloud_action.tool if cloud_action is not None else None

        available = cloud_action_available(
            item.payload,
            cloud_tool,
            self.tool_policy,
        )

        row["tool_policy"] = self.tool_policy
        row["cloud_action_available"] = available
        row.update(tool_metrics(item.payload, self.tool_policy))

        if available is False:
            # Claude chose a tool that the local model was never offered.
            # This turn cannot measure agreement under this tool-policy cohort.
            self._stats.action_space_ineligible += 1
            row["outcome"] = "action_space_ineligible"
            row["agreement"] = Agreement.UNSCORABLE.value
            self._stats.agreement[Agreement.UNSCORABLE.value] += 1

        else:
            try:
                local = self.attempt_local(item.payload)
                self._stats.attempted += 1

                if local is None:
                    row["outcome"] = "did_not_converge"

                else:
                    self._stats.converged += 1
                    row["outcome"] = "converged"

                    row.update(local_response_diagnostics(local))

                    # Which endpoint actually served the shadow, and what the dialect
                    # translation could not carry across. Read before anything is
                    # scored, because it decides whether scoring is legitimate at all.
                    kerna = local.get("_kerna")
                    if isinstance(kerna, dict):
                        row["local_dialect"] = kerna.get("local_dialect")
                        row["local_translation_dropped"] = kerna.get(
                            "translation_dropped"
                        ) or []
                        # Configured axis vs what this turn actually carried. They
                        # diverge when no tool survived the filter, and a row
                        # claiming a catalog it never sent is the same lie the
                        # ignored grammar told.
                        row["textual_catalog_sent"] = bool(
                            kerna.get("textual_catalog_sent")
                        )
                        # The configured cohort says `required`; under textual-v1 the
                        # field is never sent. Recording both is what stops the label
                        # from claiming a mechanism that was not in force.
                        row["tool_choice_sent"] = bool(kerna.get("tool_choice_sent"))

                    local_action, local_action_format = extract_local_action(
                        local,
                        allowed_tools=offered_tool_names(
                            item.payload,
                            self.tool_policy,
                        ),
                        allow_tools_wrapper=(
                            self.local_tool_choice == "required"
                        ),
                        # Bare JSON is an action only where a grammar guaranteed it.
                        allow_grammar_json=self.local_decoding in (
                            "grammar-v1", "grammar-v2"),
                        allow_abstain=(self.local_decoding == "grammar-v2"),
                    )
                    row["local_action_format"] = local_action_format

                    # v2 lets the model decline. "Chose not to act" and "produced nothing
                    # readable" are different findings, and v1 could only express the
                    # second -- which is how a search request came back as a Write with
                    # the answer stuffed into the file content.
                    local_abstained = local_action is ABSTAINED
                    if self.local_decoding == "grammar-v2":
                        row["local_abstained"] = local_abstained
                    if local_abstained:
                        local_action = None

                    if row.get("local_translation_dropped"):
                        # The local model was shown less than the cloud was, so a
                        # disagreement cannot be attributed. It might be the model's
                        # judgement or it might be our translator, and nothing in the
                        # row can separate the two.
                        #
                        # This is an eligibility outcome, not a result: the attempt
                        # converged and the plumbing worked, but the *experiment* was
                        # invalid before the model spoke. It joins `context_ineligible`
                        # and `action_space_ineligible` outside every rate, and unlike
                        # them it is not recorded as `unscorable` either -- pooling it
                        # there would hide it among turns where the cloud simply had no
                        # action to compare, which is a different fact entirely.
                        #
                        # The check is skipped for the same reason the comparison is: an
                        # action produced from less context is a handicapped attempt,
                        # and its verdict would understate local capability exactly as
                        # the agreement rate would.
                        self._stats.translation_ineligible += 1
                        row["outcome"] = "translation_ineligible"
                        row["reason"] = "dropped_context"

                    else:
                        verdict = compare(local_action, cloud_action)

                        # Which tool each side chose. A `different_action` verdict with
                        # no evidence attached cannot be interrogated, and the first
                        # real cohort landed on exactly that question: 6 of 6 differed,
                        # and nothing in the log said whether that was a weak model or
                        # an over-strict exact-match metric.
                        #
                        # Tool NAMES only. They are identifiers drawn from a closed menu
                        # we chose, not customer content, so 005 is untouched. Arguments
                        # can carry file paths and are opt-in below.
                        if cloud_action is not None:
                            row["cloud_tool"] = cloud_action.tool
                        if local_action is not None:
                            row["local_tool"] = local_action.tool

                        if self.record_action_args:
                            # Off by default and worth the friction: these carry paths
                            # and shell commands from the customer's repository.
                            #
                            # The WHOLE argument object, not just `target`. `_target_of`
                            # returns the first key it recognises, so a Grep carrying
                            # {"pattern": "sidecar", "path": "./"} logged `./` and threw
                            # the pattern away -- and the pattern is the entire content
                            # of the decision. Two rows of the first analysed cohort
                            # could not be judged because of it.
                            if cloud_action is not None:
                                row["cloud_target"] = cloud_action.target
                                row["cloud_args"] = cloud_action.args
                            if local_action is not None:
                                row["local_target"] = local_action.target
                                row["local_args"] = local_action.args

                        # Declining where the cloud acted IS a different decision, so it
                        # scores as one rather than vanishing into `unscorable`. The
                        # `local_abstained` flag keeps the two distinguishable, because
                        # "chose a different tool" and "chose no tool" are not the same
                        # finding.
                        if local_abstained and cloud_action is not None:
                            verdict = Agreement.DIFFERENT_ACTION

                        row["agreement"] = verdict.value
                        self._stats.agreement[verdict.value] += 1

                        # Recorded beside the exact verdict, never instead of it. Exact
                        # match asks whether a 7B model reproduces Claude's exact button
                        # presses; the routing question is whether it chose an
                        # equivalent way to do the same thing, and the first cohort
                        # returned 10 of 10 `different_action` where three pairs were
                        # the same search with a different tool.
                        #
                        # Components, not a conclusion: a semantic matcher is a knob
                        # that makes our own number go up, and a threshold baked into
                        # the data cannot be argued with later.
                        # Both sides against the same context. Claude re-greps a file
                        # after an edit and that is correct behaviour, so only the gap
                        # between the two is evidence -- measuring the local model alone
                        # would turn a normal habit into a pathology.
                        row.update(repetition_row(
                            item.payload, cloud_action, local_action))

                        if cloud_action is not None and local_action is not None:
                            row.update(compare_semantic(
                                cloud_action.tool, cloud_action.args,
                                local_action.tool, local_action.args,
                            ).as_dict())
                        row.update(self._check(local_action))

            except ContextIneligible:
                # The model never received a runnable task. Context eligibility is
                # machine/model evidence, not model-quality or infrastructure evidence.
                self._stats.context_ineligible += 1
                row["outcome"] = "context_ineligible"
                row["reason"] = "context_window"

            except Exception as exc:  # noqa: BLE001
                self._stats.failed += 1
                row["outcome"] = "infrastructure_failure"
                row["error"] = type(exc).__name__
                # The message, not only the class. `RuntimeError` alone is what a whole
                # Ollama investigation had to work from: the interceptor raises
                # `local server returned {status}: {body}` and this line discarded the
                # status and the body, leaving a bare class name to diagnose a transport
                # rejection that happened in 200ms.
                #
                # Safe to keep: this log never leaves the device, and the text is the
                # *local* server's own error rather than anything the customer sent.
                row["error_detail"] = str(exc)[:400]

        row["elapsed_ms"] = round((self.clock() - started) * 1000.0, 1)

        try:
            self.record(row)
        except Exception:  # noqa: BLE001
            # Recording evidence must never be able to kill the background worker.
            pass


    def _check(self, action) -> dict[str, Any]:
        """Run both arms over one proposed action. Never raises.

        Both, on the same action, or the comparison is between two populations rather
        than two checks -- which is the shape of mistake that made a self-authored corpus
        look like a capability measurement. Phase 0's arms differed only in the check;
        so do these.
        """
        if self.repo_root is None or self.test_command is None:
            return {}
        out: dict[str, Any] = {}
        try:
            form = form_only(action, self.repo_root)
            out["form_verdict"] = form.verdict.value

            behaviour = check_action(action, self.repo_root, self.test_command,
                                     allow_host_execution=self.allow_host_execution)
            out["check_verdict"] = behaviour.verdict.value
            out["check_ms"] = behaviour.elapsed_ms
            if behaviour.is_infrastructure:
                # Ours, not the model's. Recorded so a run full of these is visible as a
                # broken environment rather than read as a model that cannot write code.
                out["check_infrastructure"] = True
                out["check_detail"] = behaviour.detail
        except Exception as exc:  # noqa: BLE001
            out["check_verdict"] = Verdict.INFRASTRUCTURE_ERROR.value
            out["check_infrastructure"] = True
            out["check_detail"] = type(exc).__name__
        return out

    # ----------------------------------------------------------- the thread

    def start(self) -> None:
        if self._worker is not None:
            return
        self._stopping.clear()
        self._worker = threading.Thread(target=self._run, name="explorer", daemon=True)
        self._worker.start()

    def _run(self) -> None:
        while not self._stopping.is_set():
            try:
                if not self.drain_once(block_s=1.0):
                    # Nothing done: either empty, stale, or the developer is working.
                    self._stopping.wait(1.0)
            except Exception:  # noqa: BLE001
                self._stopping.wait(1.0)

    def close(self, timeout: float = 2.0) -> None:
        self._stopping.set()
        worker, self._worker = self._worker, None
        if worker is not None:
            worker.join(timeout=timeout)
