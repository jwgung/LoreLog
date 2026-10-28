import json
import unittest

import _env  # noqa: F401
from life.assistant import chat, diaryfile, todos
from life.assistant.config import DATA, VAULT


class DiaryFile(unittest.TestCase):
    NOTE = "---\ntype: diary\n---\n<!-- ai:begin -->\n## 오늘의 정리 (ai)\n- 하나\n- 둘\n\n## 오늘의 질문 (ai)\n왜요?\n<!-- ai:end -->\n\n## 정정\n<!-- x -->\n\n## 회고\n<!-- y -->\n"

    def test_section_roundtrip(self):
        text = diaryfile.set_section(self.NOTE, "회고", "좋은 하루")
        self.assertEqual(diaryfile.section(text, "회고"), "좋은 하루")
        self.assertEqual(diaryfile.section(text, "정정"), "", "placeholder comments are not content")

    def test_append_keeps_earlier_words_and_corrections(self):
        (VAULT / "Diary").mkdir(exist_ok=True)
        (VAULT / "Diary" / "2026-01-01.md").write_text(self.NOTE, encoding="utf-8")
        diaryfile.save_user("2026-01-01", {"append": True, "corrections": [{"wrong": "둘", "right": "셋"}], "reflection": "첫째"})
        diaryfile.save_user("2026-01-01", {"append": True, "corrections": [{"wrong": "셋", "right": "넷"}], "reflection": "둘째"})
        day = diaryfile.read_day("2026-01-01")
        self.assertEqual(day["corrections"], "- 둘 → 셋\n- 셋 → 넷")
        self.assertEqual(day["reflection"], "첫째\n\n둘째")
        self.assertEqual(day["summary"], ["하나", "둘"], "the AI part is untouched by the user's words")


class ChatHistory(unittest.TestCase):
    def test_correction_wrapper_is_unwrapped(self):
        chat.append_chat("2026-01-02", "user", '(정리의 이 문장이 틀렸다고 알려 주셨어요: "10초 통화")\n20분 했어')
        chat.append_chat("2026-01-02", "assistant", "알겠어요.")
        h = chat.chat_history("2026-01-02")
        self.assertEqual(h[0], {"role": "user", "text": "20분 했어", "wrong": "10초 통화"})
        self.assertEqual(h[1], {"role": "assistant", "text": "알겠어요."})


class Todos(unittest.TestCase):
    def setUp(self):
        if todos.TODO_FILE.exists():
            todos.TODO_FILE.unlink()

    def test_dismissed_todo_never_returns(self):
        d = todos.load_todos()
        todos._add_todo(d, "OO 님 문의에 답장하기", "메일", "", "2026-10-28")
        todos.save_todos(d)
        tid = todos.load_todos()["items"][0]["id"]
        self.assertTrue(todos.todo_act(tid, "dismiss"))
        d = todos.load_todos()
        self.assertFalse(todos._add_todo(d, "OO 님 문의에 답장하기!", "메일", "", "2026-10-29"), "same title modulo punctuation is a duplicate")
        self.assertEqual(todos.todo_view()["open"], [])

    def test_user_added_todo_is_listed(self):
        todos.todo_add_user("세금 서류 챙기기", "2026-10-28")
        self.assertEqual([i["title"] for i in todos.todo_view()["open"]], ["세금 서류 챙기기"])

    def test_reply_closes_mail_todo_automatically(self):
        d = todos.load_todos()
        todos._add_todo(d, "견적 의견 전달하기", "메일", "", "2026-10-28", mail_subject="RE: 견적 검토")
        item = d["items"][0]
        real = todos.sent_index
        try:
            todos.sent_index = lambda days=21: [(todos._subj_key("견적 검토"), "", item["created"] / 1000 + 60)]
            self.assertEqual(todos.todo_autoclose(d), 1)
        finally:
            todos.sent_index = real
        self.assertEqual((item["status"], item.get("auto")), ("done", True))


if __name__ == "__main__":
    unittest.main()
