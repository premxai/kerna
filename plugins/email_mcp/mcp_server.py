#!/usr/bin/env python3
"""Kerna starter plugin: email (send + read via IMAP/SMTP).

Speaks standard MCP over stdio. Pure Python standard library (smtplib,
imaplib, email) — no pip install. Give the agent its OWN mailbox, not your
personal one, and use an app-specific password (never your real account
password).

Kerna injects these secrets into this plugin's environment
(`kerna secrets add email`):
  EMAIL_ADDRESS    the mailbox address, e.g. myagent@gmail.com
  EMAIL_PASSWORD   an app-specific password for that mailbox
  EMAIL_SMTP_HOST  optional (default smtp.gmail.com)
  EMAIL_SMTP_PORT  optional (default 587, STARTTLS)
  EMAIL_IMAP_HOST  optional (default imap.gmail.com)

`send_email` is deliberately named so Kerna's approval prompt shows the full
recipient / subject / body before anything is sent — every send requires your
confirmation.
"""
import os
import sys
import json
import smtplib
import imaplib
import email
from email.message import EmailMessage
from email.header import decode_header

TIMEOUT = 30


def _creds():
    addr = os.environ.get("EMAIL_ADDRESS", "").strip()
    pw = os.environ.get("EMAIL_PASSWORD", "").strip()
    if not addr or not pw:
        raise ValueError(
            "EMAIL_ADDRESS / EMAIL_PASSWORD are not set. Run: kerna secrets add email "
            "(use a dedicated mailbox + an app-specific password)."
        )
    return addr, pw


def send_email(to, subject, body):
    addr, pw = _creds()
    host = os.environ.get("EMAIL_SMTP_HOST", "smtp.gmail.com").strip()
    port = int(os.environ.get("EMAIL_SMTP_PORT", "587"))
    msg = EmailMessage()
    msg["From"] = addr
    msg["To"] = to
    msg["Subject"] = subject
    msg.set_content(body)
    with smtplib.SMTP(host, port, timeout=TIMEOUT) as s:
        s.starttls()
        s.login(addr, pw)
        s.send_message(msg)
    return "Sent email to %s (subject: %s)" % (to, subject)


def _decode(value):
    if not value:
        return ""
    parts = decode_header(value)
    out = []
    for text, enc in parts:
        if isinstance(text, bytes):
            out.append(text.decode(enc or "utf-8", errors="replace"))
        else:
            out.append(text)
    return "".join(out)


def _imap():
    addr, pw = _creds()
    host = os.environ.get("EMAIL_IMAP_HOST", "imap.gmail.com").strip()
    m = imaplib.IMAP4_SSL(host, timeout=TIMEOUT)
    m.login(addr, pw)
    return m


def list_recent_emails(count=10):
    m = _imap()
    try:
        m.select("INBOX")
        _typ, data = m.search(None, "ALL")
        ids = data[0].split()
        ids = ids[-count:][::-1] if ids else []
        lines = []
        for i in ids:
            _typ, msg_data = m.fetch(i, "(BODY.PEEK[HEADER.FIELDS (FROM SUBJECT DATE)])")
            raw = msg_data[0][1]
            hdr = email.message_from_bytes(raw)
            lines.append(
                "#%s  From: %s | Subject: %s | %s"
                % (
                    i.decode(),
                    _decode(hdr.get("From")),
                    _decode(hdr.get("Subject")),
                    _decode(hdr.get("Date")),
                )
            )
        return "\n".join(lines) if lines else "inbox is empty"
    finally:
        m.logout()


def read_email(msg_id):
    m = _imap()
    try:
        m.select("INBOX")
        _typ, msg_data = m.fetch(str(msg_id).encode(), "(RFC822)")
        if not msg_data or not msg_data[0]:
            return "no message with id %s" % msg_id
        msg = email.message_from_bytes(msg_data[0][1])
        body = ""
        if msg.is_multipart():
            for part in msg.walk():
                if part.get_content_type() == "text/plain":
                    body = part.get_payload(decode=True).decode(errors="replace")
                    break
        else:
            body = msg.get_payload(decode=True).decode(errors="replace")
        return "From: %s\nSubject: %s\nDate: %s\n\n%s" % (
            _decode(msg.get("From")),
            _decode(msg.get("Subject")),
            _decode(msg.get("Date")),
            body[:4000],
        )
    finally:
        m.logout()


TOOLS = [
    {
        "name": "send_email",
        "description": "Send an email from the agent's mailbox. Requires your approval — you'll see the full recipient, subject, and body first.",
        "inputSchema": {
            "type": "object",
            "properties": {
                "to": {"type": "string"},
                "subject": {"type": "string"},
                "body": {"type": "string"},
            },
            "required": ["to", "subject", "body"],
        },
    },
    {
        "name": "list_recent_emails",
        "description": "List the most recent emails in the inbox (sender, subject, date).",
        "inputSchema": {
            "type": "object",
            "properties": {"count": {"type": "integer", "default": 10}},
        },
    },
    {
        "name": "read_email",
        "description": "Read the full text of one email by its inbox id (from list_recent_emails).",
        "inputSchema": {
            "type": "object",
            "properties": {"msg_id": {"type": "string"}},
            "required": ["msg_id"],
        },
    },
]


def call(name, args):
    if name == "send_email":
        return send_email(args["to"], args["subject"], args["body"])
    if name == "list_recent_emails":
        return list_recent_emails(int(args.get("count", 10)))
    if name == "read_email":
        return read_email(args["msg_id"])
    raise ValueError("unknown tool: %s" % name)


def handle(req):
    method = req.get("method")
    rid = req.get("id")
    if method == "initialize":
        return {"jsonrpc": "2.0", "id": rid, "result": {
            "protocolVersion": "2024-11-05", "capabilities": {"tools": {}},
            "serverInfo": {"name": "email", "version": "1.0.0"}}}
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
