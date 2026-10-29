"""Work that outlives one request: drafting the day and extracting to-dos run on background threads."""
import datetime as dt
import threading

from life.assistant.diaryfile import read_day
from life.assistant.summary import generate
from life.assistant.todos import todo_extract, todo_view

_lock = threading.Lock()
_gen = {}  # date -> "running" | error message of the last failed run
_todo_busy = {"on": False}


def _draft_job(day):
    try:
        generate(day)
        err = None
    except Exception as e:  # noqa: BLE001 - any LLM/network failure is reported to the client the same way
        err = f"LLM request failed: {e}"
    with _lock:
        if err is None:
            _gen.pop(day, None)
        else:
            _gen[day] = err


def draft(day, refresh=False):
    """The day's stored draft -> (http status, body). Starts generation in the background when it is missing or stale."""
    with _lock:
        state = _gen.get(day)
        if state == "running":
            return 202, {"status": "generating"}
        if state:  # report the last failure once, then allow a retry
            del _gen[day]
            return 502, {"error": state}
        d = read_day(day)
        if d.get("exists") and not d.get("stale") and not refresh:
            return 200, d
        _gen[day] = "running"
    threading.Thread(target=_draft_job, args=(day,), daemon=True).start()
    return 202, {"status": "generating"}


def _extract_job():
    try:
        todo_extract(dt.date.today().isoformat())
    except Exception:  # noqa: BLE001 - the next request retries; nothing for the client to wait on
        pass
    finally:
        _todo_busy["on"] = False


def extract_todos():
    with _lock:
        if not _todo_busy["on"]:
            _todo_busy["on"] = True
            threading.Thread(target=_extract_job, daemon=True).start()


def todos():
    v = todo_view()
    v["extracting"] = _todo_busy["on"]
    return v
