"""life-agent command line: run, once, flush, status, config, install, uninstall."""
import argparse
import signal
import sys
import time

from life_agent import agent as agent_mod, backends, config, spool
from life_agent.segments import Segmenter

RUN_KEY = r"Software\Microsoft\Windows\CurrentVersion\Run"
RUN_NAME = "LifeAgent"


def cmd_run(cfg):
    if not cfg["url"] or not cfg["token"]:
        print(f"Set url and token in {config.config_path()} first (life-agent config creates it).", file=sys.stderr)
        return 2
    a = agent_mod.Agent(cfg, backends.load())
    stopping = []
    for sig in (signal.SIGINT, signal.SIGTERM):
        signal.signal(sig, lambda *_: stopping.append(1))
    agent_mod.log("started")
    while not stopping:
        a.step()
        time.sleep(cfg["interval_s"])
    a.stop()
    agent_mod.log("stopped")
    return 0


def cmd_once(cfg):
    """Look once at what is in front and print it (no record is stored or sent): the first check on a new machine."""
    app, title, idle = backends.load().sample()
    shown = Segmenter(cfg)._title(app, title) if app else ""
    print(f"app={app or '(none: locked or no window)'}  title={shown!r}  idle={idle:.0f}s")
    return 0


def cmd_flush(cfg):
    sent, err = agent_mod.Agent(cfg, None).flush()
    print(f"sent {sent} records" + (f", problem: {err}" if err else ""))
    return 1 if err else 0


def cmd_status(cfg):
    print(f"config:  {config.config_path()}  (url {'set' if cfg['url'] else 'NOT set'}, token {'set' if cfg['token'] else 'NOT set'})")
    print(f"spool:   {len(spool.read())} records waiting  ({spool.path()})")
    print(f"log:     {config.home() / 'agent.log'}")
    return 0


def cmd_config(cfg):
    print(config.write_template())
    return 0


def _autostart(on):
    if not sys.platform.startswith("win"):
        print("Autostart is only set up on Windows for now.", file=sys.stderr)
        return 2
    import winreg
    with winreg.OpenKey(winreg.HKEY_CURRENT_USER, RUN_KEY, 0, winreg.KEY_SET_VALUE) as k:
        if on:
            pyw = sys.executable.replace("python.exe", "pythonw.exe")
            script = __file__.rsplit("life_agent", 1)[0] + "life-agent.pyw"
            winreg.SetValueEx(k, RUN_NAME, 0, winreg.REG_SZ, f'"{pyw}" "{script}" run')
            print("The agent will start when you sign in.")
        else:
            try:
                winreg.DeleteValue(k, RUN_NAME)
            except FileNotFoundError:
                pass
            print("Autostart removed.")
    return 0


COMMANDS = {"run": cmd_run, "once": cmd_once, "flush": cmd_flush, "status": cmd_status, "config": cmd_config,
            "install": lambda cfg: _autostart(True), "uninstall": lambda cfg: _autostart(False)}


def main(argv=None):
    ap = argparse.ArgumentParser(prog="life-agent", description=__doc__)
    ap.add_argument("command", choices=sorted(COMMANDS))
    args = ap.parse_args(argv)
    try:
        return COMMANDS[args.command](config.load())
    except NotImplementedError as e:
        print(e, file=sys.stderr)
        return 2
