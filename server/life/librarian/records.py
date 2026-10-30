"""Turn the records the user keeps into documents for the librarian, under the same rules the diary uses.

The raw records stay as they are; this only decides what the index may hold:
  mail, events          every one, body included (the user's own mailbox and calendar)
  phone (lorelog)       messenger notifications only from senders the user allowed; voice only inside conversations the
                        diary counts as the user's (the same rule the diary's context uses); calls/texts; app use.
                        Screen text is not indexed (the diary does not read it), nor location (only coordinates).
  PC                    window titles
  the user's words      what the user said in chat, reflections and corrections. The assistant's replies and its daily
                        summary are derived from records, not records, so they are not indexed.

A document is {source, kind, day, title, text} plus, when there is one, `extra` (searched but not shown: a mail's
sender, recipients and attachment names, an event's attendees) and `thread` (the first message id of a mail's
References header, else In-Reply-To, else its own id; replies of one thread come back as one hit, mails that only share
a subject do not). Traces (phone, PC) become one document per day and kind so the index does not grow by a line per app
switch; mail, events and words are one document each.

Phone files are read by the index itself (index.py), once per change: the same records give the conversation states
it keeps and the documents, so the two never come from different moments.
"""
import collections
import datetime as dt
import hashlib
import json
import re

from life.assistant.config import DATA, NOISE_PKGS, VAULT
from life.assistant.context import confirmed_voice_label, local, read_records
from life.assistant.diaryfile import split_correction
from life.assistant.senders import is_allowed, load_allow

DAY_RE = re.compile(r"\d{4}-\d{2}-\d{2}")
MSG_ID_RE = re.compile(r"[^\s<>,\[\]\"']+@[^\s<>,\[\]\"']+")
# frontmatter lines that only carry sync bookkeeping, not anything a person would search for
NOISE_KEYS = re.compile(r"^(type|mail_id|mail_state|mail_synced|references|in_reply_to|richtung|dav_uid|dav_source|dav_etag|dav_state):.*\n?", re.M)


def sources():
    """Every file the librarian reads: [(kind, path)], in a stable order."""
    out = [("mail", p) for p in (VAULT / "Mail").glob("*/*.md")]
    out += [("event", p) for p in (VAULT / "Events").glob("*.md")]
    for name in ("lorelog", "pc", "diary"):
        out += [(name, p) for p in (DATA / "raw" / name).glob("*.ndjson")]
    return sorted(out, key=lambda kp: str(kp[1]))


def allowed():
    """The senders the user allowed, read once per update: the documents and the remembered policy both use it."""
    return load_allow()


def policy_stamp(allow):
    """Fingerprint of the rules that decide what phone records may be indexed (the allowed senders)."""
    return hashlib.sha256(json.dumps(sorted(allow), ensure_ascii=False).encode()).hexdigest()


MAX_CONVERSATION_DAYS = 31  # a span longer than this is not believed; index.py then reads every phone file again


def conversation_days(c):
    """The days a conversation spans (the voice it can contain lies on these days); [] without a usable span,
    None when its days cannot be listed: longer than MAX_CONVERSATION_DAYS, or times that are numbers but no date
    (the diary's voice rule compares the numbers, so such a conversation can still hold voice on any day)."""
    try:
        start, end = int(c["start_ms"]), int(c["end_ms"])
    except (KeyError, TypeError, ValueError, OverflowError):
        return []
    try:
        first, last = local(start).date(), local(end).date()
    except (ValueError, OverflowError, OSError):
        return None
    if last < first:
        return [first.isoformat()]
    if (last - first).days >= MAX_CONVERSATION_DAYS:
        return None
    return [(first + dt.timedelta(days=i)).isoformat() for i in range((last - first).days + 1)]


def split_front(text):
    if text.startswith("---\n"):
        end = text.find("\n---", 4)
        if end != -1:
            return text[4:end + 1], text[end + 4:]
    return "", text


def front_value(front, key):
    m = re.search(rf"^{key}:[ \t]*(.*)$", front, re.M)
    return m.group(1).strip().strip('"').strip("'") if m else ""


def day_of(value, fallback_name):
    m = DAY_RE.match(str(value or "")) or DAY_RE.match(fallback_name)
    return m.group(0) if m else ""


def docs(kind, path):
    """The documents a mail, event, PC or words file gives the index (phone files go through lorelog_docs)."""
    if kind == "mail":
        return mail_docs(path)
    if kind == "event":
        return event_docs(path)
    if kind == "pc":
        return pc_docs(path)
    return word_docs(path)


def clean_body(body):
    """The mail body as a person reads it: without Obsidian comments (%% … %%) and the note's section headings."""
    body = re.sub(r"%%.*?%%", "", body, flags=re.S)
    body = re.sub(r"^#{1,6} (Notes|Message)[ \t]*$", "", body, flags=re.M)
    return re.sub(r"\n{3,}", "\n\n", body).strip()


def thread_of(front, path):
    """The thread a mail belongs to: the first id in References, else In-Reply-To, else the mail's own id."""
    for key in ("references", "in_reply_to", "mail_id"):
        ids = MSG_ID_RE.findall(front_value(front, key))
        if ids:
            return "mail:" + ids[0].lower()
    return "mail:" + path.name


def mail_docs(path):
    front, body = split_front(path.read_text(encoding="utf-8", errors="replace"))
    sent = "Sent" in front_value(front, "mail_source")
    return [{"source": "mail", "kind": "보낸 메일" if sent else "받은 메일",
             "day": day_of(front_value(front, "date"), path.name),
             "title": front_value(front, "subject") or "(제목 없음)",
             "thread": thread_of(front, path),
             "extra": NOISE_KEYS.sub("", front).strip(),  # who, to whom, attachments: searched, not shown
             "text": clean_body(body)}]


def event_docs(path):
    front, body = split_front(path.read_text(encoding="utf-8", errors="replace"))
    start, end, place = front_value(front, "start"), front_value(front, "end"), front_value(front, "location")
    when = f"{start[:16].replace('T', ' ')}~{end[11:16]}".strip("~")
    return [{"source": "event", "kind": "일정",
             "day": day_of(start, path.name),
             "title": front_value(front, "title") or path.stem,
             "extra": NOISE_KEYS.sub("", front).strip(),
             "text": "\n".join(x for x in (when, f"장소 {place}" if place else "", clean_body(body)) if x)}]


def _lines(path):
    return read_records(path)


def _hm(ts):
    try:
        return f"{local(ts):%H:%M}"
    except (TypeError, ValueError, OverflowError, OSError):
        return "--:--"


def _trace_docs(day, groups):
    return [{"source": "trace", "kind": kind, "day": day, "title": f"{day} {kind}", "text": "\n".join(lines)}
            for kind, lines in groups.items() if lines]


def _lorelog_line(d, groups, apps, convs, allow):
    t = d.get("type")
    if t == "notification" and d.get("pkg") not in NOISE_PKGS and is_allowed(allow, d.get("label"), d.get("title")):
        groups["메신저"].append(f"{_hm(d.get('ts'))} {d.get('label')}: {d.get('title')} — {d.get('text', '')}".rstrip(" —"))
    elif t == "comm":
        groups["통화/문자"].append(f"{_hm(d.get('ts'))} {d.get('kind')}/{d.get('direction')} "
                                 f"{d.get('party_name') or d.get('party') or ''} {d.get('content', '')}".strip())
    elif t == "voice" and d.get("text"):
        who = confirmed_voice_label(convs, int(d["ts"]))
        if who is not None:
            groups["대화"].append(f"{_hm(d.get('ts'))} (with {who}) {d['text']}")
    elif t == "app" and d.get("pkg") not in NOISE_PKGS and d.get("label"):
        apps[d["label"]] += 1


def lorelog_docs(day, lines, convs, allow):
    """The documents of one day's phone records, given the conversation states and the allowed senders.
    A line whose values have the wrong shape (a list for a name, a time too large to be one) is left out alone."""
    groups = {"메신저": [], "통화/문자": [], "대화": [], "앱": []}
    apps = collections.Counter()
    for d in lines:
        try:
            _lorelog_line(d, groups, apps, convs, allow)
        except (KeyError, TypeError, ValueError, OverflowError):
            continue
    if apps:
        groups["앱"].append(", ".join(f"{k} x{v}" for k, v in apps.most_common()))
    return _trace_docs(day, groups)


def pc_docs(path):
    seen, lines = set(), []
    for d in _lines(path):
        if d.get("type") == "window" and d.get("app"):
            key = (d.get("app"), d.get("title", ""))
            if key not in seen:
                seen.add(key)
                lines.append(f"{_hm(d.get('ts'))} {key[0]} {key[1]}".strip())
    return _trace_docs(path.stem, {"PC": lines})


def _word(kind, day, text):
    return {"source": "word", "kind": kind, "day": day, "title": kind, "text": text}


def word_text(said):
    """The text of the document a line the user said in the chat becomes (a correction reads "wrong → right")."""
    pair = split_correction(said)
    return f"{pair[0]} → {pair[1]}" if pair else said


def word_docs(path):
    """What the user said: chat lines, reflections, corrections. A chat line is also saved as a reflection record;
    the same words are indexed once."""
    out, said = [], set()
    records = list(_lines(path))
    for d in records:
        if d.get("type") == "chat" and d.get("role") == "user" and isinstance(d.get("text"), str) and d["text"]:
            day = day_of(d.get("date"), path.stem)
            pair = split_correction(d["text"])
            if pair:
                said.add(pair[1].strip())
                out.append(_word("정정", day, word_text(d["text"])))
            else:
                said.add(d["text"].strip())
                out.append(_word("대화에서 한 말", day, d["text"]))
    for d in records:
        if d.get("type") != "reflection":
            continue
        day = day_of(d.get("date"), path.stem)
        text = str(d.get("reflection") or "").strip()
        if text and text not in said:
            out.append(_word("회고", day, text))
        for c in d.get("corrections") or []:
            if isinstance(c, dict) and c.get("wrong") and c.get("right") and str(c["right"]).strip() not in said:
                out.append(_word("정정", day, f"{c['wrong']} → {c['right']}"))
    return out
