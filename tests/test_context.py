import json
import unittest

import _env  # noqa: F401  (sets up the throwaway vault first)
from life.assistant import context, senders
from life.assistant.config import DATA, VAULT


def write_mail(name, source, subject, sender, day, time, mail_id="", in_reply_to=""):
    (VAULT / "Mail" / "2026" / name).write_text(
        f"---\ntype: mail\nmail_id: {mail_id}\nmail_source: account/{source}\ndate: {day}\ntime: {time}\n"
        f'from: {sender}\nsubject: "{subject}"\nin_reply_to: {in_reply_to}\n---\nbody\n',
        encoding="utf-8",
    )


class MailMarks(unittest.TestCase):
    def test_subject_key_ignores_reply_prefixes(self):
        self.assertEqual(context._subj_key("Fwd: RE: 견적 검토 요청의 건"), context._subj_key("견적 검토 요청의 건"))

    def test_replied_needs_a_later_reply_in_the_same_thread(self):
        sent = [(context._subj_key("RE: 견적 건"), "", 1100), ("", "abc@h", 1500)]
        self.assertTrue(context.replied(sent, "견적 건", "x", 1000))
        self.assertFalse(context.replied(sent, "견적 건", "x", 1200), "a reply sent before the mail arrived is not an answer")
        self.assertFalse(context.replied(sent, "다른 건", "x", 1000))
        self.assertTrue(context.replied(sent, "무관", "<abc@h>", 1000), "in-reply-to pointing at the mail id counts")

    def test_gather_mail_marks_bulk_sent_and_replied(self):
        write_mail("2026-10-20-0900-a.md", "INBOX", "견적 건", "철수 <a@x.com>", "2026-10-20", "09:00", mail_id="m1")
        write_mail("2026-10-20-1000-b.md", "INBOX", "광고입니다", "홍보 <news@shop.com>", "2026-10-20", "10:00")
        write_mail("2026-10-20-1100-c.md", "Sent Messages", "RE: 견적 건", "나 <me@x.com>", "2026-10-20", "11:00")
        write_mail("2026-10-20-1200-d.md", "INBOX", "회의 자료", "영희 <b@x.com>", "2026-10-20", "12:00", mail_id="m4")
        rows = context.gather_mail("2026-10-20")
        joined = "\n".join(rows)
        self.assertIn("견적 건 [이미 회신함]", joined)
        self.assertIn("[대량 발송]", joined)
        self.assertIn("[내가 보냄]", joined)
        self.assertTrue([r for r in rows if "회의 자료" in r][0].endswith("회의 자료"), "unanswered mail carries no mark")


class Senders(unittest.TestCase):
    def test_default_allows_nothing(self):
        self.assertFalse(senders.is_allowed(set(), "카카오톡", "철수"))

    def test_app_wide_and_per_sender_entries(self):
        allow = {"WORKS: *", "카카오톡: 철수"}
        self.assertTrue(senders.is_allowed(allow, "WORKS", "아무나"))
        self.assertTrue(senders.is_allowed(allow, "카카오톡", "철수"))
        self.assertFalse(senders.is_allowed(allow, "카카오톡", "엄마"))


class Lorelog(unittest.TestCase):
    def test_only_allowed_notifications_and_confirmed_voice(self):
        day = "2026-10-21"
        recs = [
            {"type": "notification", "pkg": "com.kakao.talk", "label": "카카오톡", "title": "동료", "text": "회의 내일", "ts": "1792000000000"},
            {"type": "notification", "pkg": "com.kakao.talk", "label": "카카오톡", "title": "엄마", "text": "밥 먹었니", "ts": "1792000100000"},
            {"type": "voice", "ts": "1792000200000", "text": "확인된 대화"},
            {"type": "voice", "ts": "1792000900000", "text": "주변 소리"},
            {"type": "conversation", "id": "1", "start_ms": "1791999999000", "end_ms": "1792000300000", "participants": "동료", "status": "tagged"},
            {"type": "conversation", "id": "2", "start_ms": "1792000800000", "end_ms": "1792001000000", "participants": "", "status": "ambient"},
        ]
        (DATA / "raw" / "lorelog" / f"{day}.ndjson").write_text("\n".join(json.dumps(r) for r in recs), encoding="utf-8")
        senders.save_allow(["카카오톡: 동료"])
        out = context.gather_lorelog(day)
        self.assertEqual(list(out["notif"]), ["카카오톡: 동료"], "family notification must not reach the model")
        self.assertEqual(len(out["voice"]), 1)
        self.assertIn("확인된 대화", out["voice"][0])
        self.assertIn("with 동료", out["voice"][0])


if __name__ == "__main__":
    unittest.main()
