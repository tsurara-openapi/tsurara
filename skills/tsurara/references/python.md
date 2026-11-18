# Python

Each operation is a function in `operations/<name>.py` taking the client first and keyword arguments after it; it returns a Result object. The same function works with `Client` and, awaited, with `AsyncClient`.

```python
from tsurara_client import Client, HttpError, Success, unwrap

from .operations.show_pet_by_id import show_pet_by_id
from .pet import Pet


def pet_name(pet_id: str) -> str | None:
    with Client("https://petstore.example.com/v1") as client:
        match show_pet_by_id(client, pet_id=pet_id):
            case Success(data=pet):
                return pet.name
            case HttpError(status=status, error=error):
                print(status, error)
            case other:
                print("failed:", other.kind)
    return None


def load_pet(client: Client[None], pet_id: str) -> Pet:
    # Raises ApiError on any failure.
    return unwrap(show_pet_by_id(client, pet_id=pet_id))
```

- Records are dataclasses and enums are `Enum` classes; the runtime converts JSON with the TypeDefs in `_typedefs.py` (`CODEC.decode` / `CODEC.encode`).
- Optional parameters are keyword arguments that default to `None`; a value that may be absent or `null` uses `MISSING` for absent.
- Tests: with `python.mocks: true`, `mocks/schemas.py` has `mock_pet(**overrides)` and `mocks/operations.py` a constructor per documented status and `show_pet_by_id_route(response=None)` per operation, for the `MockRouter` of `tsurara_client.testing` (`docs/languages/python.md`, "Mocks"). Use them instead of hand-written fixtures.
