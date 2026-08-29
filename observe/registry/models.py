"""The model registry — what may be installed, on what, and on whose authority.

A catalogue of local models is not itself worth much; Ollama has one and it is free. What
matters is the two rules wrapped around it, both of which came out of measurements in this
project rather than from taste.

## Rule 1 — a benchmark score may choose a candidate, never authorise routing

Measured, by accident, on two models across two corpora:

| | Qwen3-8B | Mistral-7B |
|---|---|---|
| our authored corpus | 78% | **91%** |
| FinQA, real filings | **23%** | 8% |

**The ranking inverted.** Selecting on the first number ships the model that is three times
worse on real work. Published coding benchmarks are worse still, because every vendor tunes
to them.

So `prior` and `earned` are different types here, and the difference is enforced rather
than documented. A `PriorScore` narrows the shortlist. Only an `EarnedScore` — agreement
with the cloud, on the customer's own tasks, on that machine, carrying a Wilson lower bound
(INV-13) — may open the valve. `may_route()` will not accept a prior, and there is no
conversion between them.

## Rule 2 — the licence gate is structural

Decision 014 permits permissive licences only. This is where a mistake becomes a legal
problem rather than a bad answer, so `Licence` carries `permits_production` and
`installable()` filters on it. Two entries below exist specifically to be excluded:

  * **Codestral 22B** — MNPL, explicitly *non-production*. Attractive on benchmarks, and
    unusable inside a customer's workflow.
  * **StarCoder2 15B** — BigCode OpenRAIL-M. Commercial use is allowed but carries
    behavioural use restrictions, so it is admissible only with review.

Recording them as rejected is deliberate. An empty catalogue teaches nothing; a catalogue
that shows *why* a tempting model was refused stops it being re-proposed every quarter.

## Sizing

`min_free_vram_gb` is the **free** requirement, never installed (§6 of the playbook). The
reference machine reports 4,095 MiB total against 3,303 MiB free — a preflight reading
"total" passes every lab test and runs out of memory in the field.
"""

from __future__ import annotations

from dataclasses import dataclass
from enum import Enum


class Licence(Enum):
    """Model licences, classified by whether a customer may run them in production."""

    APACHE_2 = ("Apache-2.0", True)
    MIT = ("MIT", True)
    OPENRAIL_M = ("BigCode OpenRAIL-M", False)   # commercial, but behavioural restrictions
    MNPL = ("Mistral MNPL (non-production)", False)
    GEMMA = ("Gemma Terms of Use", False)        # use restrictions; review before shipping
    UNVERIFIED = ("unverified", False)

    def __init__(self, label: str, permits_production: bool) -> None:
        self.label = label
        self.permits_production = permits_production


class Role(Enum):
    """What a model is for. Roles are not interchangeable and must not be compared."""

    CODE_GENERATION = "code-generation"
    EMBEDDING = "embedding"
    GENERAL = "general"


@dataclass(frozen=True)
class PriorScore:
    """A published benchmark figure. Chooses candidates; authorises nothing.

    Deliberately a distinct type from `EarnedScore` so the two cannot be mixed up by a
    caller in a hurry. Contamination makes the absolute value unsafe as a capability
    claim — only the *ordering* between models measured on the same task set carries
    information, because contamination shifts both sides together.
    """

    benchmark: str
    value: float
    source: str

    def __post_init__(self) -> None:
        if not 0.0 <= self.value <= 1.0:
            raise ValueError("prior scores are proportions")


@dataclass(frozen=True)
class EarnedScore:
    """Agreement with the cloud, on this customer's tasks, on this hardware.

    The only score that may promote a task class (Decision 033). Carries its own Wilson
    lower bound because a perfect small sample never clears a bar (INV-13).
    """

    task_class: str
    agreements: int
    trials: int

    @property
    def rate(self) -> float:
        return self.agreements / self.trials if self.trials else 0.0

    @property
    def lower_bound(self) -> float:
        from ..cascade.interval import wilson_lower_bound

        return wilson_lower_bound(self.agreements, self.trials)

    def clears(self, bar: float) -> bool:
        return self.lower_bound >= bar


@dataclass(frozen=True)
class ModelCard:
    name: str
    role: Role
    licence: Licence
    params_b: float
    quantisation: str
    file_size_gb: float
    # Free VRAM required to hold the whole model plus a working KV cache. Below this a
    # machine can still run it partially offloaded -- measured at 6.1 -> 6.9 tok/s, a 13%
    # gain, so "it fits partially" is not a meaningful capability.
    min_free_vram_gb: float
    context_tokens: int
    priors: tuple[PriorScore, ...] = ()
    note: str = ""
    # Where the weights actually live. None means "we have not verified a download for
    # this entry", and the installer refuses to guess: a plausible-looking Hugging Face
    # path that 401s or serves a different quantisation would install the wrong thing
    # silently. Verified 20 Aug 2026 against the HF API, licence included.
    hf_repo: str | None = None
    hf_file: str | None = None

    @property
    def downloadable(self) -> bool:
        return bool(self.hf_repo and self.hf_file) and self.installable

    @property
    def download_url(self) -> str | None:
        if not self.downloadable:
            return None
        return f"https://huggingface.co/{self.hf_repo}/resolve/main/{self.hf_file}"

    @property
    def installable(self) -> bool:
        """Decision 014: permissive licences only."""
        return self.licence.permits_production

    def fits(self, free_vram_gb: float) -> bool:
        return free_vram_gb >= self.min_free_vram_gb

    def prior(self, benchmark: str) -> PriorScore | None:
        return next((p for p in self.priors if p.benchmark == benchmark), None)


def may_route(model: ModelCard, earned: EarnedScore, *, bar: float = 0.95) -> bool:
    """Whether this model may serve this task class locally.

    Takes an `EarnedScore` and nothing else. A `PriorScore` will not type-check here, and
    that is the entire point: no benchmark figure, however good, can authorise routing.
    """
    if not isinstance(earned, EarnedScore):  # pragma: no cover - guards a real mistake
        raise TypeError("routing requires an EarnedScore; a benchmark prior is not evidence")
    return model.installable and earned.clears(bar)


# --------------------------------------------------------------------- catalogue

SWE_BENCH = "SWE-bench Verified"

CATALOGUE: tuple[ModelCard, ...] = (
    # -- admissible ---------------------------------------------------------------
    ModelCard(
        name="qwen2.5-coder-7b",
        role=Role.CODE_GENERATION,
        licence=Licence.APACHE_2,
        params_b=7.0,
        quantisation="Q4_K_M",
        file_size_gb=4.7,
        min_free_vram_gb=6.0,
        context_tokens=32_768,
        note="Entry tier. The smallest coding model worth installing.",
        hf_repo="Qwen/Qwen2.5-Coder-7B-Instruct-GGUF",
        hf_file="qwen2.5-coder-7b-instruct-q4_k_m.gguf",
    ),
    ModelCard(
        name="qwen2.5-coder-14b",
        role=Role.CODE_GENERATION,
        licence=Licence.APACHE_2,
        params_b=14.0,
        quantisation="Q4_K_M",
        file_size_gb=9.0,
        min_free_vram_gb=11.0,
        context_tokens=32_768,
        hf_repo="Qwen/Qwen2.5-Coder-14B-Instruct-GGUF",
        hf_file="qwen2.5-coder-14b-instruct-q4_k_m.gguf",
    ),
    ModelCard(
        name="devstral-small-24b",
        role=Role.CODE_GENERATION,
        licence=Licence.APACHE_2,
        params_b=24.0,
        quantisation="Q4_K_M",
        file_size_gb=14.0,
        min_free_vram_gb=16.0,
        context_tokens=128_000,
        priors=(PriorScore(SWE_BENCH, 0.468, "vendor-reported, contaminated"),),
        note="Agentic-coding tuned. Note Devstral is Apache-2.0 while Codestral is not.",
        hf_repo="mistralai/Devstral-Small-2507_gguf",
        hf_file="Devstral-Small-2507-Q4_K_M.gguf",
    ),
    ModelCard(
        name="qwen3-coder-30b-a3b",
        role=Role.CODE_GENERATION,
        licence=Licence.APACHE_2,
        params_b=30.0,
        quantisation="Q4_K_M",
        file_size_gb=19.0,
        min_free_vram_gb=21.0,
        context_tokens=256_000,
        note=(
            "Mixture-of-experts, ~3.3B active. Unified-memory machines only, in "
            "practice. NO VERIFIED DOWNLOAD: the obvious Hugging Face path returns "
            "401, so the installer will not offer it rather than guess at a repo."
        ),
    ),
    ModelCard(
        name="bge-small-en-v1.5",
        role=Role.EMBEDDING,
        licence=Licence.MIT,
        params_b=0.033,
        quantisation="F16",
        file_size_gb=0.067,
        min_free_vram_gb=0.3,
        context_tokens=512,
        note=(
            "Measured here: 4.3 ms/query, 1.72 s cold start, +10-14 pts over BM25 on prose "
            "and level with it on table rows. Runs on any machine; carries the whole "
            "context-offload path."
        ),
        hf_repo="CompendiumLabs/bge-small-en-v1.5-gguf",
        hf_file="bge-small-en-v1.5-f16.gguf",
    ),
    # -- recorded as refused, so they stop being re-proposed -----------------------
    ModelCard(
        name="codestral-22b",
        role=Role.CODE_GENERATION,
        licence=Licence.MNPL,
        params_b=22.0,
        quantisation="Q4_K_M",
        file_size_gb=13.0,
        min_free_vram_gb=15.0,
        context_tokens=32_768,
        note=(
            "REFUSED. MNPL is explicitly non-production: fine to evaluate, not fine inside "
            "a customer workflow. Strong on benchmarks, which is exactly why it needs to be "
            "on this list rather than absent from it."
        ),
    ),
    ModelCard(
        name="starcoder2-15b",
        role=Role.CODE_GENERATION,
        licence=Licence.OPENRAIL_M,
        params_b=15.0,
        quantisation="Q4_K_M",
        file_size_gb=9.0,
        min_free_vram_gb=11.0,
        context_tokens=16_384,
        note=(
            "REFUSED pending review. OpenRAIL-M permits commercial use but attaches "
            "behavioural use restrictions, which a customer would inherit."
        ),
    ),
)


def installable(role: Role | None = None) -> list[ModelCard]:
    """Every model a customer may actually run, optionally filtered by role."""
    return [
        m for m in CATALOGUE
        if m.installable and (role is None or m.role is role)
    ]


def refused() -> list[ModelCard]:
    """Models excluded by licence, with the reason. Kept visible on purpose."""
    return [m for m in CATALOGUE if not m.installable]


def candidates_for(free_vram_gb: float, role: Role = Role.CODE_GENERATION) -> list[ModelCard]:
    """What this machine could run, largest first.

    Ordering is by size rather than by benchmark score, because the measured ranking
    inversion means a published figure cannot be trusted to order two close models. Size
    is at least an honest proxy, and the earned score settles it properly later.
    """
    fitting = [m for m in installable(role) if m.fits(free_vram_gb)]
    return sorted(fitting, key=lambda m: -m.params_b)
