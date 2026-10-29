"""To-dos the assistant proposes from the records; the user finishes or dismisses them."""

import datetime as dt
import hashlib
import json
import re
import threading

from life.assistant.config import TODO_FILE
from life.assistant.context import _subj_key, build_context, sent_index
from life.assistant import llm


TODO_SYSTEM = """너는 사용자의 개인 비서다. 기록에서 **사용자가 해야 할 일**을 뽑는다.
규칙:
- 기록에 분명히 드러난 것만 뽑는다: 사용자에게 온 요청·문의·마감·약속. 짐작해서 만들지 않는다.
- "[이미 회신함]", "[내가 보냄]", "[대량 발송]"이 붙은 메일은 뽑지 않는다. 광고, 뉴스레터, 단순 알림, 이미 끝난 일도 뽑지 않는다.
- title은 20자 안팎의 짧은 한 줄로, "무엇을 하기"로 끝낸다. 사람은 "OO 님"으로 쓰고 직함·회사명은 뺀다.
  좋은 예: "한도윤 님 도입 문의에 답장하기", "컨설팅 추진일정 확인하고 회신하기", "워크숍 발표자료 보내기", "견적 검토 의견 전달하기".
  나쁜 예(중복): "OO 문의 회신에 답하기", "요청에 응하기", "관련 메일 확인하기".
- source는 메일, 통화, 메신저, 일정, 음성 중 하나.
- source가 메일이면 mail_subject에 기록의 메일 제목을 RE:/FW: 없이 그대로 적는다. 아니면 빈 문자열.
- due는 기록에 기한이 적혀 있을 때만 그 표현을 쓰고(예: "금요일까지"), 없으면 빈 문자열.
- 많아야 5개. 없으면 빈 목록.
JSON만 출력: {"todos": [{"title": "...", "source": "메일", "mail_subject": "", "due": ""}]}"""


_LOCK = threading.Lock()  # life-ingest serves requests on threads; load-modify-save must not interleave


def _norm(t):
    return re.sub(r"[^0-9a-z가-힣]", "", t.lower())


def load_todos():
    try:
        return json.loads(TODO_FILE.read_text(encoding="utf-8"))
    except (OSError, ValueError):
        return {"items": [], "last_extract": 0}


def save_todos(d):
    TODO_FILE.parent.mkdir(parents=True, exist_ok=True)
    tmp = TODO_FILE.with_suffix(".tmp")
    tmp.write_text(json.dumps(d, ensure_ascii=False, indent=1), encoding="utf-8")
    tmp.chmod(0o600)
    tmp.replace(TODO_FILE)


def todo_view():
    with _LOCK:
        d = load_todos()
        if todo_autoclose(d):
            save_todos(d)
    items = d["items"]
    return {
        "open": [i for i in items if i["status"] == "open"],
        "done": [i for i in items if i["status"] == "done"][-10:][::-1],
    }


def _add_todo(d, title, source, due, day, mail_subject=""):
    n = _norm(title)
    if not n or any(_norm(i["title"]) == n for i in d["items"]):  # a dismissed one never comes back
        return False
    d["items"].append({"id": hashlib.sha1(n.encode()).hexdigest()[:10], "title": title.strip(), "source": source, "due": due,
                       "status": "open", "day": day, "thread": _subj_key(mail_subject) if source == "메일" else "",
                       "created": int(dt.datetime.now().timestamp() * 1000)})
    return True


def todo_autoclose(d):
    """Mark open mail to-dos done when the user has since replied in the same thread (recorded as auto-closed)."""
    sent, closed = sent_index(), 0
    for i in d["items"]:
        if i["status"] == "open" and i.get("thread") and any(k == i["thread"] and w * 1000 >= i["created"] for k, _, w in sent):
            i["status"], i["auto"] = "done", True
            closed += 1
    return closed


def todo_extract(day, force=False):
    """Ask the local LLM for to-dos in the day's records (and yesterday's). Throttled to once per 30 minutes."""
    d = load_todos()
    now = dt.datetime.now().timestamp()
    if not force and now - d.get("last_extract", 0) < 1800:
        return 0
    prev = (dt.date.fromisoformat(day) - dt.timedelta(days=1)).isoformat()
    ctx = f"[어제 {prev}]\n{build_context(prev)}\n\n[오늘 {day}]\n{build_context(day)}"
    text = llm.complete(
        [{"role": "system", "content": TODO_SYSTEM}, {"role": "user", "content": ctx}],
        temperature=0.2, max_tokens=700, json_mode=True, timeout=180,
    )
    found = llm.parse_json(text).get("todos", [])
    with _LOCK:  # the model call above takes minutes: merge into the file as it is now, not as it was then
        d = load_todos()
        added = sum(_add_todo(d, t.get("title", ""), t.get("source", ""), t.get("due", ""), day, t.get("mail_subject", "")) for t in found[:5] if isinstance(t, dict))
        todo_autoclose(d)
        d["last_extract"] = now
        save_todos(d)
    return added


def todo_act(tid, action):
    with _LOCK:
        d = load_todos()
        for i in d["items"]:
            if i["id"] == tid:
                i["status"] = {"done": "done", "dismiss": "dismissed", "reopen": "open"}[action]
                save_todos(d)
                return True
    return False


def todo_add_user(text, day):
    with _LOCK:
        d = load_todos()
        ok = _add_todo(d, text, "내가 말한 것", "", day)
        save_todos(d)
    return ok
