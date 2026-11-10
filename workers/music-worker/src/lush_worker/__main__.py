"""Entry point: `python -m lush_worker` or the `lush-worker` console script."""
from __future__ import annotations

import logging
import signal
import sys

from . import __version__
from .api_client import ControlPlaneClient
from .audio import ffmpeg_path
from .config import ConfigError, load_config
from .engines import build_engines
from .runner import ProtocolMismatch, Runner

log = logging.getLogger("lush_worker")


def main() -> int:
    try:
        config = load_config()
    except ConfigError as e:
        print(f"configuration error: {e}", file=sys.stderr)
        return 2
    logging.basicConfig(level=getattr(logging, config.log_level, logging.INFO),
                        format="%(asctime)s %(levelname)s %(name)s: %(message)s")
    logging.getLogger("httpx").setLevel(logging.WARNING)  # do not log every poll
    logging.getLogger("httpcore").setLevel(logging.WARNING)

    log.info("lush-worker %s starting: control plane=%s engines=%s work_dir=%s",
             __version__, config.control_plane_url, list(config.engines), config.work_dir)
    if not ffmpeg_path():
        log.warning("ffmpeg not found in PATH: jobs will complete without an MP3 preview artifact")
    engines = build_engines(config)
    for engine in engines.values():
        problem = engine.health()
        if problem:
            log.warning("engine %s is not ready: %s", engine.name, problem)

    api = ControlPlaneClient(config.control_plane_url, config.node_token)
    runner = Runner(config, engines, api)

    def on_signal(signum, _frame):
        if runner.stopping:  # second signal: stop waiting
            raise KeyboardInterrupt
        log.info("signal %s received: finishing up (the current job is released back to the queue)", signum)
        runner.stop()

    signal.signal(signal.SIGINT, on_signal)
    signal.signal(signal.SIGTERM, on_signal)

    try:
        runner.run_forever()
    except ProtocolMismatch as e:
        log.error("%s", e)
        return 3
    finally:
        api.close()
    return 0


if __name__ == "__main__":
    sys.exit(main())
