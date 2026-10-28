"""The assistant's chat: what the user says is saved first, then answered by the local model."""

import datetime as dt
import json
import re

from life.assistant.config import DATA
from life.assistant.context import build_context
from life.assistant.diaryfile import read_day, save_user
from life.assistant import llm


CHAT_SYSTEM = """너는 사용자의 개인 비서다. 사용자가 방금 한 말에 짧게 답한다.
규칙:
- 존댓말로 한두 문장만 말한다. 길게 설명하거나 목록으로 쓰지 않는다.
- 사용자가 자기 상황이나 기분을 말하면 먼저 그 말을 짧게 받아 준다("정신없는 하루셨네요"처럼 들은 그대로). 진단하거나 조언하지 않는다. 오늘 기록과 이어지는 것이 분명할 때만 한 가지를 덧붙인다.
- 기록에 대해 물었을 때만 기록으로 답한다. 모르거나 기록에 없으면 그때 "기록에는 없어요"라고 말한다. 사용자의 말에 기록이 없다고 되받지 않는다.
- 사용자가 한 말은 그대로 존중한다. 기록과 다르면 사용자의 말이 맞다. 기록과 어긋나도 따지지 않는다.
- 감정이나 이유를 단정하거나 진단하지 않는다. 위로나 조언은 사용자가 원할 때만 짧게 한다.
- 사용자가 틀린 문장을 고쳐 주면 "알겠어요"로 받고, 고친 내용을 한 줄로 다시 확인한다.
- 질문은 꼭 필요할 때만 하나만 한다. 같은 질문을 반복하지 않는다.
- 다른 사람의 사생활을 새로 꺼내거나 추측하지 않는다."""


def chat_log(day):
    path = DATA / "raw/diary" / f"{day}.ndjson"
    out = []
    if path.exists():
        for line in path.read_text(encoding="utf-8").splitlines():
            try:
                d = json.loads(line)
            except ValueError:
                continue
            if d.get("type") == "chat":
                out.append(d)
    return out


def append_chat(day, role, text):
    raw = DATA / "raw/diary"
    raw.mkdir(parents=True, exist_ok=True)
    f = raw / f"{day}.ndjson"
    rec = {"type": "chat", "ts": int(dt.datetime.now().timestamp() * 1000), "date": day, "role": role, "text": text,
           "_src": "diary", "_class": "personal"}
    with open(f, "a", encoding="utf-8") as fh:
        fh.write(json.dumps(rec, ensure_ascii=False, separators=(",", ":")) + "\n")
    f.chmod(0o600)


def chat_history(day):
    """Today's exchange for the app to show again: [{role, text, wrong?}]. A correction is stored with a wrapper line; unwrap it."""
    out = []
    for t in chat_log(day):
        text, wrong = t["text"], None
        m = re.match(r'^\(정리의 이 문장이 틀렸다고 알려 주셨어요: "(.*?)"\)\n(.*)$', text, re.S)
        if t["role"] == "user" and m:
            wrong, text = m.group(1), m.group(2)
        out.append({"role": t["role"], "text": text, **({"wrong": wrong} if wrong else {})})
    return out


def chat_reply(day, message, wrong=None):
    """Save what the user said (a correction or a word for the diary), then answer it with the local LLM."""
    draft = read_day(day)
    if wrong:
        save_user(day, {"append": True, "corrections": [{"wrong": wrong, "right": message}]})
    else:
        save_user(day, {"append": True, "reflection": message})
    head = CHAT_SYSTEM + "\n\n[오늘 기록]\n" + build_context(day)
    if draft.get("exists"):
        head += "\n\n[비서가 오늘 정리해서 전한 말]\n" + "\n".join(draft["summary"])
    msgs = [{"role": "system", "content": head}]
    if draft.get("exists") and draft.get("question"):
        msgs.append({"role": "assistant", "content": draft["question"]})
    for t in chat_log(day)[-12:]:
        msgs.append({"role": t["role"], "content": t["text"]})
    user = message if not wrong else f'(정리의 이 문장이 틀렸다고 알려 주셨어요: "{wrong}")\n{message}'
    msgs.append({"role": "user", "content": user})
    reply = llm.complete(msgs, temperature=0.5, max_tokens=300, timeout=120).strip()
    append_chat(day, "user", user)
    append_chat(day, "assistant", reply)
    return reply
