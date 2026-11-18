"""Clients: build a request from an OperationDef, send it with httpx, and read the response into a
Result (design.md 8, spec/README.md). `Client` is synchronous and `AsyncClient` asynchronous; both
take the same options, and generated call functions accept either."""

from __future__ import annotations

import inspect
import json
import time
from collections.abc import AsyncIterator, Awaitable, Callable, Iterator, Mapping
from dataclasses import dataclass
from typing import Any, Generic, TypeVar, cast

import httpx

from ._auth import Credentials, SecurityScheme, authenticate, authenticate_async
from ._codec import ConversionError, TypeDefs
from ._def import OPERATION_DEF_VERSION, OperationDef, ParseAs, ResponseDef
from ._info import OperationInfo, RequestInfo, UndocumentedStatusInfo, operation_info
from ._missing import MISSING
from ._retry import Retry
from ._sequences import SEQUENCES, Reader, SequenceKind
from ._result import (
    AnyResult,
    HttpError,
    NetworkFailure,
    ParseFailure,
    Success,
    TimeoutFailure,
    UndocumentedStatus,
    UnexpectedFailure,
)
from ._serialize import (
    build_cookie,
    build_query,
    build_query_string,
    encode_body,
    expand_path,
    media_type_of,
    serialize_header_param,
)

Ctx = TypeVar("Ctx")

HeaderValues = Mapping[str, Any]
"""Header values. A value that is not a string is serialized like a header parameter."""

Headers = HeaderValues | Callable[[RequestInfo[Ctx]], HeaderValues | None | Awaitable[HeaderValues | None]]
OnResult = Callable[[AnyResult, RequestInfo[Ctx]], object]
OnUndocumentedStatus = Callable[[UndocumentedStatusInfo[Ctx]], object]
Auth = Callable[[SecurityScheme, RequestInfo[Ctx]], "str | None | Awaitable[str | None]"]
"""Returns the credential for a security scheme of the operation, or `None`."""


@dataclass
class _Prepared:
    method: str
    url: str
    headers: dict[str, str]
    content: bytes | None


class _ParseError(Exception):
    pass


def _set_headers(target: dict[str, str], values: HeaderValues | None) -> None:
    for k, v in (values or {}).items():
        if v is not None:
            target[k.lower()] = v if isinstance(v, str) else serialize_header_param(v, {})


def _encode_params(group: Mapping[str, Any] | None, defs: Mapping[str, Any], type_defs: TypeDefs | None) -> dict[str, Any] | None:
    """Drops missing values and encodes typed values with their TypeDefs."""
    if group is None:
        return None
    out: dict[str, Any] = {}
    for name, value in group.items():
        if value is MISSING:
            continue
        ty = defs.get(name, {}).get("type")
        out[name] = type_defs.encode(ty, value) if type_defs is not None and ty is not None else _plain(value)
    return out


def _plain(value: Any) -> Any:
    """Typed values without a TypeDef reference (enums, dataclasses), as JSON."""
    return TypeDefs({"v": 1, "types": {}}).encode("any", value)


def _prepare(
    base_url: str,
    op: OperationDef,
    *,
    path: Mapping[str, Any] | None,
    query: Mapping[str, Any] | None,
    headers: Mapping[str, Any] | None,
    cookies: Mapping[str, Any] | None,
    querystring: Any,
    body: Any,
    content_type: str | None,
    type_defs: TypeDefs | None,
    default_headers: HeaderValues | None,
    credentials: Credentials | None = None,
) -> _Prepared:
    params = op.get("params", {})
    url = base_url.rstrip("/") + expand_path(op["path"], _encode_params(path, params.get("path", {}), type_defs), params.get("path", {}))
    qs = ""
    if "querystring" in params and querystring is not MISSING:
        qs = build_query_string(_plain(querystring), params["querystring"]["contentType"])
    else:
        qs = build_query(_encode_params(query, params.get("query", {}), type_defs), params.get("query", {}))
    if credentials is not None and credentials.query:
        qs = "&".join(p for p in [qs, *credentials.query] if p)
    if qs:
        url += "?" + qs
    out: dict[str, str] = {}
    _set_headers(out, default_headers)
    header_defs = params.get("headers", {})
    for name, value in (_encode_params(headers, header_defs, type_defs) or {}).items():
        if value is not None:
            d = header_defs.get(name, {})
            out[name.lower()] = value if isinstance(value, str) else serialize_header_param(value, d)
    if credentials is not None:
        out.update(credentials.headers)
    cookie = build_cookie(_encode_params(cookies, params.get("cookies", {}), type_defs), params.get("cookies", {}))
    if credentials is not None and credentials.cookies:
        cookie = "; ".join(c for c in [cookie, *credentials.cookies] if c)
    if cookie:
        out["cookie"] = cookie
    content: bytes | None = None
    body_def = op.get("body")
    if body_def is not None and body is not MISSING:
        chosen = content_type or next(iter(body_def["contents"]))
        media = body_def["contents"].get(chosen) or body_def["contents"].get(media_type_of(chosen)) or {}
        ty = media.get("type")
        value = type_defs.encode(ty, body) if type_defs is not None and ty is not None else _plain_body(body)
        encoded = encode_body(value, body_def, content_type)
        content = encoded.content
        if encoded.content_type is not None:
            out["content-type"] = encoded.content_type
    return _Prepared(op["method"].upper(), url, out, content)


def _plain_body(body: Any) -> Any:
    from ._file import File

    if isinstance(body, bytes | File):
        return body
    if isinstance(body, Mapping):
        # Multipart and form bodies keep binary parts as they are.
        return {k: v if isinstance(v, bytes | File) else _plain(v) for k, v in cast(Mapping[str, Any], body).items()}
    return _plain(body)


def _find_response(op: OperationDef, status: int) -> tuple[str, ResponseDef] | None:
    responses = op["responses"]
    for key in (str(status), f"{status // 100}XX", "default"):
        if key in responses:
            return key, responses[key]
    return None


def _has_success_entry(op: OperationDef) -> bool:
    for k in op["responses"]:
        if k.upper().endswith("XX"):
            if int(k[0]) < 4:
                return True
        elif k.isdigit() and int(k) < 400:
            return True
    return False


def _is_json(mt: str) -> bool:
    return mt == "application/json" or mt.endswith("+json")


def _guess(response: httpx.Response) -> ParseAs:
    mt = media_type_of(response.headers.get("content-type"))
    if response.status_code in (204, 205, 304):
        return "none"
    if _is_json(mt):
        return "json"
    if mt.startswith("text/") or mt == "":
        return "text"
    return "blob"


def _read(response: httpx.Response, parse_as: ParseAs) -> Any:
    if parse_as == "none":
        return None
    if parse_as == "text":
        return response.text
    if parse_as in ("blob", "stream"):
        return response.content
    if parse_as == "auto":
        return _read(response, _guess(response))
    mt = media_type_of(response.headers.get("content-type"))
    if mt != "" and not _is_json(mt):
        raise _ParseError(f"expected JSON but the response has Content-Type {mt}")
    try:
        return json.loads(response.content)
    except ValueError as e:
        raise _ParseError("the response body is not valid JSON") from e


def _sequence_of(op: OperationDef, response: httpx.Response) -> tuple[SequenceKind, ResponseDef] | None:
    """The sequence a success response is read as, if its definition says so."""
    status = response.status_code
    found = _find_response(op, status)
    if found is None or status >= 400 or (found[0] == "default" and _has_success_entry(op)):
        return None
    entry = found[1]
    mt = media_type_of(response.headers.get("content-type"))
    parse_as = entry.get("contentTypes", {}).get(mt, entry["parseAs"])
    for kind in SEQUENCES:
        if parse_as == kind:
            return kind, entry
    return None


def _item_decoder(entry: ResponseDef, response: httpx.Response, type_defs: TypeDefs | None) -> Callable[[Any], Any]:
    """Decodes one item of a sequence with its TypeDef, when the definition has one."""
    mt = media_type_of(response.headers.get("content-type"))
    types = entry.get("types", {})
    ty = types.get(mt) or (next(iter(types.values())) if len(types) == 1 else None)
    if type_defs is None or ty is None:
        return lambda item: item
    defs = type_defs
    return lambda item: defs.decode(ty, item)


def _no_read_timeout(response: httpx.Response) -> None:
    """Lets a stream wait for its next item as long as it takes: the timeout covers connecting and
    the response headers, as in `@tsurara/client`. Transports that read the timeout per read (httpx's
    own) see the change."""
    timeout = response.request.extensions.get("timeout")
    if isinstance(timeout, dict):
        timeout["read"] = None


def _iterate(response: httpx.Response, kind: SequenceKind, entry: ResponseDef, type_defs: TypeDefs | None) -> Iterator[Any]:
    reader = Reader(kind, entry.get("jsonEvents"))
    decode = _item_decoder(entry, response, type_defs)
    try:
        for chunk in response.iter_bytes():
            for item in reader.feed(chunk):
                yield decode(item)
        for item in reader.feed(b"", final=True):
            yield decode(item)
    finally:
        response.close()


async def _iterate_async(
    response: httpx.Response, kind: SequenceKind, entry: ResponseDef, type_defs: TypeDefs | None
) -> AsyncIterator[Any]:
    reader = Reader(kind, entry.get("jsonEvents"))
    decode = _item_decoder(entry, response, type_defs)
    try:
        async for chunk in response.aiter_bytes():
            for item in reader.feed(chunk):
                yield decode(item)
        for item in reader.feed(b"", final=True):
            yield decode(item)
    finally:
        await response.aclose()


def _read_undocumented(response: httpx.Response) -> Any:
    parse_as = _guess(response)
    if parse_as == "json":
        try:
            return json.loads(response.content)
        except ValueError:
            return response.text
    return _read(response, parse_as)


def _to_result(op: OperationDef, response: httpx.Response, type_defs: TypeDefs | None, info: OperationInfo) -> AnyResult:
    status = response.status_code
    found = _find_response(op, status)
    content_type = response.headers.get("content-type")
    mt = media_type_of(content_type)

    def parse(entry: ResponseDef) -> Any:
        parse_as = entry.get("contentTypes", {}).get(mt, entry["parseAs"])
        if parse_as == "auto":
            parse_as = _guess(response)
        data = _read(response, parse_as)
        types = entry.get("types", {})
        ty = types.get(mt) or (next(iter(types.values())) if len(types) == 1 else None)
        if type_defs is None or ty is None or parse_as != "json":
            return data
        try:
            return type_defs.decode(ty, data)
        except ConversionError as e:
            raise _ParseError(str(e)) from e

    is_default = found is not None and found[0] == "default"
    success = status < 400 and found is not None and (not is_default or not _has_success_entry(op))
    try:
        if success and found is not None:
            return Success(status, parse(found[1]), response, operation=info)
        if found is not None and status >= 400:
            return HttpError(status, parse(found[1]), response, operation=info)
        return UndocumentedStatus(status, _read_undocumented(response), response, operation=info)
    except _ParseError as e:
        return ParseFailure(e, response, content_type, operation=info)


@dataclass(frozen=True)
class _Options(Generic[Ctx]):
    base_url: str
    headers: Headers[Ctx] | None
    timeout: float | None
    on_result: OnResult[Ctx] | None
    on_undocumented_status: OnUndocumentedStatus[Ctx] | None
    retry: Retry | None
    auth: Auth[Ctx] | None


class _Deadline:
    """The client's timeout over all attempts."""

    def __init__(self, seconds: float | None) -> None:
        self.end = None if seconds is None else time.monotonic() + seconds

    def remaining(self) -> Any:
        """The timeout for the next attempt, for httpx."""
        if self.end is None:
            return httpx.USE_CLIENT_DEFAULT
        return httpx.Timeout(max(0.0, self.end - time.monotonic()))

    def fits(self, wait: float) -> bool:
        return self.end is None or time.monotonic() + wait < self.end


def _retryable(error: Exception) -> bool:
    return isinstance(error, httpx.TransportError) and not isinstance(error, httpx.TimeoutException)


def _failure(cause: BaseException, info: OperationInfo) -> AnyResult:
    if isinstance(cause, httpx.TimeoutException):
        return TimeoutFailure(cause, operation=info)
    if isinstance(cause, httpx.TransportError):
        return NetworkFailure(cause, operation=info)
    return UnexpectedFailure(cause, operation=info)


def _version_error(op: OperationDef, info: OperationInfo) -> AnyResult | None:
    if op.get("v") != OPERATION_DEF_VERSION:
        return UnexpectedFailure(
            ValueError(
                f"tsurara: OperationDef version {op.get('v')!r} is not supported by this tsurara-client "
                f"(expected {OPERATION_DEF_VERSION}); regenerate the code or update tsurara-client"
            ),
            operation=info,
        )
    return None


class Client(Generic[Ctx]):
    """A synchronous client. Pass it as the first argument of generated functions.

    `http` is the `httpx.Client` to send with (for proxies, transports, event hooks or tests); by
    default the client makes its own. `timeout` is in seconds. `on_result` observes every result and
    `on_undocumented_status` every response whose status the definition does not document;
    exceptions they raise are ignored.
    """

    def __init__(
        self,
        base_url: str,
        *,
        headers: Headers[Ctx] | None = None,
        timeout: float | None = None,
        http: httpx.Client | None = None,
        on_result: OnResult[Ctx] | None = None,
        on_undocumented_status: OnUndocumentedStatus[Ctx] | None = None,
        retry: Retry | None = None,
        auth: Auth[Ctx] | None = None,
    ) -> None:
        self.options = _Options(base_url, headers, timeout, on_result, on_undocumented_status, retry, auth)
        self._owns_http = http is None
        self._http = http if http is not None else httpx.Client()

    def __enter__(self) -> Client[Ctx]:
        return self

    def __exit__(self, *_: object) -> None:
        self.close()

    def close(self) -> None:
        """Closes the httpx client, if this client made it."""
        if self._owns_http:
            self._http.close()

    def request(
        self,
        operation: OperationDef,
        *,
        path: Mapping[str, Any] | None = None,
        query: Mapping[str, Any] | None = None,
        headers: Mapping[str, Any] | None = None,
        cookies: Mapping[str, Any] | None = None,
        querystring: Any = MISSING,
        body: Any = MISSING,
        content_type: str | None = None,
        type_defs: TypeDefs | None = None,
        timeout: float | None = None,
        context: Ctx | None = None,
    ) -> AnyResult:
        """Calls an operation. Never raises for the outcomes of a request; returns a Result."""
        info = operation_info(operation)
        result = _version_error(operation, info) or self._run(
            operation, info, path, query, headers, cookies, querystring, body, content_type, type_defs, timeout, context
        )
        _observe(self.options, result, info, context)
        return result

    def _run(
        self,
        op: OperationDef,
        info: OperationInfo,
        path: Mapping[str, Any] | None,
        query: Mapping[str, Any] | None,
        headers: Mapping[str, Any] | None,
        cookies: Mapping[str, Any] | None,
        querystring: Any,
        body: Any,
        content_type: str | None,
        type_defs: TypeDefs | None,
        timeout: float | None,
        context: Ctx | None,
    ) -> AnyResult:
        try:
            defaults = self.options.headers
            if callable(defaults):
                defaults = defaults(RequestInfo(info, context))
                if inspect.isawaitable(defaults):
                    raise TypeError("an async headers function needs AsyncClient")
            auth = self.options.auth
            request_info = RequestInfo(info, context)
            credentials = authenticate(
                op.get("security"), lambda scheme: auth(scheme, request_info) if auth is not None else None
            )
            prepared = _prepare(
                self.options.base_url,
                op,
                path=path,
                query=query,
                headers=headers,
                cookies=cookies,
                querystring=querystring,
                body=body,
                content_type=content_type,
                type_defs=type_defs,
                default_headers=defaults,
                credentials=credentials,
            )
            response = self._send(prepared, timeout if timeout is not None else self.options.timeout)
            try:
                sequence = _sequence_of(op, response)
                if sequence is not None:
                    _no_read_timeout(response)
                    kind, entry = sequence
                    data = _iterate(response, kind, entry, type_defs)
                    return Success(response.status_code, data, response, operation=info)
                response.read()
            except BaseException:
                response.close()
                raise
            response.close()
            return _to_result(op, response, type_defs, info)
        except Exception as e:
            return _failure(e, info)


    def _send(self, prepared: _Prepared, timeout: float | None) -> httpx.Response:
        """Sends, with retries when the client has them."""
        retry = self.options.retry
        retries = retry.retries(prepared.method) if retry is not None else 0
        deadline = _Deadline(timeout)
        attempt = 0
        while True:
            try:
                request = self._http.build_request(
                    prepared.method,
                    prepared.url,
                    headers=prepared.headers,
                    content=prepared.content,
                    timeout=deadline.remaining(),
                )
                response = self._http.send(request, stream=True)
            except Exception as e:
                if retry is None or attempt >= retries or not _retryable(e):
                    raise
                wait = retry.delay(attempt + 1, None)
                if not deadline.fits(wait):
                    raise
                retry.sleep(wait)
                attempt += 1
                continue
            if retry is None or attempt >= retries or response.status_code not in retry.statuses:
                return response
            wait = retry.delay(attempt + 1, response)
            if not deadline.fits(wait):
                return response
            response.close()
            retry.sleep(wait)
            attempt += 1


class AsyncClient(Generic[Ctx]):
    """An asynchronous client, with the options of `Client`; `http` is an `httpx.AsyncClient`, and
    `headers` may be an async function. Cancelling the task raises `asyncio.CancelledError` as usual
    instead of returning a result."""

    def __init__(
        self,
        base_url: str,
        *,
        headers: Headers[Ctx] | None = None,
        timeout: float | None = None,
        http: httpx.AsyncClient | None = None,
        on_result: OnResult[Ctx] | None = None,
        on_undocumented_status: OnUndocumentedStatus[Ctx] | None = None,
        retry: Retry | None = None,
        auth: Auth[Ctx] | None = None,
    ) -> None:
        self.options = _Options(base_url, headers, timeout, on_result, on_undocumented_status, retry, auth)
        self._owns_http = http is None
        self._http = http if http is not None else httpx.AsyncClient()

    async def __aenter__(self) -> AsyncClient[Ctx]:
        return self

    async def __aexit__(self, *_: object) -> None:
        await self.aclose()

    async def aclose(self) -> None:
        """Closes the httpx client, if this client made it."""
        if self._owns_http:
            await self._http.aclose()

    async def _send(self, prepared: _Prepared, timeout: float | None) -> httpx.Response:
        return await _send_async(self._http, self.options.retry, prepared, timeout)

    async def request(
        self,
        operation: OperationDef,
        *,
        path: Mapping[str, Any] | None = None,
        query: Mapping[str, Any] | None = None,
        headers: Mapping[str, Any] | None = None,
        cookies: Mapping[str, Any] | None = None,
        querystring: Any = MISSING,
        body: Any = MISSING,
        content_type: str | None = None,
        type_defs: TypeDefs | None = None,
        timeout: float | None = None,
        context: Ctx | None = None,
    ) -> AnyResult:
        """Calls an operation. Never raises for the outcomes of a request; returns a Result."""
        info = operation_info(operation)
        result = _version_error(operation, info)
        if result is None:
            try:
                defaults = self.options.headers
                if callable(defaults):
                    defaults = defaults(RequestInfo(info, context))
                    if inspect.isawaitable(defaults):
                        defaults = await defaults
                auth = self.options.auth
                request_info = RequestInfo(info, context)
                credentials = await authenticate_async(
                    operation.get("security"),
                    lambda scheme: auth(scheme, request_info) if auth is not None else None,
                )
                prepared = _prepare(
                    self.options.base_url,
                    operation,
                    path=path,
                    query=query,
                    headers=headers,
                    cookies=cookies,
                    querystring=querystring,
                    body=body,
                    content_type=content_type,
                    type_defs=type_defs,
                    default_headers=defaults,
                    credentials=credentials,
                )
                response = await self._send(prepared, timeout if timeout is not None else self.options.timeout)
                try:
                    sequence = _sequence_of(operation, response)
                    if sequence is None:
                        await response.aread()
                except BaseException:
                    await response.aclose()
                    raise
                if sequence is not None:
                    _no_read_timeout(response)
                    kind, entry = sequence
                    data = _iterate_async(response, kind, entry, type_defs)
                    result = Success(response.status_code, data, response, operation=info)
                else:
                    await response.aclose()
                    result = _to_result(operation, response, type_defs, info)
            except Exception as e:
                result = _failure(e, info)
        await _observe_async(self.options, result, info, context)
        return result


async def _send_async(http: httpx.AsyncClient, retry: Retry | None, prepared: _Prepared, timeout: float | None) -> httpx.Response:
    """`Client._send` for `AsyncClient`."""
    retries = retry.retries(prepared.method) if retry is not None else 0
    deadline = _Deadline(timeout)
    attempt = 0
    while True:
        try:
            request = http.build_request(
                prepared.method,
                prepared.url,
                headers=prepared.headers,
                content=prepared.content,
                timeout=deadline.remaining(),
            )
            response = await http.send(request, stream=True)
        except Exception as e:
            if retry is None or attempt >= retries or not _retryable(e):
                raise
            wait = retry.delay(attempt + 1, None)
            if not deadline.fits(wait):
                raise
            await retry.async_sleep(wait)
            attempt += 1
            continue
        if retry is None or attempt >= retries or response.status_code not in retry.statuses:
            return response
        wait = retry.delay(attempt + 1, response)
        if not deadline.fits(wait):
            return response
        await response.aclose()
        await retry.async_sleep(wait)
        attempt += 1


def _observe(options: _Options[Ctx], result: AnyResult, info: OperationInfo, context: Ctx | None) -> None:
    """Calls the observation hooks; their exceptions do not change the result."""
    if options.on_undocumented_status is not None and isinstance(result, UndocumentedStatus):
        try:
            options.on_undocumented_status(UndocumentedStatusInfo(info, context, result.status, result.response))
        except Exception:
            pass
    if options.on_result is not None:
        try:
            options.on_result(result, RequestInfo(info, context))
        except Exception:
            pass


async def _observe_async(options: _Options[Ctx], result: AnyResult, info: OperationInfo, context: Ctx | None) -> None:
    """`_observe` for `AsyncClient`: hooks may be async functions."""
    calls: list[Callable[[], object]] = []
    if options.on_undocumented_status is not None and isinstance(result, UndocumentedStatus):
        hook = options.on_undocumented_status
        calls.append(lambda: hook(UndocumentedStatusInfo(info, context, result.status, result.response)))
    if options.on_result is not None:
        on_result = options.on_result
        calls.append(lambda: on_result(result, RequestInfo(info, context)))
    for call in calls:
        try:
            outcome = call()
            if inspect.isawaitable(outcome):
                await outcome
        except Exception:
            pass
