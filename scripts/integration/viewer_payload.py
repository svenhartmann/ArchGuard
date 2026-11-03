"""Decode the report-only compiler transport for automated HTML consumers.

CLI/MCP exports are already ordinary models and do not need this reader.
"""
import json
import re


def decode_payload(data):
    model = data.get("calls")
    compact = model.get("compact") if isinstance(model, dict) else None
    if compact is None:
        return data
    pool, fields = compact.get("strings"), compact.get("fields")
    if compact.get("encoding") != "string-pool-v1" or not isinstance(pool, list) or not all(isinstance(s, str) for s in pool) or not isinstance(fields, dict):
        raise ValueError("Unsupported compact call encoding")

    def string(index):
        if type(index) is not int or not 0 <= index < len(pool):
            raise ValueError("Invalid call string-pool reference")
        return pool[index]

    for table in ("classes", "methods", "calls"):
        rows, keys = model.get(table), fields.get(table)
        if not isinstance(rows, list) or not isinstance(keys, list) or not all(isinstance(k, str) for k in keys):
            raise ValueError("Invalid compact call table")
        for row in rows:
            if not isinstance(row, dict):
                raise ValueError("Invalid compact call row")
            for key in keys:
                if key in row and row[key] is not None:
                    value = row[key]
                    row[key] = [string(i) for i in value] if isinstance(value, list) else string(value)
    del model["compact"]
    return data


def payload_from_html(page):
    match = re.search(r'<script id="archguard-data" type="application/json">(.*?)</script>', page, re.S)
    if match is None:
        raise ValueError("Missing archguard report payload")
    return decode_payload(json.loads(match[1]))
