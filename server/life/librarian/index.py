"""The librarian's index: SQLite FTS5 over every document, kept in step with the files it came from.

Korean is indexed in two-letter pieces ("거래처랑" -> 거래 래처 처랑) so a word is found inside a longer word with a
particle attached, and a two-letter word like "견적" is found at all; FTS5's own tokenizers do neither. A query word
becomes a phrase of its pieces (so the pieces must be adjacent); a Korean query word ending in a particle also tries
the word without it, a single Korean letter becomes a prefix, and an English word of three letters or more matches
longer words that start with it. The words of a query are joined with OR and ranked by bm25: recall first, the best
matches on top.

Only what changed is read again. Phone files are the exception that needs care: whether a day's voice is the user's
depends on conversation states that can arrive in another day's file. The index keeps every conversation state it has
read (table convs, until its file is removed; a phone file that cannot be read again keeps the states last read from it,
since the raw files only grow); when a state changes, the days that conversation spans are read again, and nothing else (a span
longer than records.MAX_CONVERSATION_DAYS cannot be believed, so then every phone file is read again). When the
allowed senders change, every phone file is read again.

Every update takes the write lock first (BEGIN IMMEDIATE) and only then lists the files and reads the allowed senders,
so `life sync` and a chat request never work from the same stale picture. A search reads one snapshot of the index.
An index of another version is not searched (it may hold what the current rules leave out); the next update empties
and fills it in one transaction. A file that cannot be read (or holds a record that cannot be understood) is skipped
and its old documents are dropped, so what the index answers is never older than the rules it was built under; the
next update tries the file again. The index is a copy built from the records: `reset` empties it, `update` builds it
again.
"""
import json
import os
import re
import sqlite3
import sys
import unicodedata
from pathlib import Path

from life.assistant.config import DATA
from life.assistant.context import conversations_in, read_records
from life.librarian import records

WORD_RE = re.compile(r"[0-9a-z]+|[가-힣\u4e00-\u9fff]+")
# particles and endings a spoken question puts after a word ("견적서는", "여수항만에서"); longest first
PARTICLES = sorted(["은", "는", "이", "가", "을", "를", "의", "에", "에서", "에게", "께", "한테", "로", "으로", "와", "과",
                    "랑", "이랑", "하고", "도", "만", "까지", "부터", "처럼", "보다", "이나", "나", "요", "야", "란", "이란",
                    "라는", "이라는", "이랑은", "에서는", "에는", "으로는", "에도", "에서도", "으로도", "로도", "한테도",
                    "에게도"], key=len, reverse=True)
MAX_QUERY_WORDS = 16
MAX_WORD_HITS = 2  # the user's own words (often past questions with the same words) may take this many slots at most
SNIPPET_BEFORE, SNIPPET_AFTER = 60, 140
WINDOW = 400  # matches looked at per search before the mails of one thread are folded together
SCHEMA_VERSION = "6"  # bump when the tables or what goes in them change: an index of another version is built again

TABLES = [
    "create table if not exists meta(key text primary key, value text)",
    "create table if not exists files(path text primary key, kind text, stamp text)",
    "create table if not exists docs(n integer primary key, file text, source text, kind text, day text, title text,"
    " thread text, text text)",
    "create index if not exists docs_file on docs(file)",
    "create index if not exists docs_thread on docs(thread)",
    "create table if not exists convs(file text, seq integer, id text, data text)",
    "create index if not exists convs_file on convs(file)",
    "create virtual table if not exists fts using fts5(terms)",
]


def default_path():
    return DATA / "librarian.sqlite"


class OutdatedIndex(RuntimeError):
    """The index was built by another version (other tables or other rules), or no update has finished yet: it is not
    searched until update() builds it, because it may hold what the current rules leave out, or nothing at all."""


# fullwidth letters, digits and signs (ＡＢＣ, ２０２４) read as their plain forms; one letter for one, so positions hold
FULLWIDTH = {o: o - 0xFEE0 for o in range(0xFF01, 0xFF5F)}


def _fold(text):
    """The text as it is matched: composed Hangul (NFC; a name sent from a Mac often arrives decomposed), fullwidth
    forms as plain ones, lower case. Only for matching: what is shown keeps the letters as written."""
    return unicodedata.normalize("NFC", text).translate(FULLWIDTH).lower()


def _pieces(word):
    return [word] if word.isascii() or len(word) == 1 else [word[i:i + 2] for i in range(len(word) - 1)]


def _stem(word):
    """The word without a trailing particle, if it has one and keeps two letters; else None."""
    if word.isascii():
        return None
    for p in PARTICLES:
        if word.endswith(p) and len(word) - len(p) >= 2:
            return word[:-len(p)]
    return None


def terms(text):
    """The index terms of a text, space separated."""
    return " ".join(p for w in WORD_RE.findall(_fold(text)) for p in _pieces(w))


def _phrase(w):
    if w.isascii():
        return f'"{w}"*' if len(w) >= 3 and not w.isdigit() else f'"{w}"'
    return f'"{w}"*' if len(w) == 1 else '"' + " ".join(_pieces(w)) + '"'


def to_query(q):
    """An FTS5 expression for the words of q (the first MAX_QUERY_WORDS of them, each with its stem), or "" if q has
    none. Every word is quoted, so nothing in q can act as an FTS5 operator or column filter."""
    words = list(dict.fromkeys(WORD_RE.findall(_fold(q))))[:MAX_QUERY_WORDS]
    parts = [_phrase(w) for w in words] + [_phrase(s) for s in map(_stem, words) if s]
    return " OR ".join(dict.fromkeys(parts))


def snippet(text, q):
    """The part of the text around the first word of the query (or its stem) that appears in it."""
    text = unicodedata.normalize("NFC", text)  # looks the same; lets the positions below match the shown text
    low = text.translate(FULLWIDTH).lower()
    words = [x for w in WORD_RE.findall(_fold(q)) for x in (w, _stem(w)) if x]
    at = [i for i in (low.find(w) for w in words) if i >= 0]
    start = max(0, min(at) - SNIPPET_BEFORE) if at else 0
    end = start + SNIPPET_BEFORE + SNIPPET_AFTER
    body = re.sub(r"\s+", " ", text[start:end]).strip()
    return ("…" if start > 0 else "") + body + ("…" if end < len(text) else "")


def _stamp(path):
    st = path.stat()
    return f"{st.st_mtime_ns}:{st.st_size}"


def _conversations_of(path):
    """The conversation records of a phone file, without decoding its other lines (most of a phone file is screen
    and app records)."""
    return conversations_in(read_records(path, only='"conversation"'))


def _day_file(path_str):
    return Path(path_str).stem


class Librarian:
    def __init__(self, path=None, timeout=30):
        self.path = Path(path) if path else default_path()
        self.path.parent.mkdir(mode=0o700, parents=True, exist_ok=True)  # like the rest of the data directory
        if not self.path.exists():  # create it private first so the WAL files SQLite adds inherit the mode
            os.close(os.open(self.path, os.O_CREAT | os.O_WRONLY, 0o600))
        self.db = sqlite3.connect(self.path, timeout=timeout)
        self.db.execute("pragma journal_mode=wal")
        if self._version() is None:  # a new file: make the tables. An index of another version is left as it is
            self._rebuild_tables(only_if_new=True)  # until update() empties and fills it in one transaction

    def close(self):
        self.db.close()

    def __enter__(self):
        return self

    def __exit__(self, *exc):
        self.close()

    def _version(self):
        try:
            row = self.db.execute("select value from meta where key = 'schema'").fetchone()
        except sqlite3.OperationalError:  # no tables yet
            return None
        return row[0] if row else None

    def _make_tables(self):
        """Drop every table and make them again; the caller holds the write lock."""
        for t in ("docs", "files", "convs", "fts", "meta"):
            self.db.execute(f"drop table if exists {t}")
        for stmt in TABLES:
            self.db.execute(stmt)
        self.db.execute("insert into meta(key, value) values ('schema', ?)", (SCHEMA_VERSION,))

    def _rebuild_tables(self, only_if_new=False):
        """Drop every table and make them again, under the write lock so two processes do not do it at once."""
        self.db.commit()
        self.db.execute("begin immediate")
        try:
            if only_if_new and self._version() is not None:  # another process made them while we waited
                self.db.commit()
                return
            self._make_tables()
            self.db.commit()
        except BaseException:
            self.db.rollback()
            raise

    def reset(self):
        """Empty the index (the file stays, so open connections keep working); the next update builds it again.
        To empty and fill it without a moment of an empty index in between, use update(reset=True)."""
        self._rebuild_tables()

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

    def _keep(self, path, kind, stamp, new):
        self._forget(path)
        for d in new:
            self._add(path, d)
        self.db.execute("insert or replace into files(path, kind, stamp) values (?, ?, ?)", (path, kind, stamp))

    def _skip(self, path, p, e):
        self._forget(path)
        self.db.execute("delete from files where path = ?", (path,))  # read it again next time
        sys.stderr.write(f"librarian: skipped {p.name}: {type(e).__name__}\n")

    def _convs(self):
        """The latest state of every conversation read so far (a later file, then a later line, wins)."""
        out = {}
        for cid, data in self.db.execute("select id, data from convs order by file, seq"):
            out[cid] = json.loads(data)
        return out

    def _changes(self, found, policy):
        """(files to read again, files gone). Every phone file is read again when the allowed senders changed."""
        known = {path: (kind, stamp) for path, kind, stamp in self.db.execute("select path, kind, stamp from files")}
        dirty = set()
        for path, (_, p) in list(found.items()):
            try:
                if known.get(path, (None, None))[1] != _stamp(p):
                    dirty.add(path)
            except FileNotFoundError:  # removed between listing and now (a sync is writing): treat it as gone
                del found[path]
        removed = [path for path in known if path not in found]
        if self._meta("policy") != policy:
            dirty |= {path for path, (kind, _) in found.items() if kind == "lorelog"}
        return dirty, removed

    def _update_phone(self, found, dirty, allow):
        """Keep the conversation states of the changed phone files, read again the days whose conversations changed,
        and index them all. Two passes, so memory holds one file at a time however many files are read again (a
        changed allow list reads them all): the first keeps only each file's stamp and conversation states, the
        second reads each file again and indexes it at once. A file that grew between the passes keeps the first
        stamp, so the next update reads it again. -> (indexed, skipped)"""
        phone_paths = {path for path, (kind, _) in found.items() if kind == "lorelog"}
        todo = {path for path in dirty if path in phone_paths}
        # files whose conversation states are kept but that are gone (also one skipped before it went, which has no
        # files row any more and so is not among `removed`)
        gone = [path for (path,) in self.db.execute("select distinct file from convs") if path not in phone_paths]
        if not todo and not gone:
            return 0, 0
        before = self._convs()
        stamps, skipped = {}, 0

        def take(paths):  # first pass: stamps and conversation states only
            nonlocal skipped
            for path in sorted(paths):
                p = found[path][1]
                try:
                    stamp, convs = _stamp(p), _conversations_of(p)
                except Exception as e:  # noqa: BLE001 - one unreadable file must not stop the rest
                    # its conversation states stay as last read: the raw files only grow, so they are still the best
                    # we know, and dropping them could bring back an older state (a voice the user took back)
                    self._skip(path, p, e)
                    skipped += 1
                    continue
                stamps[path] = stamp
                self.db.execute("delete from convs where file = ?", (path,))
                self.db.executemany("insert into convs(file, seq, id, data) values (?, ?, ?, ?)",
                                    [(path, i, str(c["id"]), json.dumps(c, ensure_ascii=False))
                                     for i, c in enumerate(convs)])

        for path in gone:
            self.db.execute("delete from convs where file = ?", (path,))
        take(todo)
        after = self._convs()
        # a conversation whose state changed (or appeared, or went) decides again about the voice on its days
        days, every_day = set(), False
        for cid in set(before) | set(after):
            if before.get(cid) != after.get(cid):
                for c in (before.get(cid), after.get(cid)):
                    if c:
                        span = records.conversation_days(c)
                        if span is None:  # too long to list its days: decide again everywhere
                            every_day = True
                        else:
                            days.update(span)
        by_day = {_day_file(path): path for path in phone_paths}
        wanted = set(phone_paths) if every_day else {by_day[d] for d in days if d in by_day}
        more = wanted - set(stamps) - todo
        if more:
            take(more)
            after = self._convs()
        indexed = 0
        for path in sorted(stamps):  # second pass: one file at a time
            p = found[path][1]
            try:
                new = records.lorelog_docs(_day_file(path), read_records(p), after, allow)
            except Exception as e:  # noqa: BLE001 - one file that cannot be read or understood must not stop the rest
                self._skip(path, p, e)  # its conversation states, just read, stay
                skipped += 1
                continue
            self._keep(path, "lorelog", stamps[path], new)
            indexed += 1
        return indexed, skipped

    def update(self, reset=False):
        """Index new and changed files, forget removed ones. With reset, or when the index is of another version,
        the index is emptied first, in the same transaction, so a search meanwhile sees the old index, never an empty
        one. The files and the allowed senders are read under the write lock: an update that committed while this
        one waited is never overwritten with what was there before it.
        -> {indexed, removed, skipped, docs}"""
        self.db.commit()
        self.db.execute("begin immediate")
        try:
            found = {str(p): (kind, p) for kind, p in records.sources()}
            allow = records.allowed()  # read once: the documents are made with it and it is what we remember
            policy = records.policy_stamp(allow)
            if reset or self._version() != SCHEMA_VERSION:
                self._make_tables()
            dirty, removed = self._changes(found, policy)
            for path in removed:
                self._forget(path)
                self.db.execute("delete from files where path = ?", (path,))
            indexed, skipped = self._update_phone(found, dirty, allow)
            for path in sorted(dirty):
                kind, p = found[path]
                if kind == "lorelog":
                    continue
                try:
                    stamp, new = _stamp(p), records.docs(kind, p)
                except Exception as e:  # noqa: BLE001 - one unreadable file must not stop the rest
                    self._skip(path, p, e)
                    skipped += 1
                    continue
                self._keep(path, kind, stamp, new)
                indexed += 1
            self.db.execute("insert or replace into meta(key, value) values ('policy', ?)", (policy,))
            self.db.commit()
        except BaseException:
            self.db.rollback()
            raise
        count = self.db.execute("select count(*) from docs").fetchone()[0]
        return {"indexed": indexed, "removed": len(removed), "skipped": skipped, "docs": count}

    def _thread_span(self, thread):
        return self.db.execute("select count(*), min(day), max(day) from docs where thread = ?", (thread,)).fetchone()

    def search(self, q, limit=10, exclude_day=None, exclude_words=None):
        """Documents matching the words of q, best first. The mails of one thread come back as one hit (its
        best-matching mail) with the thread's size and the days it spans, so a long thread does not take every slot;
        the user's own words take at most MAX_WORD_HITS slots unless nothing else is found. exclude_day leaves out
        that day's documents; with exclude_words, only that day's words of the user that are among them (lines said in
        the chat, as the chat log holds them). The search reads one snapshot of the index, so an update that lands
        meanwhile changes nothing in it. Raises OutdatedIndex for an index of another version.
        -> [{source, kind, day, title, snippet, ref, count, first, last}]"""
        expr = to_query(q or "")
        if not expr:
            return []
        self.db.commit()
        self.db.execute("begin")  # one read snapshot (WAL) for every query below
        try:
            if self._version() != SCHEMA_VERSION:
                raise OutdatedIndex(f"index version {self._version()}, expected {SCHEMA_VERSION}: run update")
            if self._meta("policy") is None:  # no update has finished yet: empty, but not because nothing is there
                raise OutdatedIndex("index not built yet: run update")
            return self._search(expr, q, int(limit), exclude_day, exclude_words)
        finally:
            self.db.rollback()  # nothing was written; ends the snapshot

    def _search(self, expr, q, limit, exclude_day, exclude_words):
        sql = "select d.n, d.thread, d.source from fts join docs d on d.n = fts.rowid where fts match ?"
        params = [expr]
        if exclude_day and exclude_words is not None:
            texts = sorted({t for w in exclude_words for t in (w, w.strip(), records.word_text(w))})
            if texts:  # compared exactly: the documents hold these lines as said, stripped, or as a correction
                sql += (" and not (d.day = ? and d.source = 'word' and d.text in"
                        f" ({','.join('?' * len(texts))}))")
                params += [exclude_day, *texts]
        elif exclude_day:
            sql += " and d.day != ?"
            params.append(exclude_day)
        sql += " order by fts.rank limit ?"
        params.append(WINDOW)
        best = {}  # thread (or the document itself) -> its best-ranked document, in rank order
        for n, thread, source in self.db.execute(sql, params):
            best.setdefault(thread or f"#{n}", (n, thread, source))
        chosen, spill, words = [], [], 0
        for g in best.values():
            if len(chosen) >= limit:
                break
            if g[2] == "word":
                if words >= MAX_WORD_HITS:
                    spill.append(g)
                    continue
                words += 1
            chosen.append(g)
        chosen += spill[:max(0, limit - len(chosen))]
        out = []
        for n, thread, _ in chosen:
            source, kind, day, title, text, file = self.db.execute(
                "select source, kind, day, title, text, file from docs where n = ?", (n,)).fetchone()
            count, first, last = self._thread_span(thread) if thread else (1, day, day)
            out.append({"source": source, "kind": kind, "day": day, "title": title, "snippet": snippet(text, q),
                        "ref": file, "count": count, "first": first or day, "last": last or day})
        return out

    def stats(self):
        """How much is indexed; outdated is True for an index of another version (not searched until update, and
        its tables may not be these, so nothing is counted)."""
        if self._version() != SCHEMA_VERSION or self._meta("policy") is None:  # another version, or never filled
            return {"docs": 0, "by_source": {}, "outdated": True, "files": 0}
        by = dict(self.db.execute("select source, count(*) from docs group by source"))
        return {"docs": sum(by.values()), "by_source": by, "outdated": False,
                "files": self.db.execute("select count(*) from files").fetchone()[0]}
