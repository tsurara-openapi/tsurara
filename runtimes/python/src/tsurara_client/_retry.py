"""Retries (design.md 8.4), with the defaults of @tsurara/client: idempotent methods, network failures
and the statuses 408, 429, 500, 502, 503 and 504, up to 2 retries, waiting `Retry-After` when the
response has it and otherwise exponential backoff with full jitter."""

from __future__ import annotations

import asyncio
import random as _random
import time
from collections.abc import Awaitable, Callable
from dataclasses import dataclass, field
from email.utils import mktime_tz, parsedate_tz

import httpx


def retry_after(response: httpx.Response | None, now: float | None = None) -> float | None:
    """`Retry-After` in seconds: seconds or an HTTP date."""
    value = response.headers.get("retry-after", "").strip() if response is not None else ""
    if not value:
        return None
    if value.isdigit():
        return float(value)
    parsed = parsedate_tz(value)
    if parsed is None:
        return None
    return max(0.0, mktime_tz(parsed) - (time.time() if now is None else now))


@dataclass(frozen=True)
class Retry:
    """Retry options for `Client(retry=...)` and `AsyncClient(retry=...)`. Times are in seconds.

    The client's timeout covers all attempts together: a wait that would end after it is not started,
    and the last response is returned. Timeouts are not retried.
    """

    max: int = 2
    """Retries after the first attempt."""
    methods: frozenset[str] = frozenset({"GET", "HEAD", "OPTIONS", "PUT", "DELETE"})
    statuses: frozenset[int] = frozenset({408, 429, 500, 502, 503, 504})
    base_delay: float = 0.1
    """The first backoff, doubled on every retry."""
    max_delay: float = 30.0
    """The longest wait, also for `Retry-After`."""
    random: Callable[[], float] = field(default=_random.random, repr=False)
    sleep: Callable[[float], object] = field(default=time.sleep, repr=False)
    async_sleep: Callable[[float], Awaitable[object]] = field(default=asyncio.sleep, repr=False)

    def delay(self, attempt: int, response: httpx.Response | None) -> float:
        """Seconds to wait before retry number `attempt` (from 1); `response` is `None` after a
        network failure."""
        after = retry_after(response)
        if after is not None:
            return min(after, self.max_delay)
        return self.random() * min(self.max_delay, self.base_delay * 2 ** (attempt - 1))

    def retries(self, method: str) -> int:
        return self.max if method.upper() in self.methods else 0
