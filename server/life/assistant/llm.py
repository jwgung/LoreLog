"""The one place that talks to the local language model (OpenAI-compatible chat completions)."""
import json
import re
import urllib.request

from life.assistant.config import LLM_MODEL, LLM_URL


def complete(messages, *, temperature, max_tokens, json_mode=False, timeout=120):
    """Send a chat and return the model's text. thinking is off (the answers are short and the wait matters)."""
    body = {
        "model": LLM_MODEL,
        "messages": messages,
        "temperature": temperature,
        "max_tokens": max_tokens,
        "chat_template_kwargs": {"enable_thinking": False},
    }
    if json_mode:
        body["response_format"] = {"type": "json_object"}
    req = urllib.request.Request(LLM_URL.rstrip("/") + "/chat/completions", json.dumps(body).encode(), {"Content-Type": "application/json"})
    with urllib.request.urlopen(req, timeout=timeout) as r:
        return json.load(r)["choices"][0]["message"]["content"]


def parse_json(text):
    """The first {...} object in the model's text."""
    return json.loads(re.search(r"\{.*\}", text, re.S).group(0))
