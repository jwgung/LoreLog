"""Before the assistant answers, it asks the librarian.

When the user asks or talks about something past, the model picks search words (the nouns a record would contain, in
Korean and in the original English for loanwords), the librarian searches every day but today, and what it finds goes
into the reply prompt next to today's records. Today's records are already there in full. Before searching the index
is brought up to date (only changed files are read again, about 50 ms when nothing changed), so mail that just
arrived is findable.

Nothing here may break the chat: if the model cannot pick words, the user's own words are searched; if the index
cannot be read, the prompt says the librarian could not look, which is not the same as "not found".
"""
import sys

from life.assistant import llm
from life.librarian import Librarian

WORDS_SYSTEM = """사용자의 말에서 지난 기록을 찾아볼 검색어를 고른다.
- 사용자가 지난 일, 사람, 회사, 물건, 일정, 메일, 약속을 묻거나 말하면 search 를 true 로 하고, 기록에 그대로 나올 만한 낱말(이름, 회사, 품목, 장소, 주제)을 1~5개 고른다. 조사와 어미는 뺀다.
- 회사·제품·기술 이름이나 외래어는 기록에 영어 원문으로 적혀 있을 수 있다. 한글 표기와 영어 원문을 둘 다 넣는다(예: "트렌드마이너" → "트렌드마이너", "TrendMiner"; "브로커" → "브로커", "broker").
- 기분이나 상태만 말하거나, 인사나 맞장구면 search 를 false 로 한다.
JSON 으로만 답한다: {"search": true 또는 false, "words": ["낱말", ...]}"""

LIMIT = 8
MAX_WORDS = 10
MAX_SNIPPET = 200
LOCK_WAIT_S = 5  # if `life sync` is writing the index, wait this long at most; the chat must not hang
HEADER = "[사서가 찾은 지난 기록]"
NOT_FOUND = "- (찾지 못함: 지난 기록에서 이 말과 맞는 것을 찾지 못했다)"
UNAVAILABLE = "- (찾아보지 못함: 지금은 사서의 색인을 읽을 수 없다. 기록이 없다는 뜻이 아니다)"
PARTIAL = "- (일부 기록 파일을 지금 읽지 못해 빠진 것이 있을 수 있다)"


def search_words(message):
    """Words to search for this message, or None if there is nothing to look up."""
    try:
        text = llm.complete([{"role": "system", "content": WORDS_SYSTEM}, {"role": "user", "content": message}],
                            temperature=0, max_tokens=80, json_mode=True, timeout=60)
        d = llm.parse_json(text)
    except Exception:  # noqa: BLE001 - the model could not pick: search what the user said
        return [message]
    if d.get("search") not in (True, "true"):
        return None
    words = d.get("words") if isinstance(d.get("words"), list) else []
    words = [str(w).strip() for w in words if str(w).strip()][:MAX_WORDS]
    return words or [message]


def render(hits):
    if not hits:
        return NOT_FOUND
    lines = []
    for h in hits:
        if h.get("count", 1) > 1:  # a mail thread: when it started comes first, then how long it ran
            lines.append(f"- {h['first']}~{h['last']} [{h['kind']}] {h['title']} (같은 메일 스레드 {h['count']}통, "
                         f"{h['first']}에 시작, 아래 내용은 {h['day']} 메일): {h['snippet'][:MAX_SNIPPET]}")
        else:
            lines.append(f"- {h['day']} [{h['kind']}] {h['title']}: {h['snippet'][:MAX_SNIPPET]}")
    return "\n".join(lines)


def find(message, day):
    """The prompt section of past records for this message, or None if there is nothing to look up."""
    words = search_words(message)
    if words is None:
        return None
    try:
        with Librarian(timeout=LOCK_WAIT_S) as lib:
            skipped = lib.update()["skipped"]
            hits = lib.search(" ".join(words), limit=LIMIT, exclude_day=day)
    except Exception as e:  # noqa: BLE001 - a broken index must not break the chat; say so instead of "not found"
        sys.stderr.write(f"librarian unavailable: {type(e).__name__}\n")
        return UNAVAILABLE
    # some files could not be read: what was found still stands, but "not found" would claim more than we know
    return render(hits) + ("\n" + PARTIAL if skipped else "")
