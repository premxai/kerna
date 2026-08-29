"""kerna-observe: the model-seam half of Kerna.

Sits between a coding agent and its provider. Records what each turn costs, and --
optionally -- attempts the same turn against a local model afterwards, on the idle
budget, comparing the result and discarding it. It never answers a request.
"""
