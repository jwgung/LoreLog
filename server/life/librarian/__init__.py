"""The librarian (사서): an index over every record, so the assistant can find things across all days.

    with Librarian() as lib:
        lib.update()                      # index new and changed records, forget removed ones
        lib.search("거래처 견적")          # -> [{source, kind, day, title, snippet, ref, count, first, last}]

Best matches first; the mails of one thread come back as one hit with the thread's size (count) and span (first,
last). It finds and does not judge: records that disagree come back together. Only records the diary rules allow are
indexed (see records.py). The index is a rebuildable copy at <data>/librarian.sqlite.
"""
from life.librarian.index import Librarian, terms

__all__ = ["Librarian", "terms"]
