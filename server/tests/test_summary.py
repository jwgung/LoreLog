import unittest

import _env  # noqa: F401
from life.assistant import llm, summary


class DropFiller(unittest.TestCase):
    def test_removes_filler_sentences(self):
        out = summary.drop_filler("오전 9시 회의가 있었어요.\n오늘은 다양한 앱을 활용하셨어요.\n오후 3시 통화를 하셨어요.")
        self.assertNotIn("다양한 앱", out)
        self.assertEqual(len(out.splitlines()), 2)

    def test_sorts_by_clock_time_and_converts_afternoon(self):
        out = summary.drop_filler("오후 7시 15분 메일이 왔어요.\n오전 8시 34분 회신을 보냈어요.\n점심시간 잡담을 나누셨어요.\n오후 1시 45분 대화가 있었어요.")
        order = [l.split(" ")[0:3] for l in out.splitlines()]
        self.assertTrue(out.splitlines()[0].startswith("오전 8시"))
        self.assertTrue(out.splitlines()[1].startswith("점심시간"))
        self.assertTrue(out.splitlines()[-1].startswith("오후 7시"))
        self.assertEqual(len(order), 4)

    def test_sentence_without_time_stays_after_its_neighbour(self):
        out = summary.drop_filler("오후 2시 회의가 있었어요.\n결론이 났어요.\n오전 9시 출근했어요.")
        self.assertEqual([l[:5] for l in out.splitlines()], ["오전 9시", "오후 2시", "결론이 났"])


class Llm(unittest.TestCase):
    def test_parse_json_takes_the_first_object(self):
        self.assertEqual(llm.parse_json('답: {"summary": "a", "question": "b"} 끝'), {"summary": "a", "question": "b"})


if __name__ == "__main__":
    unittest.main()
