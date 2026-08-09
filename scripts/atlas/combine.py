#!/usr/bin/env python3
"""Inject enriched atlas data + app.js into template.html, producing atlas.html."""
import argparse


def main():
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument("--template", required=True)
    ap.add_argument("--data", required=True, help="enriched atlas_data JSON")
    ap.add_argument("--app-js", required=True)
    ap.add_argument("--out", required=True)
    args = ap.parse_args()

    template = open(args.template).read()
    data = open(args.data).read()
    app_js = open(args.app_js).read()

    html = template.replace("__ATLAS_DATA__", data, 1).replace("__ATLAS_APP_JS__", app_js, 1)
    open(args.out, "w").write(html)
    print(f"wrote {args.out}")


if __name__ == "__main__":
    main()
