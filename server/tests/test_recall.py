"""The assistant asks the librarian before it answers: what the user said becomes search words, and what the librarian
finds on other days is put next to today's records. The model is faked here; the librarian is real."""
import json
import unittest

import _env  # noqa: F401  (sets up the throwaway vault first)
from life.assistant import chat, llm, recall
from life.assistant.config import VAULT


def mail(name, subject, body, day):
    path = VAULT / "Mail" / day[:4] / name
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(f'---\ntype: mail\nmail_source: account/INBOX\ndate: {day}\ntime: 10:00\nfrom: 거래처 <c@x.com>\n'
                    f'subject: "{subject}"\n---\n## Message\n{body}\n', encoding="utf-8")


class FakeModel:
    """Answers the search-word request with `words` (or raises), records the prompt of the reply."""

    def __init__(self, words=None, fail=False):
        self.words, self.fail, self.calls, self.reply_prompt = words, fail, [], None

    def __call__(self, messages, **kw):
        system = messages[0]["content"]
        if system.startswith(recall.WORDS_SYSTEM):
            self.calls.append("words")
            if self.fail:
                raise OSError("model down")
            return json.dumps({"search": self.words is not None, "words": self.words or []}, ensure_ascii=False)
        self.calls.append("reply")
        self.reply_prompt = system
        return "네, 확인했어요."


SECTION = recall.HEADER + "\n"  # the section itself; the rules also mention its name


def past_section(prompt):
    return prompt.split(SECTION, 1)[1] if SECTION in prompt else None


class Recall(unittest.TestCase):
    def setUp(self):
        self.real = llm.complete

    def tearDown(self):
        llm.complete = self.real

    def talk(self, model, message, day="2026-11-02", wrong=None):
        llm.complete = model
        return chat.chat_reply(day, message, wrong)

    def test_records_from_other_days_are_put_next_to_today(self):
        mail("2025-05-14-1000-q.md", "프레스 금형 견적", "금형칠번 단가는 다음 주에 확정합니다", "2025-05-14")
        model = FakeModel(words=["금형칠번"])
        self.talk(model, "그 금형칠번 견적 어떻게 됐지?")
        self.assertEqual(model.calls, ["words", "reply"])
        section = past_section(model.reply_prompt)
        self.assertIn("2025-05-14", section)
        self.assertIn("프레스 금형 견적", section)

    def test_when_nothing_is_found_the_prompt_says_so(self):
        model = FakeModel(words=["어디에도없는품목"])
        self.talk(model, "어디에도없는품목 그거 언제 샀지?")
        self.assertIn("찾지 못함", past_section(model.reply_prompt))

    def test_nothing_is_searched_when_there_is_nothing_to_look_for(self):
        model = FakeModel(words=None)
        self.talk(model, "오늘 좀 피곤하네")
        self.assertIsNone(past_section(model.reply_prompt))

    def test_if_the_model_cannot_pick_words_the_users_words_are_searched(self):
        mail("2025-06-01-1000-r.md", "출장 보고", "청주출장오징어 일정 공유", "2025-06-01")
        model = FakeModel(fail=True)
        self.talk(model, "청주출장오징어 언제였지")
        self.assertIn("출장 보고", past_section(model.reply_prompt))

    def test_a_correction_is_not_a_question_and_asks_the_librarian_nothing(self):
        model = FakeModel(words=["무엇이든"])
        self.talk(model, "회의는 두 번이었어", wrong="회의를 세 번 했어요")
        self.assertEqual(model.calls, ["reply"])

    def test_todays_own_records_are_not_repeated_as_past_records(self):
        mail("2026-11-03-1000-t.md", "오늘 메일", "당일문구참치", "2026-11-03")
        model = FakeModel(words=["당일문구참치"])
        self.talk(model, "당일문구참치 뭐였지", day="2026-11-03")
        self.assertNotIn("오늘 메일", past_section(model.reply_prompt))

    def test_files_the_librarian_could_not_read_are_mentioned(self):
        real = recall.Librarian

        class Partial:
            def __init__(self, *a, **k):
                pass

            def __enter__(self):
                return self

            def __exit__(self, *e):
                pass

            def update(self):
                return {"indexed": 0, "removed": 0, "skipped": 1, "docs": 0}

            def search(self, *a, **k):
                return []
        recall.Librarian = Partial
        try:
            model = FakeModel(words=["무엇"])
            self.talk(model, "무엇 찾아줘")
            section = past_section(model.reply_prompt)
            self.assertIn("찾지 못함", section)
            self.assertIn("일부 기록 파일을 지금 읽지 못해", section)
        finally:
            recall.Librarian = real

    def test_a_search_flag_written_as_a_string_is_read_as_meant(self):
        llm.complete = lambda *a, **k: '{"search": "false", "words": ["x"]}'
        self.assertIsNone(recall.search_words("기분이 별로야"))

    def test_a_broken_librarian_does_not_break_the_chat_and_is_not_reported_as_nothing_found(self):
        real = recall.Librarian

        class Broken:
            def __init__(self, *a, **k):
                raise OSError("disk")
        recall.Librarian = Broken
        try:
            model = FakeModel(words=["아무거나"])
            self.assertEqual(self.talk(model, "아무거나 찾아줘"), "네, 확인했어요.")
            section = past_section(model.reply_prompt)
            self.assertIn("찾아보지 못함", section)
            self.assertNotIn("찾지 못함:", section)
        finally:
            recall.Librarian = real

    def test_too_many_words_from_the_model_are_cut(self):
        model = FakeModel(words=[f"낱말{i}" for i in range(40)])
        llm.complete = model
        self.assertLessEqual(len(recall.search_words("아무 말")), recall.MAX_WORDS)


if __name__ == "__main__":
    unittest.main()
