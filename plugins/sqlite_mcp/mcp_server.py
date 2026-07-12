#!/usr/bin/env python3
"""Kerna starter plugin: read-only SQLite query.

Speaks standard MCP over stdio. Pure Python standard library (sqlite3).
Points at ONE local database file via the KERNA_SQLITE_DB env var (set it with
`kerna secrets add sqlite`, or export it directly). Read-only by default: only
SELECT / PRAGMA / EXPLAIN statements are allowed; anything that could mutate the
database is refused.
"""
import os
import sys
import json
import sqlite3

MAX_ROWS = 200


def _db_path():
    p = os.environ.get("KERNA_SQLITE_DB", "").strip()
    if not p:
        raise ValueError(
            "KERNA_SQLITE_DB is not set. Point it at a database file, e.g. "
            "`kerna secrets add sqlite` or export KERNA_SQLITE_DB=/path/to.db"
        )
    if not os.path.exists(p):
        raise ValueError("database file not found: %s" % p)
    return p


def _readonly_connect():
    # Open in immutable/read-only mode so even a bug can't write.
    uri = "file:%s?mode=ro" % _db_path()
    return sqlite3.connect(uri, uri=True)


def list_tables():
    con = _readonly_connect()
    try:
        rows = con.execute(
            "SELECT name FROM sqlite_master WHERE type='table' ORDER BY name"
        ).fetchall()
        return "\n".join(r[0] for r in rows) or "(no tables)"
    finally:
        con.close()


def sql_query(sql):
    stripped = sql.strip().lower()
    if not (
        stripped.startswith("select")
        or stripped.startswith("pragma")
        or stripped.startswith("explain")
        or stripped.startswith("with")
    ):
        raise ValueError(
            "read-only: only SELECT / WITH / PRAGMA / EXPLAIN are allowed"
        )
    con = _readonly_connect()
    try:
        cur = con.execute(sql)
        cols = [d[0] for d in (cur.description or [])]
        rows = cur.fetchmany(MAX_ROWS)
        out = []
        if cols:
            out.append(" | ".join(cols))
        for row in rows:
            out.append(" | ".join("" if v is None else str(v) for v in row))
        if len(rows) == MAX_ROWS:
            out.append("... (truncated at %d rows)" % MAX_ROWS)
        return "\n".join(out) or "(no rows)"
    finally:
        con.close()


TOOLS = [
    {
        "name": "list_tables",
        "description": "List the tables in the configured SQLite database.",
        "inputSchema": {"type": "object", "properties": {}},
    },
    {
        "name": "sql_query",
        "description": "Run a read-only SQL query (SELECT/WITH/PRAGMA/EXPLAIN only) against the configured SQLite database.",
        "inputSchema": {
            "type": "object",
            "properties": {"sql": {"type": "string"}},
            "required": ["sql"],
        },
    },
]


def call(name, args):
    if name == "list_tables":
        return list_tables()
    if name == "sql_query":
        return sql_query(args["sql"])
    raise ValueError("unknown tool: %s" % name)


def handle(req):
    method = req.get("method")
    rid = req.get("id")
    if method == "initialize":
        return {"jsonrpc": "2.0", "id": rid, "result": {
            "protocolVersion": "2024-11-05", "capabilities": {"tools": {}},
            "serverInfo": {"name": "sqlite", "version": "1.0.0"}}}
    if method in ("notifications/initialized", "notifications/cancelled"):
        return None
    if method == "tools/list":
        return {"jsonrpc": "2.0", "id": rid, "result": {"tools": TOOLS}}
    if method == "tools/call":
        params = req.get("params", {})
        try:
            text = call(params.get("name"), params.get("arguments", {}))
            return {"jsonrpc": "2.0", "id": rid, "result": {"content": [{"type": "text", "text": text}]}}
        except Exception as e:  # noqa: BLE001
            return {"jsonrpc": "2.0", "id": rid, "result": {"isError": True, "content": [{"type": "text", "text": "error: %s" % e}]}}
    return {"jsonrpc": "2.0", "id": rid, "error": {"code": -32601, "message": "method not found"}}


def main():
    for line in sys.stdin:
        line = line.strip()
        if not line:
            continue
        try:
            resp = handle(json.loads(line))
        except Exception:  # noqa: BLE001
            resp = {"jsonrpc": "2.0", "id": None, "error": {"code": -32700, "message": "parse error"}}
        if resp is not None:
            sys.stdout.write(json.dumps(resp) + "\n")
            sys.stdout.flush()


if __name__ == "__main__":
    main()
