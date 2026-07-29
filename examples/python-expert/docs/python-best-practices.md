# Python Best Practices

## Type Hints

Always annotate public functions and class attributes. Use `from __future__ import annotations` for forward references.

```python
from __future__ import annotations
from typing import Sequence

def process_items(items: Sequence[str], max_count: int = 10) -> list[str]:
    return [item.strip() for item in items[:max_count] if item.strip()]
```

Use `Optional[X]` only for Python 3.9 and below. For 3.10+ prefer `X | None`.

## Dataclasses and Pydantic

Prefer dataclasses for simple data containers, Pydantic for validation and serialization.

```python
from dataclasses import dataclass, field
from typing import ClassVar

@dataclass
class Config:
    host: str = "localhost"
    port: int = 8080
    tags: list[str] = field(default_factory=list)
    MAX_CONNECTIONS: ClassVar[int] = 100
```

Never use mutable defaults directly — always use `field(default_factory=...)`.

## Error Handling

Catch specific exceptions, never bare `except:`. Always re-raise or log with context.

```python
import logging
log = logging.getLogger(__name__)

def load_config(path: str) -> dict:
    try:
        with open(path) as f:
            return json.load(f)
    except FileNotFoundError:
        raise FileNotFoundError(f"Config not found: {path}") from None
    except json.JSONDecodeError as e:
        log.error("Invalid JSON in %s: %s", path, e)
        raise
```

## Context Managers

Use `contextlib.contextmanager` for simple cases, `__enter__`/`__exit__` for classes.

```python
from contextlib import contextmanager

@contextmanager
def timer(label: str):
    import time
    start = time.perf_counter()
    try:
        yield
    finally:
        print(f"{label}: {time.perf_counter() - start:.3f}s")
```

## Generators and Itertools

Prefer generators over building full lists when you don't need random access.

```python
import itertools

def read_chunks(path: str, size: int = 4096):
    with open(path, "rb") as f:
        yield from iter(lambda: f.read(size), b"")

# Chain multiple iterables without copying
combined = itertools.chain(list1, list2, list3)
```

## Async Python

Use `asyncio` and `httpx` for async HTTP. Never mix sync and async without `asyncio.run()`.

```python
import asyncio
import httpx

async def fetch_all(urls: list[str]) -> list[dict]:
    async with httpx.AsyncClient(timeout=30) as client:
        tasks = [client.get(url) for url in urls]
        responses = await asyncio.gather(*tasks, return_exceptions=True)
    return [r.json() for r in responses if isinstance(r, httpx.Response)]
```

## Testing with pytest

Structure: `tests/` mirrors `src/`. Name: `test_<module>.py`. Functions: `test_<what>_<when>_<expected>`.

```python
import pytest
from myapp.utils import process_items

@pytest.mark.parametrize("items,expected", [
    (["a", "b", "c"], ["a", "b", "c"]),
    (["  a  ", ""], ["a"]),
    ([], []),
])
def test_process_items_strips_and_filters(items, expected):
    assert process_items(items) == expected

def test_process_items_respects_max_count():
    result = process_items(["a"] * 20, max_count=5)
    assert len(result) == 5
```

## Performance

Profile before optimizing. Use `cProfile` for CPU, `tracemalloc` for memory.

```python
import cProfile
import pstats

with cProfile.Profile() as pr:
    your_function()

stats = pstats.Stats(pr)
stats.sort_stats("cumulative").print_stats(10)
```

Common bottlenecks:
- `str` concatenation in loops → use `"".join(parts)`
- Repeated `dict.get()` in hot path → cache the value
- `list.insert(0, x)` → use `collections.deque`
- `set` vs `list` for membership testing: `x in set_` is O(1)
