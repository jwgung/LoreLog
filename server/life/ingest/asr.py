"""Speech to text through the local speech model. The audio is forwarded and never written to disk."""
import hashlib
import json
import os
import re
import urllib.request

ASR_URL = os.environ.get("LIFE_ASR_URL", "http://10.100.3.232:8000/v1/audio/transcriptions")
ASR_MODEL = os.environ.get("LIFE_ASR_MODEL", "vibevoice-asr")
_TAG_RE = re.compile(r"\[[A-Za-z][A-Za-z ]{1,30}\]")  # sound tags such as [Environmental Sounds]


def transcribe_audio(audio):
    """Send audio to the local speech model and return clean text."""
    boundary = "----lifeasr" + hashlib.sha1(audio[:64]).hexdigest()[:16]
    parts = []
    for name, value in (("model", ASR_MODEL), ("language", "ko")):
        parts.append(f'--{boundary}\r\nContent-Disposition: form-data; name="{name}"\r\n\r\n{value}\r\n'.encode())
    parts.append(f'--{boundary}\r\nContent-Disposition: form-data; name="file"; filename="a.wav"\r\nContent-Type: audio/wav\r\n\r\n'.encode())
    body = b"".join(parts) + audio + f"\r\n--{boundary}--\r\n".encode()
    req = urllib.request.Request(ASR_URL, body, {"Content-Type": f"multipart/form-data; boundary={boundary}"})
    with urllib.request.urlopen(req, timeout=300) as r:
        text = json.load(r).get("text", "")
    return re.sub(r"\s{2,}", " ", _TAG_RE.sub("", text)).strip()
