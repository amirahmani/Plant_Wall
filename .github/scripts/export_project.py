#!/usr/bin/env python3
"""Export a GitHub Project (v2) board to JSON for the portfolio website.

Reads every item of a user-owned Project with its Status and date fields,
plus the latest commits of this repository, and writes site-data/plant-project.json.

Environment:
  PROJECT_TOKEN   classic PAT with the read:project and repo (or public_repo) scopes
  PROJECT_OWNER   GitHub user that owns the project (default: amirahmani)
  PROJECT_NUMBER  project number (default: 1)
  GITHUB_REPOSITORY  owner/repo, set by Actions
  OUT             output path (default: site-data/plant-project.json)
"""
import json
import os
import re
import sys
import urllib.request
from datetime import date, datetime, timedelta, timezone

TOKEN = os.environ.get("PROJECT_TOKEN", "").strip()
OWNER = os.environ.get("PROJECT_OWNER", "amirahmani")
NUMBER = int(os.environ.get("PROJECT_NUMBER", "1"))
REPO = os.environ.get("GITHUB_REPOSITORY", "amirahmani/Plant_Wall")
OUT = os.environ.get("OUT", "site-data/plant-project.json")

QUERY = """
query($login: String!, $number: Int!, $cursor: String) {
  user(login: $login) {
    projectV2(number: $number) {
      title
      url
      items(first: 100, after: $cursor) {
        pageInfo { hasNextPage endCursor }
        nodes {
          content {
            __typename
            ... on Issue { number title state url repository { nameWithOwner } }
            ... on PullRequest { number title state url repository { nameWithOwner } }
            ... on DraftIssue { title }
          }
          fieldValues(first: 30) {
            nodes {
              __typename
              ... on ProjectV2ItemFieldSingleSelectValue { name field { ... on ProjectV2FieldCommon { name } } }
              ... on ProjectV2ItemFieldDateValue { date field { ... on ProjectV2FieldCommon { name } } }
              ... on ProjectV2ItemFieldIterationValue { title startDate duration field { ... on ProjectV2FieldCommon { name } } }
            }
          }
        }
      }
    }
  }
}
"""


def request(url, body=None):
    headers = {
        "Authorization": f"Bearer {TOKEN}",
        "Accept": "application/vnd.github+json",
        "User-Agent": "plant-wall-project-export",
    }
    data = None
    if body is not None:
        data = json.dumps(body).encode()
        headers["Content-Type"] = "application/json"
    req = urllib.request.Request(url, data=data, headers=headers)
    with urllib.request.urlopen(req, timeout=30) as resp:
        return json.loads(resp.read().decode())


def fetch_items():
    items, cursor, meta = [], None, {}
    while True:
        res = request(
            "https://api.github.com/graphql",
            {"query": QUERY, "variables": {"login": OWNER, "number": NUMBER, "cursor": cursor}},
        )
        if res.get("errors"):
            sys.exit("GraphQL error: " + json.dumps(res["errors"]))
        proj = res["data"]["user"]["projectV2"]
        if proj is None:
            sys.exit(f"Project {OWNER}/{NUMBER} not found or token lacks read:project")
        meta = {"title": proj["title"], "url": proj["url"]}
        page = proj["items"]
        items.extend(page["nodes"])
        if not page["pageInfo"]["hasNextPage"]:
            return meta, items
        cursor = page["pageInfo"]["endCursor"]


def pick(fields, pattern):
    for name, value in fields.items():
        if re.search(pattern, name, re.I) and value:
            return value
    return None


def normalise(node):
    c = node.get("content") or {}
    fields, dates = {}, {}
    for fv in node["fieldValues"]["nodes"]:
        field = (fv.get("field") or {}).get("name")
        if not field:
            continue
        t = fv.get("__typename")
        if t == "ProjectV2ItemFieldSingleSelectValue":
            fields[field] = fv.get("name")
        elif t == "ProjectV2ItemFieldDateValue":
            dates[field] = fv.get("date")
        elif t == "ProjectV2ItemFieldIterationValue":
            start = fv.get("startDate")
            fields[field] = fv.get("title")
            if start:
                end = date.fromisoformat(start) + timedelta(days=int(fv.get("duration") or 1) - 1)
                dates.setdefault(field + " start", start)
                dates.setdefault(field + " end", end.isoformat())
    start = pick(dates, r"start|begin")
    end = pick(dates, r"target|end|due|finish")
    if start and not end:
        end = start
    if end and not start:
        start = end
    return {
        "number": c.get("number"),
        "title": c.get("title") or "Untitled",
        "state": (c.get("state") or "OPEN").lower(),
        "url": c.get("url"),
        "status": fields.get("Status") or pick(fields, r"status"),
        "start": start,
        "end": end,
        "fields": fields,
    }


def fetch_commits(n=8):
    rows = request(f"https://api.github.com/repos/{REPO}/commits?per_page=30")
    out = []
    for r in rows:
        msg = r["commit"]["message"].split("\n")[0]
        if "[skip ci]" in msg or msg.startswith("Update project export"):
            continue  # skip this workflow's own commits
        out.append({
            "sha": r["sha"][:7],
            "date": r["commit"]["author"]["date"],
            "message": msg,
            "url": r["html_url"],
        })
        if len(out) == n:
            break
    return out


def main():
    if not TOKEN:
        sys.exit("PROJECT_TOKEN secret is not set; see README in .github/")
    meta, nodes = fetch_items()
    items = [normalise(n) for n in nodes]
    payload = {
        "source": "github-project",
        "project": meta,
        "repo": f"https://github.com/{REPO}",
        "items": items,
        "commits": fetch_commits(),
    }
    old = None
    if os.path.exists(OUT):
        with open(OUT) as f:
            old = json.load(f)
        old.pop("updated", None)
    if old == payload:
        print("No change")
        return
    payload = {"updated": datetime.now(timezone.utc).strftime("%Y-%m-%dT%H:%M:%SZ"), **payload}
    os.makedirs(os.path.dirname(OUT), exist_ok=True)
    with open(OUT, "w") as f:
        json.dump(payload, f, indent=1)
        f.write("\n")
    print(f"Wrote {len(items)} items to {OUT}")


if __name__ == "__main__":
    main()
