"""Explicit, bounded model requests from the ReactQM research panel."""
import json
import os
import re
import sys
from urllib.error import HTTPError, URLError
from urllib.parse import urlsplit
from urllib.request import HTTPRedirectHandler, ProxyHandler, Request, build_opener

LIMIT = 2 * 1024 * 1024


class RejectRedirect(HTTPRedirectHandler):
    def redirect_request(self, req, fp, code, msg, headers, newurl):
        raise HTTPError(req.full_url, code, "Redirect rejected", headers, fp)


def ask(provider, model, prompt):
    if (not isinstance(model, str) or not re.fullmatch(r"[A-Za-z0-9_.:/-]{1,128}", model)
            or not isinstance(prompt, str) or not prompt.strip() or len(prompt.encode()) > 16000):
        raise ValueError("research_input")
    headers = {"Content-Type": "application/json"}
    if provider == "gemini":
        if not re.fullmatch(r"[A-Za-z0-9_-]{1,128}", model):
            raise ValueError("research_input")
        key = os.environ.get("GEMINI_API_KEY")
        if not key:
            raise ValueError("research_key")
        headers["x-goog-api-key"] = key
        url = f"https://generativelanguage.googleapis.com/v1beta/models/{model}:generateContent"
        body = {"contents": [{"role": "user", "parts": [{"text": prompt}]}],
                "generationConfig": {"maxOutputTokens": 2048}}
    elif provider in ("openai", "local"):
        if provider == "openai":
            url = "https://api.openai.com/v1/chat/completions"
            key = os.environ.get("OPENAI_API_KEY")
            if not key:
                raise ValueError("research_key")
        else:
            base = urlsplit(os.environ.get("REACTQM_LOCAL_API", "http://127.0.0.1:11434/v1"))
            if (base.scheme != "http" or base.hostname not in ("127.0.0.1", "::1")
                    or base.username or base.password or base.query or base.fragment or base.path.rstrip("/") != "/v1"):
                raise ValueError("research_local_endpoint")
            url = base.geturl().rstrip("/") + "/chat/completions"
            key = os.environ.get("REACTQM_LOCAL_API_KEY")
        if key:
            headers["Authorization"] = f"Bearer {key}"
        body = {"model": model, "messages": [{"role": "user", "content": prompt}]}
        body["max_completion_tokens" if provider == "openai" else "max_tokens"] = 2048
    else:
        raise ValueError("research_input")

    request = Request(url, data=json.dumps(body).encode(), headers=headers, method="POST")
    opener = build_opener(ProxyHandler({}), RejectRedirect())
    try:
        with opener.open(request, timeout=35) as response:
            raw = response.read(LIMIT + 1)
    except HTTPError as error:
        raise ValueError(f"research_http_{error.code}") from None
    except (URLError, TimeoutError, OSError):
        raise ValueError("research_connection") from None
    if len(raw) > LIMIT:
        raise ValueError("research_response")
    result = json.loads(raw)
    if provider == "gemini":
        answer = "".join(part.get("text", "") for part in result["candidates"][0]["content"]["parts"]
                         if not part.get("thought"))
    else:
        answer = result["choices"][0]["message"]["content"]
    if not isinstance(answer, str) or not answer.strip() or len(answer.encode()) > 40000:
        raise ValueError("research_response")
    return answer


def serve():
    try:
        raw = sys.stdin.buffer.read(65537)
        if len(raw) > 65536:
            raise ValueError("research_input")
        request = json.loads(raw)
        result = {"answer": ask(request["provider"], request["model"], request["prompt"])}
    except ValueError as error:
        code = str(error)
        result = {"error": code if re.fullmatch(r"research_[a-z_]+(?:[0-9]+)?", code) else "research_input"}
    except (KeyError, IndexError, TypeError, AttributeError):
        result = {"error": "research_response"}
    print(json.dumps(result, ensure_ascii=False))
    return 0
