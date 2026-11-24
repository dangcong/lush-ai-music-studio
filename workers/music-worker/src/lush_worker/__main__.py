"""Entry point: `python -m lush_worker` or the `lush-worker` console script."""
from __future__ import annotations

import logging
import signal
import sys

from . import __version__
from .api_client import ControlPlaneClient
from .audio import ffmpeg_path
from .captioning import discover_captioners
from .config import ConfigError, load_config
from .engines import build_engines
from .gpu import query_gpu
from .runner import ProtocolMismatch, Runner
from .scoring import load_scorers
from .training import build_trainer

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
    trainer = build_trainer(config, engines, query_gpu)
    if trainer:
        log.info("training enabled: trainer=sidestep acestep_dir=%s data_dir=%s", config.acestep_dir,
                 config.data_dir)
    captioners = discover_captioners(config.captioner_dir)
    if captioners:
        log.info("captioning enabled: %s", [f"{c.key}@{c.version}" for c in captioners])
    scorers = load_scorers(config.scorer_dir)
    if scorers:
        log.info("scorers installed: %s", [f"{s.key}@{s.version}" for s in scorers.values()])
    runner = Runner(config, engines, api, trainer=trainer, captioners=captioners, scorers=scorers)

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
