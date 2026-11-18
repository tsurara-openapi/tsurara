use std::time::Duration;
use url::Url;

#[derive(Debug, thiserror::Error)]
pub enum FetchError {
    #[error("failed to read {url}: {source}")]
    Io {
        url: Url,
        #[source]
        source: std::io::Error,
    },
    #[error("failed to fetch {url}: {message}")]
    Http { url: Url, message: String },
    #[error("unsupported URL `{0}`")]
    Unsupported(Url),
}

/// Source of definitions. Can be replaced with a custom HTTP stack or cache.
pub trait Fetcher {
    fn fetch(&self, url: &Url) -> Result<Vec<u8>, FetchError>;
}

impl<F: Fetcher + ?Sized> Fetcher for &F {
    fn fetch(&self, url: &Url) -> Result<Vec<u8>, FetchError> {
        (**self).fetch(url)
    }
}

/// HTTP options. `headers` are only sent with requests to the same origin as `origin`,
/// so that auth tokens are not sent to another host that an external `$ref` points to.
#[derive(Debug, Clone, Default)]
pub struct HttpOptions {
    pub headers: Vec<(String, String)>,
    pub origin: Option<url::Origin>,
    pub timeout: Option<Duration>,
}

/// Default implementation handling `file://` and (with the `http` feature) `http(s)://`.
#[derive(Debug, Clone, Default)]
pub struct DefaultFetcher {
    pub http: HttpOptions,
}

impl DefaultFetcher {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_http(http: HttpOptions) -> Self {
        Self { http }
    }
}

impl Fetcher for DefaultFetcher {
    fn fetch(&self, url: &Url) -> Result<Vec<u8>, FetchError> {
        match url.scheme() {
            "file" => {
                let path = url
                    .to_file_path()
                    .map_err(|()| FetchError::Unsupported(url.clone()))?;
                std::fs::read(&path).map_err(|source| FetchError::Io {
                    url: url.clone(),
                    source,
                })
            }
            #[cfg(feature = "http")]
            "http" | "https" => http::fetch(&self.http, url),
            _ => Err(FetchError::Unsupported(url.clone())),
        }
    }
}

#[cfg(feature = "http")]
mod http {
    use super::{FetchError, HttpOptions};
    use url::Url;

    const MAX_BODY: u64 = 1 << 30;

    const MAX_REDIRECTS: usize = 10;

    /// Fetches `url`, following redirects itself so that the input headers are only sent to the
    /// origin they are meant for (ureq would keep every header but `Authorization` on a redirect to
    /// another host), and only to `http(s)` URLs.
    pub(super) fn fetch(options: &HttpOptions, url: &Url) -> Result<Vec<u8>, FetchError> {
        let agent: ureq::Agent = ureq::Agent::config_builder()
            .timeout_global(options.timeout)
            .max_redirects(0)
            .http_status_as_error(false)
            .build()
            .into();
        let mut current = url.clone();
        for _ in 0..=MAX_REDIRECTS {
            let failed = |message: String| FetchError::Http {
                url: current.clone(),
                message,
            };
            if !matches!(current.scheme(), "http" | "https") {
                return Err(FetchError::Http {
                    url: url.clone(),
                    message: format!("a redirect to {current} leaves HTTP"),
                });
            }
            let mut request = agent.get(current.as_str());
            let same_origin = options
                .origin
                .as_ref()
                .is_none_or(|origin| *origin == current.origin());
            if same_origin {
                for (name, value) in &options.headers {
                    request = request.header(name, value);
                }
            }
            let mut response = request.call().map_err(|e| failed(e.to_string()))?;
            let status = response.status().as_u16();
            if (300..400).contains(&status) {
                let location = response
                    .headers()
                    .get("location")
                    .and_then(|v| v.to_str().ok())
                    .ok_or_else(|| failed(format!("HTTP {status} without a Location")))?;
                current = current
                    .join(location)
                    .map_err(|e| failed(format!("invalid redirect: {e}")))?;
                continue;
            }
            if status >= 400 {
                return Err(failed(format!("HTTP {status}")));
            }
            return response
                .body_mut()
                .with_config()
                .limit(MAX_BODY)
                .read_to_vec()
                .map_err(|e| failed(e.to_string()));
        }
        Err(FetchError::Http {
            url: url.clone(),
            message: format!("more than {MAX_REDIRECTS} redirects"),
        })
    }
}

/// A Fetcher that returns in-memory documents. For tests and embedding.
#[derive(Debug, Clone, Default)]
pub struct MemoryFetcher {
    pub documents: std::collections::HashMap<Url, Vec<u8>>,
}

impl MemoryFetcher {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn insert(&mut self, url: &str, body: impl Into<Vec<u8>>) -> &mut Self {
        let url = Url::parse(url).expect("valid URL");
        self.documents.insert(url, body.into());
        self
    }
}

impl Fetcher for MemoryFetcher {
    fn fetch(&self, url: &Url) -> Result<Vec<u8>, FetchError> {
        let mut key = url.clone();
        key.set_fragment(None);
        self.documents
            .get(&key)
            .cloned()
            .ok_or_else(|| FetchError::Io {
                url: url.clone(),
                source: std::io::Error::from(std::io::ErrorKind::NotFound),
            })
    }
}
