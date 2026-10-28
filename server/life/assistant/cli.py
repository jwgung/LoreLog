"""Command line of life-diary (see `life-diary -h`)."""

import argparse
import datetime as dt
import json
import sys

from life.assistant.chat import chat_history, chat_reply
from life.assistant.context import build_context
from life.assistant.diaryfile import read_day, save_user
from life.assistant.senders import list_senders, save_allow
from life.assistant.summary import generate
from life.assistant.todos import todo_act, todo_add_user, todo_extract, todo_view


def main():
    ap = argparse.ArgumentParser(prog="life-diary", description=__doc__.split("\n\n")[0])
    ap.add_argument("date", nargs="?", default=dt.date.today().isoformat(), help="YYYY-MM-DD (default: today)")
    ap.add_argument("--context", action="store_true", help="print the context sent to the LLM and exit")
    ap.add_argument("--json", action="store_true", help="print the stored draft, corrections and reflection of the day as JSON")
    ap.add_argument("--save", action="store_true", help="read {corrections:[{wrong,right}], reflection} from stdin and store it")
    ap.add_argument("--senders", action="store_true", help="print notification senders of the last 14 days with their allowed flag (JSON)")
    ap.add_argument("--set-allow", action="store_true", help="read {allow:['label: title' | 'label: *']} from stdin and store it")
    ap.add_argument("--chatlog", action="store_true", help="print the day's chat exchange as JSON")
    ap.add_argument("--chat", action="store_true", help="read {message, wrong?} from stdin; save it and print the assistant's reply as JSON")
    ap.add_argument("--todos", action="store_true", help="print open and recently done to-dos as JSON")
    ap.add_argument("--todos-extract", action="store_true", help="extract new to-dos from the day's records (throttled)")
    ap.add_argument("--todo-act", action="store_true", help='read {id, action: done|dismiss|reopen} from stdin')
    ap.add_argument("--todo-add", action="store_true", help="read {text} from stdin and add it as the user's own to-do")
    ap.add_argument("--dry-run", action="store_true", help="print the generated draft instead of writing it")
    args = ap.parse_args()

    if args.todos:
        print(json.dumps(todo_view(), ensure_ascii=False))
        return 0
    if args.todos_extract:
        try:
            print(json.dumps({"added": todo_extract(args.date)}))
        except Exception as e:  # noqa: BLE001
            print(f"todo extract failed: {e}", file=sys.stderr)
            return 1
        return 0
    if args.todo_act:
        p = json.load(sys.stdin)
        return 0 if todo_act(p["id"], p["action"]) else 1
    if args.todo_add:
        todo_add_user(json.load(sys.stdin)["text"], args.date)
        return 0
    if args.chatlog:
        print(json.dumps({"messages": chat_history(args.date)}, ensure_ascii=False))
        return 0
    if args.chat:
        payload = json.load(sys.stdin)
        try:
            print(json.dumps({"reply": chat_reply(args.date, payload["message"], payload.get("wrong"))}, ensure_ascii=False))
        except Exception as e:  # noqa: BLE001
            print(f"chat failed: {e}", file=sys.stderr)
            return 1
        return 0
    if args.senders:
        print(json.dumps(list_senders(), ensure_ascii=False))
        return 0
    if args.set_allow:
        save_allow(json.load(sys.stdin).get("allow", []))
        return 0
    if args.json:
        print(json.dumps(read_day(args.date), ensure_ascii=False))
        return 0
    if args.save:
        save_user(args.date, json.load(sys.stdin))
        return 0
    if args.context:
        print(build_context(args.date, summary=True))
        return 0
    try:
        text, out = generate(args.date, args.dry_run)
    except Exception as e:  # noqa: BLE001 - report any LLM/network failure the same way
        print(f"LLM request failed: {e}", file=sys.stderr)
        return 1
    print(text if out is None else f"wrote {out}")
    return 0
