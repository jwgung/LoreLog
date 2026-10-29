"""The day's diary note: the AI part, the user's corrections and reflection, and their raw copy."""

import datetime as dt
import json
import re

from life.assistant.config import AI_BEGIN, AI_END, ALLOW_FILE, DATA, VAULT


USER_PART = """
## 정정
<!-- 틀린 줄이 있으면 "틀린 것 → 맞는 것" 한 줄씩 -->

## 회고
<!-- 오늘 어땠는지 자유롭게 -->
"""

# A correction said in chat is stored as one user line: the summary sentence it corrects, then what the user said.
_CORRECTION = '(정리의 이 문장이 틀렸다고 알려 주셨어요: "{wrong}")\n{message}'
_CORRECTION_RE = re.compile(r'^\(정리의 이 문장이 틀렸다고 알려 주셨어요: "(.*?)"\)\n(.*)$', re.S)


def wrap_correction(wrong, message):
    return _CORRECTION.format(wrong=wrong, message=message)


def split_correction(text):
    """(wrong sentence, what the user said) for a stored correction line, else None."""
    m = _CORRECTION_RE.match(text or "")
    return (m.group(1), m.group(2)) if m else None


def section(text, name):
    """Body of '## <name>' up to the next '## ' heading or the end, without the placeholder comment."""
    m = re.search(rf"^## {re.escape(name)}[ \t]*\n(.*?)(?=^## |\Z)", text, re.M | re.S)
    return re.sub(r"<!--.*?-->", "", m.group(1), flags=re.S).strip() if m else ""


def set_section(text, name, body):
    pat = re.compile(rf"(^## {re.escape(name)}[ \t]*\n).*?(?=^## |\Z)", re.M | re.S)
    new = body.strip() + "\n\n" if body.strip() else ""
    if pat.search(text):
        return pat.sub(lambda m: m.group(1) + new, text, count=1)
    return text.rstrip("\n") + f"\n\n## {name}\n{new}"


def read_day(day):
    out = VAULT / "Diary" / f"{day}.md"
    text = out.read_text(encoding="utf-8") if out.exists() else ""
    m = re.search(re.escape(AI_BEGIN) + "(.*?)" + re.escape(AI_END), text, re.S)
    if not m:
        return {"date": day, "exists": False}
    ai = m.group(1)
    g = re.search(r"^ai_generated:[ \t]*(.+)$", text, re.M)
    stale = True
    if g:
        generated = dt.datetime.fromisoformat(g.group(1).strip()).timestamp()
        inputs = [DATA / "raw/lorelog" / f"{day}.ndjson", ALLOW_FILE]
        stale = any(p.exists() and p.stat().st_mtime > generated for p in inputs)
    summary = [l[2:].strip() if l.startswith("- ") else l.strip() for l in section(ai, "오늘의 정리 (ai)").splitlines() if l.strip()]
    return {
        "date": day, "exists": True, "stale": stale, "summary": summary,
        "question": section(ai, "오늘의 질문 (ai)"),
        "corrections": section(text, "정정"), "reflection": section(text, "회고"),
    }


def save_user(day, payload):
    """Write the user's corrections and reflection into the day's note and append a raw copy."""
    out = VAULT / "Diary" / f"{day}.md"
    text = out.read_text(encoding="utf-8") if out.exists() else f"---\ntype: diary\ndate: {day}\nclass: personal\n---\n# {day}\n"
    lines = [f"- {c['wrong'].strip()} → {c['right'].strip()}" for c in payload.get("corrections", []) if c.get("wrong") and c.get("right")]
    reflection = payload.get("reflection", "")
    if payload.get("append"):  # keep what was said earlier today; add the new words after it
        old_fix, old_ref = section(text, "정정"), section(text, "회고")
        lines = ([old_fix] if old_fix else []) + lines
        reflection = "\n\n".join(x for x in (old_ref, reflection) if x)
    text = set_section(text, "정정", "\n".join(lines))
    text = set_section(text, "회고", reflection)
    out.parent.mkdir(mode=0o700, exist_ok=True)
    out.write_text(text, encoding="utf-8")
    out.chmod(0o600)
    raw = DATA / "raw/diary"
    raw.mkdir(parents=True, exist_ok=True)
    rec = {"type": "reflection", "ts": int(dt.datetime.now().timestamp() * 1000), "date": day,
           "corrections": payload.get("corrections", []), "reflection": payload.get("reflection", ""),
           "_src": "diary", "_class": "personal"}
    f = raw / f"{day}.ndjson"
    with open(f, "a", encoding="utf-8") as fh:
        fh.write(json.dumps(rec, ensure_ascii=False, separators=(",", ":")) + "\n")
    f.chmod(0o600)


def render(day, ai, existing):
    stamp = dt.datetime.now().astimezone().isoformat(timespec="seconds")
    block = f"{AI_BEGIN}\n## 오늘의 정리 (ai)\n{chr(10).join('- ' + l.strip().lstrip('- ').strip() for l in ai['summary'].strip().splitlines() if l.strip())}\n\n## 오늘의 질문 (ai)\n{ai['question'].strip()}\n{AI_END}"
    if existing and AI_BEGIN in existing:
        text = re.sub(re.escape(AI_BEGIN) + ".*?" + re.escape(AI_END), lambda _: block, existing, flags=re.S)
        if re.search(r"^ai_generated:.*$", text, re.M):
            return re.sub(r"^ai_generated:.*$", f"ai_generated: {stamp}", text, count=1, flags=re.M)
        return text.replace("\nclass: personal\n", f"\nclass: personal\nai_generated: {stamp}\n", 1)
    return f"---\ntype: diary\ndate: {day}\nclass: personal\nai_generated: {stamp}\n---\n# {day}\n\n{block}\n{USER_PART}"
