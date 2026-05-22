from __future__ import annotations

import json

from ansible.plugins.callback import CallbackBase


class CallbackModule(CallbackBase):
    CALLBACK_VERSION = 2.0
    CALLBACK_TYPE = "aggregate"
    CALLBACK_NAME = "tugboat_stats"
    CALLBACK_NEEDS_ENABLED = True

    def v2_playbook_on_stats(self, stats):
        summaries = {
            host: stats.summarize(host)
            for host in sorted(stats.processed)
        }
        self._display.display(
            "TUGBOAT_ANSIBLE_STATS_JSON=" + json.dumps(summaries, sort_keys=True)
        )
