"""The librarian: an index over every record, searched across all days. It finds and does not judge.

What it must keep: Korean words are found inside longer words and as two letters; only records the diary rules allow
are indexed (allowed messengers, confirmed voice); the assistant's own words are not records; a changed or removed file
is reflected; records that disagree come back together.
"""
import datetime as dt
import json
import os
import tempfile
import unittest
from pathlib import Path

import _env  # noqa: F401  (sets up the throwaway vault first)
from life.assistant import senders
from life.assistant.config import DATA, VAULT
from life.librarian import Librarian, records, terms
from life.librarian.index import to_query


def ms(y, m, d, h=9, mi=0):
    return int(dt.datetime(y, m, d, h, mi).timestamp() * 1000)


def write(path, text):
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(text, encoding="utf-8")


def mail(name, subject, body, day, source="INBOX", sender="철수 <a@x.com>", mail_id=None, references="[]", in_reply_to=""):
    path = VAULT / "Mail" / day[:4] / name
    write(path, f'---\ntype: mail\nmail_id: {mail_id or f"<{name}@x>"}\nmail_source: account/{source}\nmail_synced: 2024-01-01\n'
                f'date: {day}\ntime: 09:00\nfrom: {sender}\nto: 나 <me@x.com>\nsubject: "{subject}"\n'
                f'in_reply_to: {in_reply_to}\nreferences: {references}\nattachments: []\n---\n## Notes\n\n## Message\n{body}\n')
    return path


def lorelog(day, recs):
    write(DATA / "raw/lorelog" / f"{day}.ndjson", "\n".join(json.dumps(r, ensure_ascii=False) for r in recs) + "\n")


class Terms(unittest.TestCase):
    def test_korean_is_indexed_in_two_letter_pieces(self):
        self.assertEqual(terms("거래처랑"), "거래 래처 처랑")

    def test_latin_words_and_numbers_are_lowercased_whole(self):
        self.assertEqual(terms("PO-2024 Quote"), "po 2024 quote")

    def test_a_single_korean_letter_stays_as_it_is(self):
        self.assertEqual(terms("책 좀"), "책 좀")


class Search(unittest.TestCase):
    def setUp(self):
        self.allow_before = senders.load_allow()
        tmp = tempfile.TemporaryDirectory()
        self.addCleanup(tmp.cleanup)
        self.lib = Librarian(Path(tmp.name) / "librarian.sqlite")

    def tearDown(self):
        self.lib.close()
        senders.save_allow(self.allow_before)

    def find(self, q, **kw):
        return self.lib.search(q, **kw)

    def test_a_mail_body_is_found_by_a_word_inside_a_longer_word_and_by_two_letters(self):
        path = mail("2024-03-05-0900-meet.md", "미팅 결과", "거래처랑 견적 얘기를 했다", "2024-03-05")
        self.lib.update()
        for q in ("거래처", "견적"):
            hits = [h for h in self.find(q) if h["title"] == "미팅 결과"]
            self.assertEqual(len(hits), 1, q)
            h = hits[0]
            self.assertEqual((h["source"], h["day"]), ("mail", "2024-03-05"))
            self.assertIn(q, h["snippet"])
            self.assertEqual(h["ref"], str(path))

    def test_the_mail_header_is_searched_but_the_snippet_comes_from_the_body(self):
        mail("2024-03-05-1100-hdr.md", "헤더제목수달 안내", "%% mailstone:begin %%\n본문첫줄입니다 수달 일정 공유", "2024-03-05",
             sender="보낸이두더지 <mole@x.com>")
        self.lib.update()
        self.assertTrue(self.find("보낸이두더지"), "who sent it is searchable")
        h = [h for h in self.find("헤더제목수달") if h["title"] == "헤더제목수달 안내"][0]
        self.assertTrue(h["snippet"].startswith("본문첫줄입니다"), h["snippet"])
        for noise in ("mail_source", "date:", "@x.com", "mailstone"):
            self.assertNotIn(noise, h["snippet"])

    def test_a_single_letter_query_finds_words_that_start_with_it(self):
        mail("2024-03-05-1000-desk.md", "가구", "책상 주문함", "2024-03-05")
        self.lib.update()
        self.assertTrue([h for h in self.find("책") if h["title"] == "가구"])

    def test_events_are_found(self):
        write(VAULT / "Events" / "2024-03-07 장비 점검.md",
              "---\ntype: event\ndav_uid: u1\ntitle: 장비 점검\nstart: 2024-03-07T14:00:00\nend: 2024-03-07T15:00:00\n"
              "all_day: false\nlocation: 2공장\nattendees:\n  - 영희 <b@x.com>\n---\n")
        self.lib.update()
        hits = [h for h in self.find("장비 점검") if h["source"] == "event"]
        self.assertEqual([(h["title"], h["day"]) for h in hits], [("장비 점검", "2024-03-07")])
        self.assertTrue(self.find("2공장"), "where it happens is searchable too")

    def test_only_allowed_messengers_and_confirmed_voice_are_indexed(self):
        senders.save_allow({"카카오톡: 동료사서"})
        day = "2024-03-06"
        lorelog(day, [
            {"type": "notification", "pkg": "com.kakao.talk", "label": "카카오톡", "title": "동료사서", "text": "도면 수정본 보냈어요", "ts": ms(2024, 3, 6, 9)},
            {"type": "notification", "pkg": "com.kakao.talk", "label": "카카오톡", "title": "엄마사서", "text": "고래밥 사와", "ts": ms(2024, 3, 6, 10)},
            {"type": "voice", "ts": ms(2024, 3, 6, 11, 5), "text": "부품 단가 협의"},
            {"type": "voice", "ts": ms(2024, 3, 6, 15, 5), "text": "옆자리 잡담바나나"},
            {"type": "conversation", "id": "lib-c1", "start_ms": ms(2024, 3, 6, 11), "end_ms": ms(2024, 3, 6, 11, 30), "participants": "동료사서", "status": "tagged"},
            {"type": "conversation", "id": "lib-c2", "start_ms": ms(2024, 3, 6, 15), "end_ms": ms(2024, 3, 6, 15, 30), "participants": "?", "status": "ambient"},
        ])
        self.lib.update()
        found = {h["kind"] for h in self.find("도면 수정본") if h["day"] == day}
        self.assertEqual(found, {"메신저"})
        self.assertEqual({h["kind"] for h in self.find("부품 단가") if h["day"] == day}, {"대화"})
        self.assertEqual(self.find("고래밥"), [], "a sender the user did not allow is not indexed")
        self.assertEqual(self.find("잡담바나나"), [], "voice in a conversation that is not the user's is not indexed")

        # allowing the sender later makes it findable; taking it back hides it again
        senders.save_allow({"카카오톡: 동료사서", "카카오톡: 엄마사서"})
        self.lib.update()
        self.assertTrue(self.find("고래밥"))
        senders.save_allow({"카카오톡: 동료사서"})
        self.lib.update()
        self.assertEqual(self.find("고래밥"), [])

    def test_pc_windows_are_found(self):
        write(DATA / "raw/pc" / "2024-03-09.ndjson", json.dumps(
            {"type": "window", "ts": ms(2024, 3, 9, 9), "end": ms(2024, 3, 9, 9, 40), "dur_s": 2400, "app": "EXCEL.EXE",
             "title": "견적서_v3.xlsx - Excel"}, ensure_ascii=False) + "\n")
        self.lib.update()
        self.assertEqual({(h["source"], h["kind"]) for h in self.find("견적서") if h["day"] == "2024-03-09"}, {("trace", "PC")})

    def test_the_users_words_are_indexed_but_not_the_assistants(self):
        day = "2024-03-08"
        write(DATA / "raw/diary" / f"{day}.ndjson", "\n".join(json.dumps(r, ensure_ascii=False) for r in [
            {"type": "chat", "ts": ms(2024, 3, 8, 21), "date": day, "role": "user", "text": "김부장님이랑 일정 미뤘어"},
            {"type": "chat", "ts": ms(2024, 3, 8, 21, 1), "date": day, "role": "assistant", "text": "알겠어요 비서답변기린"},
            {"type": "reflection", "ts": ms(2024, 3, 8, 22), "date": day, "reflection": "오늘은 지쳤다호랑이",
             "corrections": [{"wrong": "회의 세 번", "right": "회의 두 번"}]},
        ]) + "\n")
        write(VAULT / "Diary" / f"{day}.md", "---\ntype: diary\n---\n<!-- ai:begin -->\n## 오늘의 정리 (ai)\n- 정리문장코끼리\n<!-- ai:end -->\n")
        self.lib.update()
        self.assertEqual({h["source"] for h in self.find("김부장")}, {"word"})
        self.assertTrue(self.find("지쳤다호랑이"))
        self.assertTrue(self.find("회의 두 번"), "a correction is the user's word")
        self.assertEqual(self.find("비서답변기린"), [], "the assistant's reply is not a record")
        self.assertEqual(self.find("정리문장코끼리"), [], "the assistant's summary is derived, not a record")

    def test_a_changed_file_is_reindexed_and_a_removed_file_is_forgotten(self):
        path = mail("2024-03-10-0900-edit.md", "수정", "낡은문구가오리", "2024-03-10")
        self.lib.update()
        self.assertTrue(self.find("가오리"))
        mail("2024-03-10-0900-edit.md", "수정", "새문구해마", "2024-03-10")
        os.utime(path, ns=(path.stat().st_atime_ns, path.stat().st_mtime_ns + 10_000_000))
        self.lib.update()
        self.assertEqual(self.find("가오리"), [])
        self.assertTrue(self.find("해마"))
        path.unlink()
        self.lib.update()
        self.assertEqual(self.find("해마"), [])

    def test_a_second_update_with_nothing_new_touches_nothing(self):
        mail("2024-03-11-0900-same.md", "그대로", "변함없음", "2024-03-11")
        self.lib.update()
        self.assertEqual(self.lib.update()["indexed"], 0)

    def test_a_mail_thread_comes_back_as_one_hit_with_its_span(self):
        root = "<root-penguin@x>"
        mail("2024-04-01-0900-t1.md", "브로커서버 정보 공유 건", "스레드원문펭귄", "2024-04-01", mail_id=root)
        mail("2024-04-03-0900-t2.md", "RE: 브로커서버 정보 공유 건", "답장 스레드원문펭귄", "2024-04-03",
             mail_id="<r1-penguin@x>", in_reply_to=root, references=f"[{root}]")
        mail("2024-04-09-0900-t3.md", "Re: RE: 브로커서버 정보 공유 건", "또 답장 스레드원문펭귄", "2024-04-09",
             mail_id="<r2-penguin@x>", in_reply_to="<r1-penguin@x>", references=f'["{root}", "<r1-penguin@x>"]')
        mail("2024-04-02-0900-other.md", "다른 건", "스레드원문펭귄 언급", "2024-04-02")
        self.lib.update()
        hits = self.find("스레드원문펭귄")
        thread = [h for h in hits if "브로커서버" in h["title"]]
        self.assertEqual(len(thread), 1, "replies in one thread do not take every slot")
        self.assertEqual((thread[0]["count"], thread[0]["first"], thread[0]["last"]), (3, "2024-04-01", "2024-04-09"))
        self.assertTrue([h for h in hits if h["title"] == "다른 건"], "another mail still has its own place")

    def test_mails_that_only_share_a_subject_are_not_one_thread(self):
        mail("2024-05-01-0900-a.md", "견적", "같은제목물개 1월 견적", "2024-05-01")
        mail("2024-06-07-0900-b.md", "견적", "같은제목물개 6월 견적", "2024-06-07")
        self.lib.update()
        hits = [h for h in self.find("같은제목물개") if h["title"] == "견적"]
        self.assertEqual(sorted(h["day"] for h in hits), ["2024-05-01", "2024-06-07"])
        self.assertEqual({h["count"] for h in hits}, {1})

    def test_a_conversation_confirmed_on_a_later_day_makes_the_earlier_voice_findable(self):
        senders.save_allow(set())
        lorelog("2024-03-14", [{"type": "voice", "ts": ms(2024, 3, 14, 10, 5), "text": "나중확인대화하마"},
                               {"type": "conversation", "id": "lib-c3", "start_ms": ms(2024, 3, 14, 10), "end_ms": ms(2024, 3, 14, 10, 30),
                                "participants": "?", "status": "pending"}])
        self.lib.update()
        self.assertEqual(self.find("나중확인대화하마"), [])
        # the user confirms it the next day: the new state arrives in the next day's file
        lorelog("2024-03-15", [{"type": "conversation", "id": "lib-c3", "start_ms": ms(2024, 3, 14, 10), "end_ms": ms(2024, 3, 14, 10, 30),
                                "participants": "팀장", "status": "tagged"}])
        self.lib.update()
        self.assertEqual([h["day"] for h in self.find("나중확인대화하마")], ["2024-03-14"])
        # and taking it back hides it again
        lorelog("2024-03-15", [{"type": "conversation", "id": "lib-c3", "start_ms": ms(2024, 3, 14, 10), "end_ms": ms(2024, 3, 14, 10, 30),
                                "participants": "?", "status": "ambient"}])
        self.lib.update()
        self.assertEqual(self.find("나중확인대화하마"), [])

    @unittest.skipIf(os.name != "posix" or os.geteuid() == 0, "needs file permissions that bind the test user")
    def test_a_phone_file_that_cannot_be_read_after_taking_a_sender_back_is_not_left_searchable(self):
        senders.save_allow({"카카오톡: 철회대상"})
        lorelog("2024-03-16", [{"type": "notification", "pkg": "com.kakao.talk", "label": "카카오톡", "title": "철회대상",
                                "text": "철회문구너구리", "ts": ms(2024, 3, 16, 9)}])
        self.lib.update()
        self.assertTrue(self.find("철회문구너구리"))
        path = DATA / "raw/lorelog" / "2024-03-16.ndjson"
        senders.save_allow(set())
        path.chmod(0)
        try:
            self.lib.update()
            self.assertEqual(self.find("철회문구너구리"), [], "a file that could not be read again is forgotten, not kept as it was")
        finally:
            path.chmod(0o600)
        self.lib.update()
        self.assertEqual(self.find("철회문구너구리"), [], "once readable it is read under the new rules")

    def test_a_record_that_cannot_be_understood_skips_that_file_and_indexes_the_rest(self):
        broken = DATA / "raw/diary" / "2024-03-17.ndjson"
        write(broken, json.dumps({"type": "reflection", "date": 20240317, "reflection": "깨진기록바퀴", "corrections": 5}) + "\n")
        mail("2024-03-17-0900-ok.md", "멀쩡함", "멀쩡한문구고슴도치", "2024-03-17")
        try:
            r = self.lib.update()
            self.assertEqual(r["skipped"], 1)
            self.assertTrue(self.find("고슴도치"), "the other files are indexed")
            self.assertEqual(self.lib.update()["skipped"], 1, "the skipped file is tried again next time")
        finally:
            broken.unlink()

    def test_a_number_where_a_date_should_be_falls_back_to_the_file_day(self):
        path = DATA / "raw/diary" / "2024-03-19.ndjson"
        write(path, json.dumps({"type": "reflection", "date": 20240319, "reflection": "숫자날짜올빼미"}) + "\n")
        self.lib.update()
        self.assertEqual([h["day"] for h in self.find("숫자날짜올빼미")], ["2024-03-19"])

    def test_an_index_of_another_shape_is_built_again(self):
        mail("2024-03-18-0900-v.md", "버전", "버전문구다람쥐", "2024-03-18")
        self.lib.update()
        with self.lib.db:
            self.lib.db.execute("update meta set value = 'old' where key = 'schema'")
        self.lib.close()
        self.lib = Librarian(self.lib.path)
        self.assertEqual(self.find("다람쥐"), [], "the old index is dropped")
        self.lib.update()
        self.assertTrue(self.find("다람쥐"))

    def test_a_long_message_is_cut_to_a_bounded_query(self):
        self.lib.update()
        words = " ".join(f"단어{i}" for i in range(200))
        self.assertLessEqual(to_query(words).count(" OR ") + 1, 16)

    def test_a_broken_line_in_a_phone_file_does_not_stop_the_index(self):
        senders.save_allow({"카카오톡: 깨진줄동료"})
        path = DATA / "raw/lorelog" / "2024-03-20.ndjson"
        rec = {"type": "notification", "pkg": "com.kakao.talk", "label": "카카오톡", "title": "깨진줄동료",
               "text": "깨진줄옆문구수리부엉이", "ts": ms(2024, 3, 20, 9)}
        path.write_bytes(b"\xea\xb0 cut off\n[1]\n" + json.dumps(rec, ensure_ascii=False).encode() + b"\n")
        mail("2024-03-20-0900-fine.md", "멀쩡", "옆메일문구너구리개", "2024-03-20")
        r = self.lib.update()
        self.assertEqual(r["skipped"], 0)
        self.assertTrue(self.find("수리부엉이"), "the good lines of the file are indexed")
        self.assertTrue(self.find("너구리개"), "and the other files too")

    def test_a_new_line_in_one_phone_file_reads_only_that_file(self):
        senders.save_allow({"카카오톡: 증분동료"})
        for d in (21, 22, 23):
            lorelog(f"2024-03-{d}", [{"type": "notification", "pkg": "com.kakao.talk", "label": "카카오톡", "title": "증분동료",
                                     "text": f"증분문구{d}", "ts": ms(2024, 3, d, 9)}])
        self.lib.update()
        path = DATA / "raw/lorelog" / "2024-03-23.ndjson"
        with open(path, "a", encoding="utf-8") as fh:
            fh.write(json.dumps({"type": "notification", "pkg": "com.kakao.talk", "label": "카카오톡", "title": "증분동료",
                                 "text": "새로온증분고양이", "ts": ms(2024, 3, 23, 10)}, ensure_ascii=False) + "\n")
        self.assertEqual(self.lib.update()["indexed"], 1, "only the phone file that changed is read again")
        self.assertTrue(self.find("새로온증분고양이"))

    def test_the_allowed_senders_used_and_the_ones_remembered_are_the_same(self):
        # the allow list changes while an update runs: what was indexed must match what is remembered,
        # so the next update still sees the change
        lorelog("2024-03-24", [{"type": "notification", "pkg": "com.kakao.talk", "label": "카카오톡", "title": "경합상대",
                                "text": "경합문구두루미", "ts": ms(2024, 3, 24, 9)}])
        answers = iter([{"카카오톡: 경합상대"}] + [set()] * 20)
        real = records.load_allow
        records.load_allow = lambda: next(answers)
        try:
            self.lib.update()
            self.assertTrue(self.find("두루미"), "indexed under the list read at the start")
            self.lib.update()
            self.assertEqual(self.find("두루미"), [], "the sender taken back is dropped at the next update")
        finally:
            records.load_allow = real

    def test_a_query_word_with_a_particle_still_finds_the_word(self):
        mail("2024-03-25-0900-p.md", "조사", "견적서 보냄 그리고 여수항만 일정", "2024-03-25")
        self.lib.update()
        for q in ("견적서는", "여수항만에서", "견적서를"):
            self.assertTrue([h for h in self.find(q) if h["title"] == "조사"], q)

    def test_an_english_word_finds_longer_forms_of_it(self):
        mail("2024-03-26-0900-e.md", "brokerz", "two Brokers and a brokerage", "2024-03-26")
        self.lib.update()
        self.assertTrue([h for h in self.find("broker") if h["title"] == "brokerz"])

    def test_the_users_own_past_questions_do_not_crowd_out_the_records(self):
        for d in range(1, 6):
            write(DATA / "raw/diary" / f"2024-02-0{d}.ndjson", json.dumps(
                {"type": "chat", "ts": ms(2024, 2, d, 21), "date": f"2024-02-0{d}", "role": "user", "text": "펠리컨견적 언제 왔지?"},
                ensure_ascii=False) + "\n")
        mail("2024-01-15-0900-pel.md", "펠리컨 건", "펠리컨견적 송부드립니다", "2024-01-15")
        self.lib.update()
        hits = self.find("펠리컨견적", limit=3)
        self.assertIn("mail", {h["source"] for h in hits}, "the record itself is among the top hits")
        self.assertLessEqual(sum(h["source"] == "word" for h in hits), 2)
        self.assertTrue([h for h in hits if h["source"] == "word"], "what the user said is still findable")

    def test_a_reset_empties_the_index_without_removing_the_file(self):
        mail("2024-03-27-0900-r.md", "리셋", "리셋문구해달", "2024-03-27")
        self.lib.update()
        inode = self.lib.path.stat().st_ino
        self.lib.reset()
        self.assertEqual(self.find("해달"), [])
        self.assertEqual(self.lib.path.stat().st_ino, inode)
        self.lib.update()
        self.assertTrue(self.find("해달"))

    def test_a_phone_line_with_values_of_the_wrong_shape_does_not_stop_the_index(self):
        senders.save_allow({"카카오톡: 모양동료"})
        lorelog("2024-03-28", [
            {"type": "app", "label": ["목록"], "pkg": "y"},
            {"type": "voice", "text": "무한시각", "ts": 1e400},
            {"type": "conversation", "id": "lib-inf", "start_ms": 1, "end_ms": 1e400, "status": "tagged"},
            {"type": "notification", "pkg": "com.kakao.talk", "label": "카카오톡", "title": "모양동료",
             "text": "모양틀린줄옆문구가마우지", "ts": ms(2024, 3, 28, 9)},
        ])
        mail("2024-03-28-0900-shape.md", "모양", "모양옆메일물총새", "2024-03-28")
        self.lib.update()
        self.assertTrue(self.find("가마우지"), "the good lines of the day are still indexed")
        self.assertTrue(self.find("물총새"), "and the rest of the update went through")
        lorelog("2024-03-29", [{"type": "voice", "ts": ms(2024, 3, 29, 9), "text": "다음날음성저어새"},
                               {"type": "conversation", "id": "lib-ok", "start_ms": ms(2024, 3, 29, 8),
                                "end_ms": ms(2024, 3, 29, 10), "participants": "팀", "status": "tagged"}])
        self.lib.update()
        self.assertTrue(self.find("저어새"), "a broken conversation kept from before does not stop other days")

    def test_a_conversation_over_several_days_is_decided_again_on_every_day(self):
        lorelog("2024-04-12", [{"type": "voice", "ts": ms(2024, 4, 12, 9), "text": "사흘째음성황새"}])
        lorelog("2024-04-10", [{"type": "conversation", "id": "lib-long", "start_ms": ms(2024, 4, 10, 9),
                                "end_ms": ms(2024, 4, 13, 9), "participants": "?", "status": "pending"}])
        self.lib.update()
        self.assertEqual(self.find("황새"), [])
        lorelog("2024-04-14", [{"type": "conversation", "id": "lib-long", "start_ms": ms(2024, 4, 10, 9),
                                "end_ms": ms(2024, 4, 13, 9), "participants": "팀", "status": "tagged"}])
        self.lib.update()
        self.assertEqual([h["day"] for h in self.find("황새")], ["2024-04-12"])

    def test_a_conversation_whose_end_is_no_date_is_decided_again_on_every_day(self):
        lorelog("2024-05-03", [{"type": "voice", "ts": ms(2024, 5, 3, 9), "text": "끝없는대화음성두루미"}])
        lorelog("2024-05-01", [{"type": "conversation", "id": "lib-nodate", "start_ms": ms(2024, 5, 1, 9),
                                "end_ms": 10 ** 17, "participants": "팀", "status": "tagged"}])
        self.lib.update()
        self.assertTrue(self.find("두루미"))
        lorelog("2024-05-04", [{"type": "conversation", "id": "lib-nodate", "start_ms": ms(2024, 5, 1, 9),
                                "end_ms": 10 ** 17, "participants": "팀", "status": "skipped"}])
        self.lib.update()
        self.assertEqual(self.find("두루미"), [], "voice the user withdrew does not stay in the index")

    def test_a_long_query_keeps_its_last_words(self):
        words = "지난달에 부산에서 거래처랑 만난 사람이 보낸 견적서는 메일로 언제 왔는지 회의록에도 있었나요 트렌드마이너"
        q = to_query(words)
        self.assertIn('"회의 의록"', q)
        self.assertIn('"트렌 렌드 드마 마이 이너"', q)

    def test_a_rebuild_never_shows_an_empty_index(self):
        mail("2024-03-30-0900-rb.md", "재구축", "재구축문구따오기", "2024-03-30")
        self.lib.update()
        r = self.lib.update(reset=True)
        self.assertGreater(r["indexed"], 0)
        self.assertTrue(self.find("따오기"))

    def test_records_that_disagree_come_back_together(self):
        senders.save_allow({"카카오톡: 거래처담당"})
        mail("2024-03-12-0900-ship.md", "납품", "납품일정 확정입니다", "2024-03-12")
        lorelog("2024-03-12", [{"type": "notification", "pkg": "com.kakao.talk", "label": "카카오톡", "title": "거래처담당",
                                "text": "납품일정 보류해 주세요", "ts": ms(2024, 3, 12, 14)}])
        self.lib.update()
        hits = [h for h in self.find("납품일정") if h["day"] == "2024-03-12"]
        self.assertEqual({h["source"] for h in hits}, {"mail", "trace"}, "both records are returned; the librarian does not pick one")

    def test_nothing_found_is_an_empty_list(self):
        self.lib.update()
        self.assertEqual(self.find("아무데도없는말"), [])
        self.assertEqual(self.find(""), [])
        self.assertEqual(self.find("   ...  "), [])

    def test_a_day_can_be_left_out(self):
        mail("2024-03-13-0900-skip.md", "제외", "하루빼기문어", "2024-03-13")
        self.lib.update()
        self.assertTrue(self.find("문어"))
        self.assertEqual(self.find("문어", exclude_day="2024-03-13"), [])

    def test_the_index_file_is_private(self):
        self.lib.update()
        if os.name == "posix":
            self.assertEqual(oct(self.lib.path.stat().st_mode & 0o777), "0o600")


if __name__ == "__main__":
    unittest.main()
