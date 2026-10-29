"""Everything the assistant reads about a day: mail, events, calls, allowed messenger notifications and confirmed voice."""

import collections
import datetime as dt
import json
import re

from life.assistant.config import DATA, NOISE_PKGS, VAULT
from life.assistant.senders import is_allowed, load_allow


def front(path, keys):
    head = path.read_text(encoding="utf-8", errors="replace")[:3000]
    out = {}
    for k in keys:
        m = re.search(rf"^{k}:[ \t]*(.*)$", head, re.M)
        out[k] = m.group(1).strip().strip('"') if m else ""
    return out


def local(ts_ms):
    return dt.datetime.fromtimestamp(int(ts_ms) / 1000)


def gather_events(day):
    rows = []
    for p in (VAULT / "Events").glob(f"{day}*.md"):
        f = front(p, ["title", "start", "end", "all_day"])
        rows.append(f"{f['start'][11:16] or '종일'}~{f['end'][11:16]} {f['title']}")
    return sorted(rows)


def _subj_key(subject):
    """Subject without RE:/FW: prefixes, reduced to letters and digits, for matching a reply to its request."""
    sub = re.sub(r"^(\s*(re|fw|fwd|회신|전달)\s*:\s*)+", "", subject or "", flags=re.I)
    return re.sub(r"[^0-9a-z가-힣]", "", sub.lower())


def sent_index(days=21):
    """Mail the user sent in the last days: [(subject_key, in_reply_to, epoch_seconds)]. Only the frontmatter is read."""
    out, cutoff = [], dt.date.today() - dt.timedelta(days=days)
    for p in (VAULT / "Mail").glob("*/*.md"):
        m = re.match(r"(\d{4}-\d{2}-\d{2})-(\d{2})(\d{2})-", p.name)
        if not m or dt.date.fromisoformat(m.group(1)) < cutoff:
            continue
        f = front(p, ["mail_source", "subject", "in_reply_to"])
        if "Sent" not in f["mail_source"]:
            continue
        when = dt.datetime.fromisoformat(f"{m.group(1)}T{m.group(2)}:{m.group(3)}:00").timestamp()
        out.append((_subj_key(f["subject"]), f["in_reply_to"].strip("<>"), when))
    return out


def replied(sent, subject, mail_id, received_at):
    """True if the user sent a reply to this mail after it arrived (same thread subject, or an in-reply-to pointing at it)."""
    key, mid = _subj_key(subject), (mail_id or "").strip("<>")
    return any(w >= received_at and ((key and k == key) or (mid and r == mid)) for k, r, w in sent)


BULK_RE = re.compile(r"(no-?reply|newsletter|news@|info@|support@|marketing|promo|stibee|mailchimp|광고|뉴스레터|\(광고\))", re.I)


def gather_mail(day):
    """Mail of the day. Marked, not removed: bulk senders, mail the user sent, mail the user already replied to.
    'RE: RE:' chains collapse to one row."""
    rows, seen, sent = [], set(), sent_index()
    for p in (VAULT / "Mail").glob(f"*/{day}-*.md"):
        f = front(p, ["time", "from", "subject", "mail_source", "mail_id"])
        subj = re.sub(r"^(\s*(re|fw|fwd|회신|전달)\s*:\s*)+", "", f["subject"], flags=re.I).strip()
        key = (f["from"], subj)
        if key in seen:
            continue
        seen.add(key)
        if "Sent" in f["mail_source"]:
            mark = " [내가 보냄]"
        elif BULK_RE.search(f["from"] + " " + f["subject"]):
            mark = " [대량 발송]"
        else:
            hh, mm = (f["time"] or "00:00").split(":")[:2]
            received = dt.datetime.fromisoformat(f"{day}T{int(hh):02d}:{int(mm):02d}:00").timestamp()
            mark = " [이미 회신함]" if replied(sent, f["subject"], f["mail_id"], received) else ""
        rows.append(f"{f['time']} {f['from']} — {subj}{mark}")
    return sorted(rows)


def load_conversations():
    """Latest state of every LoreLog conversation (an updated status arrives as a new line)."""
    convs = {}
    for path in sorted((DATA / "raw/lorelog").glob("*.ndjson")):
        for line in path.read_text(encoding="utf-8").splitlines():
            try:
                d = json.loads(line)
            except ValueError:
                continue
            if d.get("type") == "conversation":
                convs[d["id"]] = d
    return convs


def confirmed_voice_label(convs, ts_ms):
    """Participants of the conversation this voice belongs to if the user confirmed it as theirs, else None.
    ambient / pending / skipped conversations and voice outside any conversation are never used."""
    for c in convs.values():
        if c.get("status") in ("tagged", "auto") and int(c["start_ms"]) <= ts_ms <= int(c["end_ms"]):
            return c.get("participants") or "?"
    return None


def gather_lorelog(day):
    path = DATA / "raw/lorelog" / f"{day}.ndjson"
    out = {"voice": [], "comm": [], "notif": collections.Counter(), "msgs": collections.defaultdict(list), "apps": collections.Counter(), "places": []}
    if not path.exists():
        return out
    places = []
    convs = load_conversations()
    allow = load_allow()
    for line in path.read_text(encoding="utf-8").splitlines():
        try:
            d = json.loads(line)
        except ValueError:
            continue
        t = d.get("type")
        if t == "voice" and d.get("text"):
            who = confirmed_voice_label(convs, int(d["ts"]))
            if who is not None:
                out["voice"].append(f"{local(d['ts']):%H:%M} (with {who}) {d['text'][:200]}")
        elif t == "comm":
            out["comm"].append(f"{local(d['ts']):%H:%M} {d.get('kind')}/{d.get('direction')} {d.get('party_name') or d.get('party')} {d.get('content', '')}")
        elif t == "notification" and d.get("pkg") not in NOISE_PKGS and is_allowed(allow, d.get("label"), d.get("title")):
            key = f"{d.get('label')}: {d.get('title')}"
            out["notif"][key] += 1
            if d.get("text"):
                out["msgs"][key].append(f"{local(d['ts']):%H:%M} {d['text'][:80]}")
        elif t == "app" and d.get("pkg") not in NOISE_PKGS:
            out["apps"][d.get("label")] += 1
        elif t == "location":
            places.append((local(d["ts"]), round(float(d["lat"]), 3), round(float(d["lon"]), 3)))
    # collapse consecutive points in the same ~100 m cell into stays
    stay = None
    for ts, la, lo in sorted(places):
        if stay and stay[2:] == (la, lo):
            stay[1] = ts
        else:
            stay = [ts, ts, la, lo]
            out["places"].append(stay)
    if out["places"]:
        cells = {(la, lo) for _, _, la, lo in out["places"]}
        out["places"] = [f"위치 기록 {out['places'][0][0]:%H:%M}~{out['places'][-1][1]:%H:%M}, 서로 다른 장소 {len(cells)}곳 (이름 없음)"]
    return out


def gather_pc(day, top=12):
    """What the user worked in on the PC (life-agent): the longest stays, same app and title merged across short breaks."""
    path = DATA / "raw/pc" / f"{day}.ndjson"
    if not path.exists():
        return []
    segs = []
    for line in path.read_text(encoding="utf-8").splitlines():
        try:
            d = json.loads(line)
        except ValueError:
            continue
        if d.get("type") == "window" and d.get("app") and d.get("ts") and d.get("end"):
            segs.append(d)
    merged = []
    for d in sorted(segs, key=lambda x: x["ts"]):
        m = merged[-1] if merged else None
        if m and m["app"] == d["app"] and m["title"] == d["title"] and d["ts"] - m["end"] <= 5 * 60 * 1000:
            m["end"], m["secs"] = d["end"], m["secs"] + d["dur_s"]
        else:
            merged.append({"app": d["app"], "title": d["title"], "ts": d["ts"], "end": d["end"], "secs": d["dur_s"]})
    longest = sorted((m for m in merged if m["secs"] >= 300), key=lambda m: -m["secs"])[:top]
    return [f"{local(m['ts']):%H:%M}~{local(m['end']):%H:%M} {m['app']} \"{m['title'][:80]}\" ({m['secs'] // 60}분)"
            for m in sorted(longest, key=lambda m: m["ts"])]


def build_context(day, summary=False):
    lore = gather_lorelog(day)
    parts = [
        ("일정", gather_events(day)),
        ("받거나 보낸 메일(제목만)", gather_mail(day)[:60]),
        ("통화/문자", lore["comm"]),
        ("대화(사용자가 본인 대화로 확인한 음성 전사, 오류 있음)", lore["voice"][:30]),
        ("PC에서 5분 넘게 머문 창(앱, 창 제목, 시간)", gather_pc(day)),
        ("메신저 알림(사용자가 허용한 상대만, 건수: 최근 내용)", [f"{k} x{v}: " + " / ".join(lore["msgs"][k][-4:]) for k, v in lore["notif"].most_common(15)]),
        ("앱 사용(기록 횟수)", [f"{k} x{v}" for k, v in lore["apps"].most_common(12)]),
        ("위치", lore["places"][:20]),
    ]
    if summary:  # 장소 이동 횟수·앱 사용 횟수는 정리를 "다양한 앱을 활용하셨어요" 같은 빈말로 채웠다. 재료에서는 두지 않는다.
        parts = [(n, r) for n, r in parts if n not in ("앱 사용(기록 횟수)", "위치")]
    return "\n".join(f"## {name}\n" + ("\n".join(f"- {r}" for r in rows) if rows else "- (없음)") for name, rows in parts)
