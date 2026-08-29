"""kerna-observe: the model-seam half of Kerna.

Sits between a coding agent and its provider. Records what each turn costs, and --
optionally -- attempts the same turn against a local model afterwards, on the idle
budget, comparing the result and discarding it. It never answers a request.
"""

# Read by the release workflow, which refuses to publish unless this agrees
# with the kernel, desktop and npm versions and with the tag. It was missing
# while the workflow already expected it, so any tag would have died on an
# AttributeError instead of reporting a version mismatch.
__version__ = "0.2.9"
