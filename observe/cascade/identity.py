"""Which model produced this row.

Every local request sets `model: "local"`, because llama.cpp has one model loaded and the
field only exists to satisfy APIs that require it. That was harmless while exactly one
model was ever tested, and it is fatal the moment a second one is: rows from two models
are indistinguishable, they pool, and the pooled rate describes no model that has ever
existed.

This is the same failure as an unlabelled cohort — the one `class_version` exists to
prevent, and the one that cost this project a whole grammar run. A measurement pipeline
built for general use has to answer "which model?" before it answers anything else.

## Probed, then declared, then refused

The identity is taken from the server itself where possible: llama.cpp answers
`GET /v1/models` with the loaded model. A label the operator types is a fallback, because
a label is a claim about the world that drifts the first time someone swaps a GGUF and
forgets to update a flag.

When neither is available the row says `unknown` and says *why*. An unknown that announces
itself can be excluded; an unknown that borrows the last known name is a wrong answer with
a confident label on it.
"""

from __future__ import annotations

from typing import Any

PROBED = "probed"
DECLARED = "declared"
UNKNOWN = "unknown"


def probe_model(base_url: str, *, timeout_s: float = 5.0) -> str | None:
    """The model the local server reports, or None.

    Best-effort by design: a server that does not implement `/v1/models`, or is not up
    yet, must not stop a run. It costs one request at startup and the fallback is a
    label, not a failure.
    """
    import httpx

    base = base_url.rstrip("/")
    for path in (f"{base}/v1/models", f"{base}/models"):
        try:
            response = httpx.get(path, timeout=timeout_s)
            if response.status_code != 200:
                continue
            payload = response.json()
        except Exception:  # noqa: BLE001
            continue

        data = payload.get("data") if isinstance(payload, dict) else None
        if isinstance(data, list) and data:
            first = data[0]
            if isinstance(first, dict):
                name = first.get("id") or first.get("model")
                if isinstance(name, str) and name.strip():
                    return _short(name.strip())
    return None


def _short(name: str) -> str:
    """A model identifier without the operator's directory layout.

    llama.cpp answers with the full path it loaded, which on a real machine is
    `C:\\Users\\someone\\localm\\models\\qwen2.5-coder-7b-instruct-q4_k_m.gguf`. The
    filename carries everything identity needs — family, size, quantisation — and the
    rest is a home directory that would travel into every shared log and report for no
    benefit whatsoever.
    """
    cleaned = name.replace("\\", "/").rstrip("/")
    return cleaned.rsplit("/", 1)[-1] or name


def resolve_model(base_url: str | None, declared: str | None) -> tuple[str, str]:
    """(model, source). Probe first, fall back to the declared label, then refuse.

    A declared label wins over nothing and loses to the server's own answer: the operator
    is describing what they *think* is loaded, and the server knows.
    """
    if base_url:
        probed = probe_model(base_url)
        if probed:
            return probed, PROBED

    if declared and declared.strip():
        return declared.strip(), DECLARED

    return UNKNOWN, UNKNOWN


def identity_row(model: str, source: str) -> dict[str, Any]:
    return {"local_model": model, "local_model_source": source}
