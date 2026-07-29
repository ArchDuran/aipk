"""
AIPK Pipeline for Open WebUI
https://github.com/open-webui/pipelines

Установка:
  1. Settings → Pipelines → загрузи этот файл
  2. Настрой AIPK_SERVER_URL в Valves
  3. Выбери "AIPK Pipeline" как модель в чате

Требования (на хосте Open WebUI):
  pip install httpx
"""
from typing import AsyncGenerator, Iterator, List, Optional, Union
from pydantic import BaseModel
import httpx


class Pipeline:
    class Valves(BaseModel):
        AIPK_SERVER_URL: str = "http://localhost:8080"
        MODEL: str = "llama3.2"
        SHOW_SOURCES: bool = True  # добавить найденные чанки в ответ

    def __init__(self):
        self.name = "AIPK Pipeline"
        self.valves = self.Valves()

    async def on_startup(self):
        try:
            async with httpx.AsyncClient(timeout=5) as client:
                resp = await client.get(f"{self.valves.AIPK_SERVER_URL}/health")
                data = resp.json()
                stats = data.get("stats", {})
                print(
                    f"[AIPK] Connected — chunks={stats.get('chunks', '?')} "
                    f"rag={stats.get('rag_active', '?')} "
                    f"skills={stats.get('skills', '?')}"
                )
        except Exception as e:
            print(f"[AIPK] Warning: server not reachable — {e}")

    async def on_shutdown(self):
        pass

    async def pipe(
        self,
        user_message: str,
        model_id: str,
        messages: List[dict],
        body: dict,
    ) -> Union[str, AsyncGenerator]:
        base = self.valves.AIPK_SERVER_URL.rstrip("/")
        payload = {**body, "model": self.valves.MODEL, "messages": messages}
        stream: bool = body.get("stream", False)

        async with httpx.AsyncClient(timeout=120) as client:
            if stream:
                return self._stream(client, base, payload)
            else:
                resp = await client.post(
                    f"{base}/v1/chat/completions", json=payload
                )
                data = resp.json()
                content = data["choices"][0]["message"]["content"]

                if self.valves.SHOW_SOURCES:
                    sources = await self._fetch_sources(client, base, user_message)
                    if sources:
                        previews = "; ".join(
                            s[:80].replace("\n", " ") + ("…" if len(s) > 80 else "")
                            for s in sources[:3]
                        )
                        content += f"\n\n---\n*Sources: {previews}*"

                return content

    async def _stream(self, client: httpx.AsyncClient, base: str, payload: dict):
        async with client.stream("POST", f"{base}/v1/chat/completions", json=payload) as resp:
            async for line in resp.aiter_lines():
                if line.startswith("data: ") and line != "data: [DONE]":
                    yield line + "\n\n"

    async def _fetch_sources(
        self, client: httpx.AsyncClient, base: str, query: str
    ) -> List[str]:
        try:
            resp = await client.post(
                f"{base}/v1/retrieve",
                json={"query": query},
                timeout=10,
            )
            if resp.status_code == 200:
                return resp.json().get("chunks", [])
        except Exception:
            pass
        return []
