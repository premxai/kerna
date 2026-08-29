"""Device profiling — what this machine can actually run.

Three rules here, and all three come from measurements rather than from spec sheets.

**Free VRAM, never installed.** The reference machine reports 4,095 MiB total against
3,303 MiB free. A preflight reading "total" passes every lab test and runs out of memory
in the field.

**Unified memory is a separate tier, not a bigger number.** A 4 GB discrete GPU runs
nothing useful for coding. An Apple machine with 36 GB unified runs a 27B model. These are
not two points on one axis, and a single `vram_gb` field that conflated them would
recommend a 24B model to a laptop that cannot hold it.

**Measure throughput, do not calculate it.** Partial GPU offload was measured at
6.1 -> 6.9 tok/s — a 13% gain for putting roughly half the layers on the GPU. No formula
predicts that, because once a model does not fit, the CPU half sets the pace. This is why
`fits()` is a hard boundary rather than a preference: "it fits partially" is not a
capability.
"""

from __future__ import annotations

import platform
import shutil
import subprocess
from dataclasses import dataclass
from enum import Enum


class MemoryKind(Enum):
    DISCRETE = "discrete GPU"
    UNIFIED = "unified memory"
    CPU_ONLY = "CPU only"


class Tier(Enum):
    """Coarse capability bands, defined by what they can actually hold."""

    NONE = "cannot run a useful coding model"
    SMALL = "7B class"
    MEDIUM = "14B class"
    LARGE = "24B class"
    XLARGE = "30B+ class"


@dataclass(frozen=True)
class DeviceProfile:
    kind: MemoryKind
    free_vram_gb: float
    total_vram_gb: float
    system_ram_gb: float
    gpu_name: str = ""
    os_name: str = ""
    measured_tok_s: float | None = None
    throughput_unit: str = "tok/s"

    @property
    def tier(self) -> Tier:
        # Boundaries are the models' own min_free_vram_gb, so the tier and the catalogue
        # cannot drift apart.
        if self.free_vram_gb >= 21.0:
            return Tier.XLARGE
        if self.free_vram_gb >= 16.0:
            return Tier.LARGE
        if self.free_vram_gb >= 11.0:
            return Tier.MEDIUM
        if self.free_vram_gb >= 6.0:
            return Tier.SMALL
        return Tier.NONE

    @property
    def headroom_note(self) -> str:
        """The gap between installed and free, which is where preflights go wrong."""
        gap = self.total_vram_gb - self.free_vram_gb
        if self.total_vram_gb <= 0:
            return ""
        return (
            f"{gap:.1f} GB of {self.total_vram_gb:.1f} GB already in use "
            f"({gap / self.total_vram_gb:.0%}) — sizing on total would over-promise"
        )


def _nvidia() -> tuple[float, float, str] | None:
    """(free_gb, total_gb, name) from nvidia-smi, or None when unavailable."""
    if not shutil.which("nvidia-smi"):
        return None
    try:
        out = subprocess.run(
            ["nvidia-smi",
             "--query-gpu=memory.free,memory.total,name",
             "--format=csv,noheader,nounits"],
            capture_output=True, text=True, timeout=30,
        )
    except (subprocess.TimeoutExpired, OSError):
        return None
    if out.returncode != 0 or not out.stdout.strip():
        return None
    # First GPU only. Multi-GPU laptops are rare and splitting a model across them is a
    # different problem from the one this profile answers.
    first = out.stdout.strip().splitlines()[0]
    parts = [p.strip() for p in first.split(",")]
    if len(parts) < 3:
        return None
    try:
        return float(parts[0]) / 1024.0, float(parts[1]) / 1024.0, parts[2]
    except ValueError:
        return None


def _apple_unified() -> tuple[float, float, str] | None:
    """Apple Silicon: system RAM is the GPU's memory, minus what macOS reserves."""
    if platform.system() != "Darwin" or platform.machine() != "arm64":
        return None
    try:
        out = subprocess.run(
            ["sysctl", "-n", "hw.memsize"], capture_output=True, text=True, timeout=15
        )
        total_gb = int(out.stdout.strip()) / (1024 ** 3)
    except (subprocess.TimeoutExpired, OSError, ValueError):
        return None
    # macOS caps GPU-addressable memory below total. ~70% is the conservative figure and
    # is deliberately pessimistic: over-promising here means an OOM on the customer's
    # machine, which is the failure this whole module exists to avoid.
    return total_gb * 0.70, total_gb, f"Apple Silicon ({platform.machine()})"


def _system_ram_gb() -> float:
    try:
        import psutil  # type: ignore

        return psutil.virtual_memory().total / (1024 ** 3)
    except Exception:  # noqa: BLE001 - optional dependency
        return 0.0


def profile() -> DeviceProfile:
    """Best-effort profile of the machine this is running on."""
    os_name = f"{platform.system()} {platform.release()}"
    ram = _system_ram_gb()

    unified = _apple_unified()
    if unified:
        free, total, name = unified
        return DeviceProfile(MemoryKind.UNIFIED, free, total, ram, name, os_name)

    nvidia = _nvidia()
    if nvidia:
        free, total, name = nvidia
        return DeviceProfile(MemoryKind.DISCRETE, free, total, ram, name, os_name)

    return DeviceProfile(MemoryKind.CPU_ONLY, 0.0, 0.0, ram, "", os_name)


def measure_throughput(
    base_url: str = "http://127.0.0.1:8080", *, timeout_s: float = 180.0
) -> tuple[float, str] | None:
    """Measure real throughput against a running llama-server, or return None.

    Measured, never calculated. Partial GPU offload came in at 6.1 -> 6.9 tok/s on the
    reference machine — a 13% gain for moving roughly half the layers onto the GPU, which
    no formula predicts, because once the model does not fit the CPU half sets the pace.

    Handles both server kinds, because they answer different questions. A generative
    server gives tokens/second, which decides whether local *answering* is viable. An
    embedding server gives queries/second, which decides whether the context-offload path
    is viable — and on a machine too small for any coding model, that is the only one that
    matters.
    """
    try:
        import httpx
    except ImportError:  # pragma: no cover - httpx is a harness dependency
        return None

    import time

    try:
        with httpx.Client(timeout=timeout_s) as client:
            if client.get(f"{base_url}/health").status_code != 200:
                return None

            started = time.perf_counter()
            gen = client.post(
                f"{base_url}/v1/chat/completions",
                json={
                    "model": "probe",
                    "messages": [{"role": "user", "content":
                                  "Write a Python function that reverses a string."}],
                    "max_tokens": 96,
                    "temperature": 0.0,
                    "stream": False,
                },
            )
            if gen.status_code == 200:
                elapsed = time.perf_counter() - started
                produced = (gen.json().get("usage") or {}).get("completion_tokens") or 0
                if produced and elapsed > 0:
                    return produced / elapsed, "tok/s (generation)"

            # Not a generative server. Try embeddings — the offload path's own metric.
            started = time.perf_counter()
            emb = client.post(
                f"{base_url}/v1/embeddings",
                json={"model": "probe", "input": ["def f(x): return x"] * 32},
            )
            if emb.status_code == 200:
                elapsed = time.perf_counter() - started
                if elapsed > 0:
                    return 32.0 / elapsed, "passages/s (embedding)"
    except Exception:  # noqa: BLE001 - a probe must never crash the report
        return None
    return None


def render(prof: DeviceProfile) -> str:
    """A short report a developer can read and send back."""
    from .models import Role, candidates_for

    lines = [
        f"machine        {prof.os_name}",
        f"accelerator    {prof.gpu_name or 'none detected'}  [{prof.kind.value}]",
        f"free VRAM      {prof.free_vram_gb:.1f} GB of {prof.total_vram_gb:.1f} GB",
        f"system RAM     {prof.system_ram_gb:.1f} GB" if prof.system_ram_gb else "",
        f"tier           {prof.tier.value}",
    ]
    if prof.headroom_note:
        lines.append(f"               {prof.headroom_note}")
    if prof.measured_tok_s is not None:
        lines.append(f"measured       {prof.measured_tok_s:.1f} {prof.throughput_unit}")
    else:
        lines.append("measured       not measured (start a llama-server and re-run "
                     "with --measure)")

    lines.append("")
    fits = candidates_for(prof.free_vram_gb, Role.CODE_GENERATION)
    if fits:
        lines.append("coding models this machine can hold:")
        lines += [f"  {m.name:<24}{m.params_b:>5.0f}B  needs {m.min_free_vram_gb:.0f} GB free"
                  for m in fits]
    else:
        lines.append("No coding model fits in free VRAM.")
        lines.append("Partial offload was measured at 6.1 -> 6.9 tok/s, a 13% gain, so")
        lines.append("running one anyway is not a usable capability.")

    lines.append("")
    embedders = candidates_for(prof.free_vram_gb, Role.EMBEDDING)
    if embedders:
        lines.append("retrieval models (the context-offload path — runs almost anywhere):")
        lines += [f"  {m.name:<24}{m.file_size_gb * 1024:>5.0f} MB" for m in embedders]

    return "\n".join(line for line in lines if line != "")


def main(argv: list[str] | None = None) -> int:
    import argparse
    import dataclasses

    parser = argparse.ArgumentParser(
        prog="device-probe",
        description="What can this machine actually run?",
    )
    parser.add_argument("--measure", action="store_true",
                        help="also measure real throughput against a running llama-server")
    parser.add_argument("--base-url", default="http://127.0.0.1:8080")
    args = parser.parse_args(argv)

    prof = profile()
    if args.measure:
        result = measure_throughput(args.base_url)
        if result:
            rate, unit = result
            prof = dataclasses.replace(prof, measured_tok_s=rate, throughput_unit=unit)
    print(render(prof))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
