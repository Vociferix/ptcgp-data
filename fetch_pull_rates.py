#!/usr/bin/env python3
"""
Fetch pull rate data for a Pokemon TCG Pocket pack from ptcgp.raenonx.cc.
Usage: python3 fetch_pull_rates.py [pack_id]
Default pack_id: BN006_0010_00_000
"""

import re
import json
import sys
import urllib.request

HEADERS = {
    "User-Agent": "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/124.0.0.0 Safari/537.36",
    "Accept": "text/html,application/xhtml+xml,application/xml;q=0.9,*/*;q=0.8",
    "Accept-Language": "en-US,en;q=0.5",
    "Accept-Encoding": "gzip, deflate, br",
}

def fetch_pack_pull_rates(pack_id: str) -> dict:
    url = f"https://ptcgp.raenonx.cc/en/pack/{pack_id}"
    print(f"Fetching {url} ...")

    req = urllib.request.Request(url, headers=HEADERS)
    with urllib.request.urlopen(req) as resp:
        # Handle gzip/br encoding
        encoding = resp.headers.get("Content-Encoding", "")
        raw = resp.read()

    if "gzip" in encoding:
        import gzip
        data = gzip.decompress(raw).decode("utf-8")
    elif "br" in encoding:
        import brotli
        data = brotli.decompress(raw).decode("utf-8")
    elif "zstd" in encoding:
        import zstandard
        data = zstandard.ZstdDecompressor().decompress(raw).decode("utf-8")
    else:
        data = raw.decode("utf-8")

    print(f"Received {len(data)} bytes")
    print(f"cardPullProbabilityMap at: {data.find('cardPullProbabilityMap')}")
    print(f"packPullProbabilityData at: {data.find('packPullProbabilityData')}")

    # Find the Next.js streamed chunks
    chunks = re.findall(r'self\.__next_f\.push\(\[1,"(.*?)"\]\)', data, re.DOTALL)
    print(f"Stream chunks found: {len(chunks)}")

    for i, chunk in enumerate(chunks):
        if "cardPullProbabilityMap" not in chunk:
            continue

        print(f"Target data in chunk {i} (length {len(chunk)})")

        # The chunk content is a JSON-encoded string — unescape it
        unescaped = chunk.encode("utf-8").decode("unicode_escape")

        try:
            return {
                "cardPullProbabilityMap": extract_json_value(unescaped, "cardPullProbabilityMap"),
                "packPullProbabilityData": extract_json_value(unescaped, "packPullProbabilityData"),
            }
        except json.JSONDecodeError as e:
            print(f"JSON parse failed: {e}")
            print(f"Sample: {repr(unescaped[start:start+300])}")
            raise

    raise ValueError(f"cardPullProbabilityMap not found in any stream chunk for pack {pack_id}")


def extract_json_value(text: str, key: str):
    """Find "key":{ in text and extract the complete JSON value."""
    search = f'"{key}":'
    idx = text.find(search)
    if idx == -1:
        return None
    val_start = idx + len(search)
    # Use raw_decode to parse just the value at this position,
    # regardless of what comes after
    decoder = json.JSONDecoder()
    value, _ = decoder.raw_decode(text, val_start)
    return value


def main():
    pack_id = sys.argv[1] if len(sys.argv) > 1 else "BN006_0010_00_000"
    result = fetch_pack_pull_rates(pack_id)

    out_path = f"/tmp/pull_rates_{pack_id}.json"
    with open(out_path, "w") as f:
        json.dump(result, f, indent=2)

    print(f"\nSuccess!")
    print(f"cardPullProbabilityMap: {len(result.get('cardPullProbabilityMap') or {})} cards")
    print(f"Output written to {out_path}")
    print("\npackPullProbabilityData:")
    print(json.dumps(result.get("packPullProbabilityData"), indent=2))


if __name__ == "__main__":
    main()
