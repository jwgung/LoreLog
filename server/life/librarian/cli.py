"""Command line of the librarian (사서): keep the index of every record and search it.

The index is a copy at <data>/librarian.sqlite (mode 600). Only records the diary rules allow are indexed:
messengers the user allowed, voice in conversations the diary counts as the user's. Deleting the file loses nothing.
"""
import argparse
import time

from life.librarian import Librarian


def parser():
    p = argparse.ArgumentParser(prog="life-librarian", description=__doc__.strip().splitlines()[0],
                                epilog="\n".join(__doc__.strip().splitlines()[2:]),
                                formatter_class=argparse.RawDescriptionHelpFormatter)
    sub = p.add_subparsers(dest="cmd", required=True)
    sub.add_parser("update", help="index new and changed records, forget removed ones (life sync runs this)")
    sub.add_parser("rebuild", help="throw the index away and build it again from the records")
    s = sub.add_parser("search", help="what the librarian finds for these words, across all days")
    s.add_argument("words", nargs="+")
    s.add_argument("--limit", type=int, default=10)
    sub.add_parser("stats", help="how much is indexed")
    return p


def update(lib, reset=False):
    t = time.time()
    r = lib.update(reset=reset)
    skipped = f", skipped {r['skipped']} (see above)" if r["skipped"] else ""
    print(f"indexed {r['indexed']} files, removed {r['removed']}{skipped}, {r['docs']} documents ({time.time() - t:.1f}s)")


def search(lib, words, limit):
    hits = lib.search(" ".join(words), limit=limit)
    if not hits:
        print("nothing found")
    for h in hits:
        span = f"  (thread of {h['count']}, {h['first']}~{h['last']})" if h["count"] > 1 else ""
        print(f"{h['day']}  [{h['kind']}] {h['title']}{span}\n    {h['snippet']}\n    {h['ref']}")


def stats(lib):
    s = lib.stats()
    print(f"{s['docs']} documents from {s['files']} files: " + ", ".join(f"{k} {v}" for k, v in sorted(s["by_source"].items())))


def main(argv=None):
    args = parser().parse_args(argv)
    with Librarian() as lib:
        if args.cmd in ("update", "rebuild"):
            # rebuild empties and fills the index in one transaction: a search meanwhile sees the old index
            update(lib, reset=args.cmd == "rebuild")
        elif args.cmd == "search":
            search(lib, args.words, args.limit)
        else:
            stats(lib)
    return 0
