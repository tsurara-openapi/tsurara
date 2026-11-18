# Rust

Each operation is an `async fn` in the generated `operations` module: required parameters are arguments, optional ones fields of `{Op}Options` (which implements `Default`). It returns `Result<Success<T>, Failure<E>>`.

```rust
use super::operations::show_pet_by_id;
use tsurara_client::{ErrorBody, Failure};

pub async fn pet_name(client: &tsurara_client::Client, pet_id: &str) -> Option<String> {
    match show_pet_by_id(client, pet_id.to_owned(), Default::default()).await {
        Ok(success) => Some(success.data.name),
        Err(Failure::Http {
            status,
            body: ErrorBody::Documented(error),
            ..
        }) => {
            eprintln!("{status}: {}", error.message);
            None
        }
        Err(failure) => {
            eprintln!("{failure:?}");
            None
        }
    }
}

pub fn client() -> tsurara_client::Client {
    tsurara_client::Client::builder("https://petstore.example.com/v1").build()
}
```

- Types are in the generated `types` module; optional fields are `Option<T>`, nullable ones `Nullable<T>`.
- The runtime re-exports `serde` and `serde_json`; generated code needs no other dependency.
- Tests: with `rust.mocks: true`, `mocks::schemas::mock_pet()` and per operation a `{Op}MockResponse` with a variant per documented status and a function that adds the route to the runtime's `MockTransport` (the `testing` feature; `docs/languages/rust.md`, "Mocks").
