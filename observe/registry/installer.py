"""Provision the right model for whatever machine this is running on.

    python provision.py                 # profile, pick, download, print the run command
    python provision.py --dry-run       # decide and explain, download nothing
    python provision.py --role embedding

The point is fleet measurement: the same command on a 4 GB laptop and a 24 GB workstation
installs different weights and says why, so a second machine can be measured without
anyone hand-matching models to hardware. It is also the roadmap's "model manager" in its
first honest form.

## What it refuses to do

**Guess a download.** A model with no verified `hf_repo`/`hf_file` is never fetched. A
plausible Hugging Face path that 401s or serves a different quantisation would install the
wrong thing silently, and the catalogue records exactly one such case already
(`qwen3-coder-30b-a3b`) rather than papering over it.

**Ship a licence problem.** Selection runs through `installable()`, so a non-production
licence cannot be chosen no matter how well it scores — the Codestral trap, enforced
rather than remembered.

**Size on installed memory.** Selection uses **free** VRAM. On the reference machine that
is the difference between "fits" and an out-of-memory crash twenty minutes into a run.

## Resumability

Downloads are 4–14 GB over consumer connections, so a partial file is resumed with an HTTP
Range request rather than restarted. The size is checked against the server's own
`Content-Length` at the end; a truncated GGUF loads far enough to look fine and then
produces garbage, which is a miserable thing to diagnose from a bad benchmark number.
"""

from __future__ import annotations

import os
import shutil
import sys
import time
import urllib.error
import urllib.request
from dataclasses import dataclass
from pathlib import Path

from .device import DeviceProfile, profile
from .models import CATALOGUE, ModelCard, Role, candidates_for

CHUNK = 1 << 20  # 1 MiB


@dataclass
class Plan:
    """What this machine should run, and the reasoning, whether or not it is good news."""

    profile: DeviceProfile
    chosen: ModelCard | None
    reason: str
    blocked: tuple[ModelCard, ...] = ()   # would fit, but no verified download

    def render(self) -> str:
        lines = [
            f"machine        {self.profile.gpu_name or 'no accelerator detected'}",
            f"free VRAM      {self.profile.free_vram_gb:.1f} GB of "
            f"{self.profile.total_vram_gb:.1f} GB",
            f"tier           {self.profile.tier.value}",
            "",
        ]
        if self.chosen:
            c = self.chosen
            lines += [
                f"selected       {c.name}  ({c.params_b:.0f}B {c.quantisation}, "
                f"{c.file_size_gb:.1f} GB, {c.licence.label})",
                f"reason         {self.reason}",
            ]
        else:
            lines += [f"selected       nothing — {self.reason}"]
        if self.blocked:
            lines += ["", "would fit but has no verified download:"]
            lines += [f"  {m.name}" for m in self.blocked]
        return "\n".join(lines)


def plan_for(
    prof: DeviceProfile | None = None, *, role: Role = Role.CODE_GENERATION
) -> Plan:
    """Choose the largest model this machine can hold and we can actually fetch."""
    prof = prof or profile()
    fitting = candidates_for(prof.free_vram_gb, role)   # already licence-filtered, big first
    downloadable = [m for m in fitting if m.downloadable]
    blocked = tuple(m for m in fitting if not m.downloadable)

    if downloadable:
        chosen = downloadable[0]
        return Plan(
            prof, chosen,
            f"largest {role.value} model that fits in {prof.free_vram_gb:.1f} GB free "
            f"and has a verified download",
            blocked,
        )

    if blocked:
        return Plan(prof, None,
                    "every model that fits lacks a verified download; refusing to guess",
                    blocked)

    smallest = min(
        (m for m in CATALOGUE if m.role is role and m.installable),
        key=lambda m: m.min_free_vram_gb, default=None,
    )
    need = f"{smallest.min_free_vram_gb:.0f} GB" if smallest else "more"
    return Plan(
        prof, None,
        f"no {role.value} model fits: {prof.free_vram_gb:.1f} GB free, smallest needs {need}. "
        "Partial offload was measured at a 13% gain, so running one anyway is not a "
        "usable capability.",
    )


# ------------------------------------------------------------------ download


def _human(n: float) -> str:
    return f"{n / (1 << 30):.2f} GB"


def download(card: ModelCard, dest_dir: Path, *, quiet: bool = False) -> Path:
    """Fetch the weights, resuming a partial file rather than starting over."""
    url = card.download_url
    if not url:
        raise ValueError(f"{card.name} has no verified download")

    dest_dir.mkdir(parents=True, exist_ok=True)
    target = dest_dir / card.hf_file  # type: ignore[operator]
    done = target.stat().st_size if target.exists() else 0

    request = urllib.request.Request(url, headers={"User-Agent": "localm-installer"})
    if done:
        request.add_header("Range", f"bytes={done}-")
        if not quiet:
            print(f"  resuming at {_human(done)}")

    try:
        response = urllib.request.urlopen(request, timeout=120)
    except urllib.error.HTTPError as exc:
        if exc.code == 416 and done:
            # The server says there is nothing past our offset: already complete.
            return target
        raise

    # A 200 to a Range request means the server ignored it and is sending the whole file;
    # appending would corrupt what is already on disk.
    if done and response.status == 200:
        if not quiet:
            print("  server ignored the resume request — restarting the download")
        done = 0

    total = int(response.headers.get("Content-Length") or 0) + done
    mode = "ab" if done else "wb"
    started, last = time.monotonic(), 0.0

    with response, target.open(mode) as fh:
        while True:
            block = response.read(CHUNK)
            if not block:
                break
            fh.write(block)
            done += len(block)
            now = time.monotonic()
            if not quiet and now - last > 1.0:
                rate = done / max(now - started, 1e-6) / (1 << 20)
                pct = f"{done / total:6.1%}" if total else "     ?"
                print(f"\r  {pct}  {_human(done)}  {rate:5.1f} MB/s", end="", flush=True)
                last = now
    if not quiet:
        print(f"\r  100.0%  {_human(done)}{' ' * 20}")

    if total and done != total:
        # A truncated GGUF loads far enough to look fine and then emits garbage. Refuse
        # to hand back a file we cannot vouch for.
        raise IOError(
            f"{card.hf_file} is {done} bytes, expected {total}. "
            "Re-run to resume; the partial file is kept."
        )
    return target


def verify(path: Path, card: ModelCard) -> tuple[bool, str]:
    """Sanity-check what landed on disk.

    Size against the card's declared figure, with a generous tolerance because the
    catalogue rounds to a tenth of a gigabyte, plus the GGUF magic bytes — which catches
    the common failure of downloading an HTML error page under a .gguf name.
    """
    if not path.exists():
        return False, "file is missing"
    size_gb = path.stat().st_size / (1 << 30)
    if abs(size_gb - card.file_size_gb) > max(1.0, card.file_size_gb * 0.25):
        return False, f"size {size_gb:.2f} GB is far from the expected {card.file_size_gb} GB"
    with path.open("rb") as fh:
        if fh.read(4) != b"GGUF":
            return False, "not a GGUF file (an HTML error page saved under a .gguf name?)"
    return True, f"{size_gb:.2f} GB, GGUF header present"


def run_command(card: ModelCard, path: Path, prof: DeviceProfile) -> str:
    """The exact llama-server line for this model on this machine."""
    server = "llama-server" if shutil.which("llama-server") else "<path-to>/llama-server"
    ctx = min(card.context_tokens, 8192)
    flags = f"-c {ctx} -ngl 999"
    if card.role is Role.EMBEDDING:
        flags = f"-c 512 --embeddings"
    reasoning = "" if card.role is Role.EMBEDDING else " --reasoning off"
    return f'{server} -m "{path}" --host 127.0.0.1 --port 8080 {flags}{reasoning}'


def main(argv: list[str] | None = None) -> int:
    import argparse

    parser = argparse.ArgumentParser(
        prog="provision",
        description="Install the right local model for this machine.",
    )
    parser.add_argument("--dest", default=os.environ.get("LOCALM_MODELS", "models"),
                        help="where to put the weights (default: ./models)")
    parser.add_argument("--role", default="code-generation",
                        choices=[r.value for r in Role])
    parser.add_argument("--dry-run", action="store_true",
                        help="decide and explain, download nothing")
    parser.add_argument("--quiet", action="store_true")
    args = parser.parse_args(argv)

    plan = plan_for(role=Role(args.role))
    print(plan.render())

    if plan.chosen is None:
        print()
        print("Nothing to install for this role on this machine.")
        if Role(args.role) is Role.CODE_GENERATION:
            print("The retrieval model runs almost anywhere — try:")
            print("  python provision.py --role embedding")
        return 1

    if args.dry_run:
        print("\n(dry run — nothing downloaded)")
        return 0

    dest = Path(args.dest).expanduser().resolve()
    print(f"\ndownloading to {dest}")
    try:
        path = download(plan.chosen, dest, quiet=args.quiet)
    except Exception as exc:  # noqa: BLE001
        print(f"\ndownload failed: {exc}")
        print("Re-run to resume — any partial file is kept.")
        return 2

    ok, detail = verify(path, plan.chosen)
    print(f"verify         {'OK' if ok else 'FAILED'} — {detail}")
    if not ok:
        return 2

    print()
    print("=" * 70)
    print("RUN IT")
    print("=" * 70)
    print(run_command(plan.chosen, path, plan.profile))
    if plan.chosen.role is Role.CODE_GENERATION:
        print()
        print("then, in a second terminal:")
        print(f"  python bench.py --dataset evals/data/humaneval-164.yaml \\")
        print(f"      --runner llamacpp --model {plan.chosen.name} --arms constrained \\")
        print(f"      --out evals/m0-report-{plan.chosen.name}.md")
        print()
        print("Check llama-server's first ten lines for how many layers reached the GPU.")
        print("With -ngl 999 on a card that cannot fit the model, llama.cpp aborts the")
        print("fit and runs on CPU — measured here at 6.9 tok/s, which reads as a slow")
        print("model rather than a configuration problem.")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
