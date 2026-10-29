import json
import os
import unittest
from email.message import EmailMessage
from pathlib import Path

import _env  # noqa: F401
from life.collect import cli
from life.collect.mailfiles import attachments, blob_path, is_sent_folder, keep

PDF = b"%PDF-1.4 quote v1"
XLSX = b"PK\x03\x04 spreadsheet"


def mail(mid, files, date="Tue, 03 Feb 2026 10:00:00 +0900", to="client@example.com"):
    m = EmailMessage()
    m["Message-ID"] = f"<{mid}>"
    m["Date"] = date
    m["From"] = "me@example.com"
    m["To"] = to
    m["Subject"] = "견적서 송부"
    m.set_content("첨부 확인 부탁드립니다")
    for name, mime, data in files:
        main, sub = mime.split("/")
        m.add_attachment(data, maintype=main, subtype=sub, filename=name)
    return m


def lines(root):
    out = []
    for f in sorted((root / "raw" / "mailfile").glob("*.ndjson")):
        out += [json.loads(line) for line in f.read_text(encoding="utf-8").splitlines()]
    return out


class Keep(unittest.TestCase):
    def setUp(self):
        self.root = _env.ROOT / f"mf-{self._testMethodName}"

    def test_attachments_are_kept_untouched_under_their_hash(self):
        n, written = keep(self.root, mail("a1", [("견적서.pdf", "application/pdf", PDF), ("산출.xlsx", "application/octet-stream", XLSX)]), "acct/Sent Messages")
        self.assertEqual((n, written), (2, len(PDF) + len(XLSX)))
        recs = lines(self.root)
        self.assertEqual({r["name"] for r in recs}, {"견적서.pdf", "산출.xlsx"})
        for r in recs:
            self.assertEqual(blob_path(self.root, r["sha256"]).read_bytes(), {"견적서.pdf": PDF, "산출.xlsx": XLSX}[r["name"]])
            self.assertEqual((r["type"], r["direction"], r["mail_id"], r["mail_source"], r["_src"]), ("file", "sent", "a1", "acct/Sent Messages", "mailfile"))
        if os.name == "posix":
            self.assertEqual(oct(blob_path(self.root, recs[0]["sha256"]).stat().st_mode & 0o777), "0o600")
        self.assertEqual(Path(self.root / "raw/mailfile").glob("2026-02-03.ndjson").__next__().name, "2026-02-03.ndjson", "dated by when it was sent")

    def test_recipients_are_not_copied_into_the_record(self):
        keep(self.root, mail("a2", [("a.pdf", "application/pdf", PDF)]), "acct/Sent")
        text = "".join(f.read_text(encoding="utf-8") for f in (self.root / "raw" / "mailfile").glob("*.ndjson"))
        self.assertNotIn("client@example.com", text, "who it went to lives in the mail note and is worked out later")

    def test_same_file_in_two_mails_is_one_blob_and_two_sightings(self):
        keep(self.root, mail("b1", [("v1.pdf", "application/pdf", PDF)]), "acct/Sent")
        n, written = keep(self.root, mail("b2", [("forward.pdf", "application/pdf", PDF)], date="Wed, 04 Feb 2026 09:00:00 +0900"), "acct/Sent")
        self.assertEqual((n, written), (1, 0), "no bytes written for a file already kept")
        recs = lines(self.root)
        self.assertEqual(len(recs), 2)
        self.assertEqual(len({r["sha256"] for r in recs}), 1)
        self.assertEqual(len(list((self.root / "raw" / "files").rglob("*"))) - 1, 1)  # one blob (+ its folder)

    def test_a_mail_read_twice_adds_no_second_line(self):
        m = mail("c1", [("a.pdf", "application/pdf", PDF)])
        keep(self.root, m, "acct/Sent")
        keep(self.root, m, "acct/Sent")
        self.assertEqual(len(lines(self.root)), 1)

    def test_mail_without_attachments_stores_nothing(self):
        self.assertEqual(keep(self.root, mail("d1", []), "acct/Sent"), (0, 0))
        self.assertFalse((self.root / "raw").exists())

    def test_received_mail_is_marked_received(self):
        keep(self.root, mail("g1", [("in.pdf", "application/pdf", PDF)]), "acct/INBOX", "received")
        self.assertEqual([r["direction"] for r in lines(self.root)], ["received"])

    def test_sent_folder_names(self):
        self.assertTrue(all(is_sent_folder(n) for n in ("Sent Messages", "Sent", "보낸편지함", "INBOX.Sent Items")))
        self.assertFalse(any(is_sent_folder(n) for n in ("INBOX", "Drafts", "Trash")))

    def test_unnamed_attachment_gets_a_placeholder(self):
        m = mail("e1", [])
        m.add_attachment(b"data", maintype="application", subtype="octet-stream")
        self.assertEqual(attachments(m)[0][0], "(unnamed)")


class FakeImap:
    """Just enough IMAP for run_folder: one folder, mails by UID."""

    def __init__(self, mails):
        self.mails, self.fetched = mails, []

    def select(self, name, readonly=False):
        assert readonly, "the server is never written"
        return "OK", [b"1"]

    def response(self, code):
        return code, [b"7"]

    def uid(self, cmd, _charset, *args):
        if cmd == "search":
            return "OK", [b" ".join(str(u).encode() for u in self.mails)]
        assert cmd == "fetch"
        u = int(_charset)  # for fetch the UID arrives as the second positional argument
        self.fetched.append(u)
        return "OK", [(b"1 (UID %d)" % u, self.mails[u].as_bytes()), b")"]


class Run(unittest.TestCase):
    def test_only_unseen_mails_are_read_and_the_state_resumes(self):
        root = _env.ROOT / "mf-run"
        imap = FakeImap({1: mail("r1", [("a.pdf", "application/pdf", PDF)]), 2: mail("r2", []), 3: mail("r3", [("b.xlsx", "application/octet-stream", XLSX)])})
        state = {}
        self.assertEqual(cli.run_folder(imap, "Sent Messages", "acct", root, state, 2, False)[:3], (2, 1, len(PDF), ))  # limit 2: uids 1, 2
        self.assertEqual(sorted(imap.fetched), [1, 2])
        imap.fetched.clear()
        mails, files, written, problem = cli.run_folder(imap, "Sent Messages", "acct", root, state, 10, False)
        self.assertEqual((sorted(imap.fetched), files, problem), ([3], 1, ""), "the next run reads only what is left")
        self.assertEqual(len(lines(root)), 2)


if __name__ == "__main__":
    unittest.main()
