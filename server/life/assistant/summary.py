"""The day summary: prompt, filler filtering, time ordering, and writing the draft."""

import re

from life.assistant.config import VAULT
from life.assistant.context import build_context
from life.assistant.diaryfile import render
from life.assistant import llm


SYSTEM = """너는 사용자의 개인 비서다. 오늘 수집된 기록을 보고 하루를 정리해서 사용자에게 말하듯 전한다.
규칙:
- 기록에 없는 것은 쓰지 않는다. 추측은 "~로 보여요", "~한 것 같아요"로 표시하고 단정하지 않는다.
- 직함, 감정, 관계는 기록에 적힌 그대로만 쓴다. 기록에 없는 직함을 붙이지 않는다.
- 다른 사람이 보낸 메시지나 한 말은 사용자가 한 말로 쓰지 않는다. "~님이 ~라고 하셨어요"처럼 주체를 구분한다.
- 대화 전사는 사용자가 본인 대화로 확인한 것이지만 오류가 있다. 상대의 사적인 내용(가족, 건강 등)은 자세히 옮기지 않는다.
- 하루의 흐름이 보이게 쓴다. 오전, 오후, 저녁 순서로 이어지는 5~7개의 짧은 문장(각각 한 줄)이다. 같은 종류의 일(메일 여러 통 등)은 한 문장으로 묶고, 가장 의미 있는 일(통화, 일정, 결정이 필요한 요청)을 먼저 구체적으로 쓴다.
- "[내가 보냄]"이 붙은 메일은 사용자가 보낸 것이다("~에게 메일을 보내셨어요"). 받은 메일과 구분한다. "[대량 발송]"이 붙은 메일, 광고, 시스템 알림은 쓰지 않는다. 앱 사용 횟수나 장소 이동 횟수는 쓰지 않는다.
- 쓰지 않는 빈말: "다양한 앱을 활용하셨어요", "여러 곳을 이동하셨어요", "업무 관련 대화를 나누셨어요"처럼 무엇인지 알 수 없는 문장. 구체적인 주제를 모르면 그 문장을 빼거나 "동료분들과 연락이 오갔어요"처럼 사실만 쓴다.
- 메신저 알림은 제목보다 내용을 근거로 한다. 점심·잡담은 업무로 쓰지 않고 한 줄 이내로 쓰거나 뺀다.
- 시각은 기록에 적힌 시각을 그대로 옮긴다. 24시간제를 오전/오후로 바꿀 때 틀리지 않게 한다(19:15 = 저녁 7시 15분, 08:34 = 오전 8시 34분).\n- 정리는 존댓말로 비서가 말하듯 쓰고 "~했어요", "~셨어요", "~보여요"로 끝낸다. 글머리 기호를 붙이지 않는다.
- 질문은 하나만. 오늘 가장 의미 있는 일(통화, 일정 변경, 결정이 필요한 요청) 하나를 골라 "그때 어떠셨어요?"를 묻는 식으로, 사용자의 마음이나 선택 이유를 부드러운 존댓말 한 문장으로 묻는다. 다른 사람의 말을 사용자의 말로 단정하는 질문은 하지 않는다.
JSON만 출력: {"summary": "문장1\\n문장2\\n...", "question": "..."}"""


FILLER = re.compile(r"(다양한 앱|앱과 메신저를 활용|곳의 장소|장소를 이동|여러 곳을 이동|업무 관련 대화를 나누|메시지를 주고받으셨어요$)")


_CLOCK = re.compile(r"(오전|오후|저녁|밤|아침|점심|정오|새벽)?\s*(\d{1,2})\s*시\s*(?:(\d{1,2})\s*분)?")


def _minutes(line, fallback):
    """Minutes since midnight of the first clock time in the sentence; 점심/정오 without a number -> 12:00. Unknown -> fallback."""
    m = _CLOCK.search(line)
    if not m:
        return 12 * 60 if re.search(r"점심|정오", line) else fallback
    part, h, mi = m.group(1), int(m.group(2)), int(m.group(3) or 0)
    if part in ("오후", "저녁", "밤") and h < 12:
        h += 12
    return h * 60 + mi


def drop_filler(summary):
    """Remove filler sentences and put the rest in time order (the model often mixes the order)."""
    lines = [l for l in summary.splitlines() if l.strip() and not FILLER.search(l)]
    keyed, last = [], 0
    for i, l in enumerate(lines):
        last = _minutes(l, last)
        keyed.append((last, i, l))
    return "\n".join(l for _, _, l in sorted(keyed))


def ask_llm(context, day):
    text = llm.complete(
        [{"role": "system", "content": SYSTEM}, {"role": "user", "content": f"날짜: {day}\n\n{context}"}],
        temperature=0.3, max_tokens=1500, json_mode=True, timeout=180,
    )
    data = llm.parse_json(text)
    data["summary"] = drop_filler(data.get("summary", "")) or data.get("summary", "")
    return data


def generate(day, dry_run=False):
    """Build the day's draft with the local model and write it into the diary note. Returns (text, path or None)."""
    ai = ask_llm(build_context(day, summary=True), day)
    out = VAULT / "Diary" / f"{day}.md"
    text = render(day, ai, out.read_text(encoding="utf-8") if out.exists() else "")
    if dry_run:
        return text, None
    out.parent.mkdir(mode=0o700, exist_ok=True)
    out.write_text(text, encoding="utf-8")
    out.chmod(0o600)
    return text, out
