"""Mail account settings shared by the mail tools: ~/.config/vault-sync/env (secrets) and sources.toml ([mail] scope)."""
import tomllib
from pathlib import Path

ENV_FILE = Path.home() / ".config/vault-sync/env"
SOURCES_FILE = Path.home() / ".config/vault-sync/sources.toml"


def load():
    cfg = {}
    if ENV_FILE.exists():
        for line in ENV_FILE.read_text(encoding="utf-8").splitlines():
            line = line.strip()
            if line and not line.startswith("#") and "=" in line:
                k, _, v = line.partition("=")
                cfg[k.strip()] = v.strip().strip('"').strip("'")
    if SOURCES_FILE.exists():  # sources.toml (non-secret settings: what to collect) overrides env
        m = tomllib.loads(SOURCES_FILE.read_text(encoding="utf-8")).get("mail", {})
        for toml_key, env_key in (("host", "IMAP_HOST"), ("port", "IMAP_PORT"), ("account", "MAIL_ACCOUNT"), ("since", "MAIL_SINCE")):
            if toml_key in m:
                cfg[env_key] = str(m[toml_key])
        if "folders" in m:
            cfg["MAIL_FOLDERS"] = ",".join(m["folders"])
    return cfg
