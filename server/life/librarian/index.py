"""The librarian's index: SQLite FTS5 over every document, kept in step with the files it came from.

Korean is indexed in two-letter pieces ("거래처랑" -> 거래 래처 처랑) so a word is found inside a longer word with a
particle attached, and a two-letter word like "견적" is found at all; FTS5's own tokenizers do neither. A query word
becomes a phrase of its pieces (so the pieces must be adjacent), a single Korean letter becomes a prefix, and the words
of a query are joined with OR and ranked by bm25: recall first, the best matches on top.

The index is a copy built from the records. Deleting the file loses nothing; `rebuild` makes it again. A file that
cannot be read (or holds a record that cannot be understood) is skipped and its old documents are dropped, so what the
index answers is never older than the rules it was built under; the next update tries the file again.
"""
import os
import re
import sqlite3
import sys
from pathlib import Path

from life.assistant.config import DATA
from life.librarian import records

WORD_RE = re.compile(r"[0-9a-z]+|[가-힣\u4e00-\u9fff]+")
MAX_QUERY_WORDS = 16
SNIPPET_BEFORE, SNIPPET_AFTER = 60, 140
WINDOW = 400  # matches looked at per search before the mails of one thread are folded together
SCHEMA_VERSION = "4"  # bump when the tables or what goes in them change: an index of another version is built again

SCHEMA = """
create table if not exists meta(key text primary key, value text);
create table if not exists files(path text primary key, kind text, stamp text);
create table if not exists docs(n integer primary key, file text, source text, kind text, day text, title text,
                                thread text, text text);
create index if not exists docs_file on docs(file);
create index if not exists docs_thread on docs(thread);
create virtual table if not exists fts using fts5(terms);
"""


def default_path():
    return DATA / "librarian.sqlite"


def _pieces(word):
    return [word] if word.isascii() or len(word) == 1 else [word[i:i + 2] for i in range(len(word) - 1)]


def terms(text):
    """The index terms of a text, space separated."""
    return " ".join(p for w in WORD_RE.findall(text.lower()) for p in _pieces(w))


def to_query(q):
    """An FTS5 expression for the words of q (at most MAX_QUERY_WORDS of them), or "" if q has none.
    Every word is quoted, so nothing in q can act as an FTS5 operator or column filter."""
    parts = []
    for w in WORD_RE.findall(q.lower()):
        parts.append(f'"{w}"*' if not w.isascii() and len(w) == 1 else '"' + " ".join(_pieces(w)) + '"')
    return " OR ".join(list(dict.fromkeys(parts))[:MAX_QUERY_WORDS])


def snippet(text, q):
    """The part of the text around the first word of the query that appears in it."""
    low = text.lower()
    at = [i for i in (low.find(w) for w in WORD_RE.findall(q.lower())) if i >= 0]
    start = max(0, min(at) - SNIPPET_BEFORE) if at else 0
    end = start + SNIPPET_BEFORE + SNIPPET_AFTER
    body = re.sub(r"\s+", " ", text[start:end]).strip()
    return ("…" if start > 0 else "") + body + ("…" if end < len(text) else "")


def _stamp(path):
    st = path.stat()
    return f"{st.st_mtime_ns}:{st.st_size}"


class Librarian:
    def __init__(self, path=None, timeout=30):
        self.path = Path(path) if path else default_path()
        self.path.parent.mkdir(parents=True, exist_ok=True)
        if not self.path.exists():  # create it private first so the WAL files SQLite adds inherit the mode
            os.close(os.open(self.path, os.O_CREAT | os.O_WRONLY, 0o600))
        self.db = sqlite3.connect(self.path, timeout=timeout)
        self.db.execute("pragma journal_mode=wal")
        self.db.executescript(SCHEMA)
        if self._meta("schema") != SCHEMA_VERSION:
            with self.db:  # an index of another shape: start over (it is only a copy of the records)
                self.db.executescript("drop table if exists docs; drop table if exists files; drop table if exists fts;"
                                      "delete from meta;" + SCHEMA)
                self.db.execute("insert into meta(key, value) values ('schema', ?)", (SCHEMA_VERSION,))

    def close(self):
        self.db.close()

    def __enter__(self):
        return self

    def __exit__(self, *exc):
        self.close()

    def _meta(self, key):
        row = self.db.execute("select value from meta where key = ?", (key,)).fetchone()
        return row[0] if row else None

    def _forget(self, path):
        ns = [n for (n,) in self.db.execute("select n from docs where file = ?", (path,))]
        self.db.executemany("delete from fts where rowid = ?", [(n,) for n in ns])
        self.db.execute("delete from docs where file = ?", (path,))

    def _add(self, path, d):
        n = self.db.execute("insert into docs(file, source, kind, day, title, thread, text) values (?, ?, ?, ?, ?, ?, ?)",
                            (path, d["source"], d["kind"], d["day"], d["title"], d.get("thread"), d["text"])).lastrowid
        self.db.execute("insert into fts(rowid, terms) values (?, ?)",
                        (n, terms("\n".join((d["title"], d.get("extra") or "", d["text"])))))

    def _changes(self, found):
        """(files to read again, files gone). Phone files are all read again when the allowed senders change or any
        phone file changes: a conversation's state can arrive in a later day's file and change an earlier day."""
        known = {path: (kind, stamp) for path, kind, stamp in self.db.execute("select path, kind, stamp from files")}
        dirty = set()
        for path, (_, p) in list(found.items()):
            try:
                if known.get(path, (None, None))[1] != _stamp(p):
                    dirty.add(path)
            except FileNotFoundError:  # removed between listing and now (a sync is writing): treat it as gone
                del found[path]
        removed = [path for path in known if path not in found]
        phone = {path for path, (kind, _) in found.items() if kind == "lorelog"}
        if self._meta("policy") != records.policy_stamp() or dirty & phone or any(known[p][0] == "lorelog" for p in removed):
            dirty |= phone
        return dirty, removed

    def update(self):
        """Index new and changed files, forget removed ones. -> {indexed, removed, skipped, docs}"""
        found = {str(p): (kind, p) for kind, p in records.sources()}
        dirty, removed = self._changes(found)
        rules = records.phone_rules() if any(found[p][0] == "lorelog" for p in dirty) else None
        indexed = skipped = 0
        with self.db:
            for path in removed:
                self._forget(path)
                self.db.execute("delete from files where path = ?", (path,))
            for path in sorted(dirty):
                kind, p = found[path]
                self._forget(path)
                try:
                    stamp, new = _stamp(p), records.docs(kind, p, rules)
                except Exception as e:  # noqa: BLE001 - one unreadable file must not stop the rest
                    self.db.execute("delete from files where path = ?", (path,))  # read it again next time
                    sys.stderr.write(f"librarian: skipped {p.name}: {type(e).__name__}\n")
                    skipped += 1
                    continue
                for d in new:
                    self._add(path, d)
                self.db.execute("insert or replace into files(path, kind, stamp) values (?, ?, ?)", (path, kind, stamp))
                indexed += 1
            self.db.execute("insert or replace into meta(key, value) values ('policy', ?)", (records.policy_stamp(),))
        count = self.db.execute("select count(*) from docs").fetchone()[0]
        return {"indexed": indexed, "removed": len(removed), "skipped": skipped, "docs": count}

    def _thread_span(self, thread):
        return self.db.execute("select count(*), min(day), max(day) from docs where thread = ?", (thread,)).fetchone()

    def search(self, q, limit=10, exclude_day=None):
        """Documents matching the words of q, best first. The mails of one thread come back as one hit (its
        best-matching mail) with the thread's size and the days it spans, so a long thread does not take every slot.
        -> [{source, kind, day, title, snippet, ref, count, first, last}]"""
        expr = to_query(q or "")
        if not expr:
            return []
        sql = "select d.n, d.thread from fts join docs d on d.n = fts.rowid where fts match ?"
        params = [expr]
        if exclude_day:
            sql += " and d.day != ?"
            params.append(exclude_day)
        sql += " order by fts.rank limit ?"
        params.append(WINDOW)
        best = {}  # thread (or the document itself) -> its best-ranked document, in rank order
        for n, thread in self.db.execute(sql, params):
            best.setdefault(thread or f"#{n}", (n, thread))
        out = []
        for n, thread in list(best.values())[:int(limit)]:
            source, kind, day, title, text, file = self.db.execute(
                "select source, kind, day, title, text, file from docs where n = ?", (n,)).fetchone()
            count, first, last = self._thread_span(thread) if thread else (1, day, day)
            out.append({"source": source, "kind": kind, "day": day, "title": title, "snippet": snippet(text, q),
                        "ref": file, "count": count, "first": first or day, "last": last or day})
        return out

    def stats(self):
        by = dict(self.db.execute("select source, count(*) from docs group by source"))
        return {"docs": sum(by.values()), "by_source": by, "files": self.db.execute("select count(*) from files").fetchone()[0]}
