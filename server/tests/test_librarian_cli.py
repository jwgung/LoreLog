"""The librarian's command line and its place in `life sync`."""
import contextlib
import importlib.machinery
import importlib.util
import io
import unittest
from pathlib import Path

import _env  # noqa: F401  (sets up the throwaway vault first)
from life.assistant.config import VAULT
from life.librarian import cli
from life.librarian.index import default_path

BIN = Path(__file__).resolve().parent.parent / "bin"


def run(*argv):
    out = io.StringIO()
    with contextlib.redirect_stdout(out):
        code = cli.main(list(argv))
    return code, out.getvalue()


class Cli(unittest.TestCase):
    def setUp(self):
        path = VAULT / "Mail" / "2023" / "2023-08-01-0900-cli.md"
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_text('---\nmail_source: account/INBOX\ndate: 2023-08-01\nsubject: "명령줄 확인"\n---\n## Message\n명령줄문구도롱뇽\n',
                        encoding="utf-8")

    def test_update_search_stats_and_rebuild(self):
        code, out = run("update")
        self.assertEqual(code, 0)
        self.assertIn("indexed", out)
        self.assertEqual(run("update")[1].split(",")[0], "indexed 0 files", "nothing new the second time")
        code, out = run("search", "도롱뇽")
        self.assertEqual(code, 0)
        self.assertIn("2023-08-01", out)
        self.assertIn("명령줄 확인", out)
        self.assertIn("documents from", run("stats")[1])
        code, out = run("rebuild")
        self.assertEqual(code, 0)
        self.assertNotIn("indexed 0 files", out, "a rebuild reads everything again")
        self.assertTrue(default_path().exists())

    def test_nothing_found_and_bad_arguments(self):
        run("update")
        self.assertIn("nothing found", run("search", "어디에도없는도롱뇽말")[1])
        for argv in ([], ["search"], ["search", "x", "--limit", "many"], ["nonsense"]):
            with self.assertRaises(SystemExit) as cm, contextlib.redirect_stderr(io.StringIO()):
                cli.main(argv)
            self.assertEqual(cm.exception.code, 2, argv)


class SyncRunsTheLibrarianLast(unittest.TestCase):
    def setUp(self):
        loader = importlib.machinery.SourceFileLoader("life_cli_bin", str(BIN / "life"))
        spec = importlib.util.spec_from_loader("life_cli_bin", loader)
        self.life = importlib.util.module_from_spec(spec)
        loader.exec_module(self.life)
        self.ran = []
        self.life.read_sources = lambda: {"mail": {"folders": ["INBOX"]}, "calendar": {"urls": ["x"]}}
        self.life.run_job = lambda name, cmd, dry: self.ran.append((name, cmd[-1])) or True

    def test_the_librarian_indexes_after_collecting(self):
        self.assertEqual(self.life.sync(dry=False), 0)
        self.assertEqual([n for n, _ in self.ran], ["mail", "files", "calendar", "librarian"])
        self.assertEqual(self.ran[-1], ("librarian", "update"))

    def test_a_dry_run_does_not_touch_the_index(self):
        self.life.sync(dry=True)
        self.assertNotIn("librarian", [n for n, _ in self.ran])


if __name__ == "__main__":
    unittest.main()
