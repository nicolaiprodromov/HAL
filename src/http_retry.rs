//! HTTP client for rig that retries transient xAI failures.
//!
//! An agent turn is a chain of model requests. Without retries, one
//! `503 server is shutting down` (xAI does this during deploys) throws
//! away minutes of Architect work. rig's SSE layer has a retry policy,
//! but its OpenAI-compatible stream parser stops at the first error, so
//! that policy never runs. This wrapper retries below rig instead.
//!
//! Two kinds of failure are retried, with exponential backoff:
//!
//! * the request fails outright: 408, 429, most 5xx, or a connection
//!   error;
//! * the response is `200` but its first SSE event is an error payload
//!   (`{"error":{"message":"Service temporarily unavailable…"}}`). rig
//!   cannot parse that event and skips it, so without this check the
//!   turn would end with no output and no error.
//!
//! An error payload that arrives after content has streamed cannot be
//! retried here (rig has already seen part of the answer). It is turned
//! into a stream error so the turn fails loudly instead of finishing
//! empty.

use std::pin::Pin;
use std::time::Duration;

use bytes::Bytes;
use futures::StreamExt;
use rig::http_client::{
    self, HttpClientExt, LazyBody, MultipartForm, Request, ReqwestClient, Response,
    StreamingResponse,
};
use rig::wasm_compat::{WasmCompatSend, WasmCompatSendStream};

/// Backoff between attempts. Five retries, about 30s in total.
pub const DEFAULT_RETRY_DELAYS: &[Duration] = &[
    Duration::from_secs(1),
    Duration::from_secs(2),
    Duration::from_secs(4),
    Duration::from_secs(8),
    Duration::from_secs(15),
];

#[derive(Clone, Debug)]
pub struct RetryingHttp {
    inner: ReqwestClient,
    delays: &'static [Duration],
}

impl Default for RetryingHttp {
    fn default() -> Self {
        Self::with_delays(DEFAULT_RETRY_DELAYS)
    }
}

impl RetryingHttp {
    pub fn with_delays(delays: &'static [Duration]) -> Self {
        Self {
            inner: ReqwestClient::default(),
            delays,
        }
    }
}

/// Status codes worth another attempt.
fn transient_status(code: u16) -> bool {
    matches!(code, 408 | 425 | 429) || ((500..600).contains(&code) && !matches!(code, 501 | 505))
}

/// Whether a failed request is worth another attempt.
pub fn is_transient(err: &http_client::Error) -> bool {
    use http_client::Error as E;
    match err {
        E::InvalidStatusCode(s) | E::InvalidStatusCodeWithMessage(s, _) => {
            transient_status(s.as_u16())
        }
        // reqwest errors: connect, timeout, reset. A `Builder` error
        // means the request itself is malformed, so retrying is useless.
        E::Instance(e) => !format!("{e:?}").contains("Builder"),
        _ => false,
    }
}

/// If `chunk` carries an SSE `data:` line whose JSON is an error object
/// (and not a normal completion chunk), return its message.
pub fn sse_error_message(chunk: &[u8]) -> Option<String> {
    let text = std::str::from_utf8(chunk).ok()?;
    if !text.contains("\"error\"") {
        return None;
    }
    text.lines().find_map(|line| {
        let data = line.trim().strip_prefix("data:")?.trim();
        let v: serde_json::Value = serde_json::from_str(data).ok()?;
        if v.get("choices").is_some() {
            return None;
        }
        let err = v.get("error")?;
        let msg = err
            .get("message")
            .and_then(|m| m.as_str())
            .map(str::to_string)
            .unwrap_or_else(|| err.to_string());
        Some(msg)
    })
}

fn stream_error(message: String) -> http_client::Error {
    http_client::Error::InvalidStatusCodeWithMessage(http::StatusCode::SERVICE_UNAVAILABLE, message)
}

/// Log and return the delay before the next attempt, or `None` to give up.
fn next_delay(
    delays: &[Duration],
    attempt: usize,
    err: &http_client::Error,
    uri: &http::Uri,
) -> Option<Duration> {
    if !is_transient(err) {
        return None;
    }
    let delay = *delays.get(attempt)?;
    tracing::warn!(
        target: "hal::llm",
        error = %err,
        uri = %uri.path(),
        attempt = attempt + 1,
        max = delays.len(),
        delay_ms = delay.as_millis() as u64,
        "transient model API error; retrying"
    );
    Some(delay)
}

type ByteStream = Pin<Box<dyn WasmCompatSendStream<InnerItem = http_client::Result<Bytes>>>>;

impl HttpClientExt for RetryingHttp {
    fn send<T, U>(
        &self,
        req: Request<T>,
    ) -> impl std::future::Future<Output = http_client::Result<Response<LazyBody<U>>>>
           + WasmCompatSend
           + 'static
    where
        T: Into<Bytes>,
        T: WasmCompatSend,
        U: From<Bytes>,
        U: WasmCompatSend + 'static,
    {
        let inner = self.inner.clone();
        let delays = self.delays;
        let (parts, body) = req.into_parts();
        let body: Bytes = body.into();
        async move {
            let mut attempt = 0;
            loop {
                let req = Request::from_parts(parts.clone(), body.clone());
                match inner.send::<Bytes, U>(req).await {
                    Ok(r) => return Ok(r),
                    Err(e) => match next_delay(delays, attempt, &e, &parts.uri) {
                        Some(d) => tokio::time::sleep(d).await,
                        None => return Err(e),
                    },
                }
                attempt += 1;
            }
        }
    }

    fn send_multipart<U>(
        &self,
        req: Request<MultipartForm>,
    ) -> impl std::future::Future<Output = http_client::Result<Response<LazyBody<U>>>>
           + WasmCompatSend
           + 'static
    where
        U: From<Bytes>,
        U: WasmCompatSend + 'static,
    {
        // Multipart bodies cannot be cloned for a second attempt. HAL
        // does not upload files to the model, so pass these through.
        self.inner.send_multipart(req)
    }

    fn send_streaming<T>(
        &self,
        req: Request<T>,
    ) -> impl std::future::Future<Output = http_client::Result<StreamingResponse>> + WasmCompatSend
    where
        T: Into<Bytes>,
    {
        let inner = self.inner.clone();
        let delays = self.delays;
        let (parts, body) = req.into_parts();
        let body: Bytes = body.into();
        async move {
            let mut attempt = 0;
            loop {
                let req = Request::from_parts(parts.clone(), body.clone());
                let result = match inner.send_streaming(req).await {
                    Ok(resp) => peek_first_chunk(resp).await,
                    Err(e) => Err(e),
                };
                match result {
                    Ok(r) => return Ok(r),
                    Err(e) => match next_delay(delays, attempt, &e, &parts.uri) {
                        Some(d) => tokio::time::sleep(d).await,
                        None => return Err(e),
                    },
                }
                attempt += 1;
            }
        }
    }
}

/// Read the first body chunk. An error payload there becomes a
/// retryable error. Otherwise the chunk is put back in front of the
/// rest of the stream, and later error payloads become stream errors.
async fn peek_first_chunk(resp: StreamingResponse) -> http_client::Result<StreamingResponse> {
    let (parts, mut body) = resp.into_parts();
    let first = body.next().await;
    if let Some(Ok(chunk)) = &first {
        if let Some(msg) = sse_error_message(chunk) {
            return Err(stream_error(msg));
        }
    }
    let rest = body.map(|item| match item {
        Ok(chunk) => match sse_error_message(&chunk) {
            Some(msg) => Err(stream_error(msg)),
            None => Ok(chunk),
        },
        Err(e) => Err(e),
    });
    let stream: ByteStream = Box::pin(futures::stream::iter(first).chain(rest));
    Ok(Response::from_parts(parts, stream))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Read, Write};
    use std::net::TcpListener;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;

    const FAST: &[Duration] = &[Duration::from_millis(10), Duration::from_millis(10)];

    /// Serve `responses` in order, one per connection. Returns the base
    /// URL and a counter of requests served.
    fn serve(responses: Vec<&'static str>) -> (String, Arc<AtomicUsize>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let hits = Arc::new(AtomicUsize::new(0));
        let h = hits.clone();
        std::thread::spawn(move || {
            for resp in responses {
                let Ok((mut sock, _)) = listener.accept() else {
                    return;
                };
                let mut buf = [0u8; 4096];
                let mut seen = Vec::new();
                while !seen.windows(4).any(|w| w == b"\r\n\r\n") {
                    let n = sock.read(&mut buf).unwrap_or(0);
                    if n == 0 {
                        break;
                    }
                    seen.extend_from_slice(&buf[..n]);
                }
                h.fetch_add(1, Ordering::SeqCst);
                let _ = sock.write_all(resp.as_bytes());
            }
        });
        (format!("http://{addr}/v1/chat/completions"), hits)
    }

    const UNAVAILABLE: &str = "HTTP/1.1 503 Service Unavailable\r\nContent-Length: 23\r\nConnection: close\r\n\r\nserver is shutting down";
    const BAD_REQUEST: &str =
        "HTTP/1.1 400 Bad Request\r\nContent-Length: 3\r\nConnection: close\r\n\r\nbad";
    const SSE_OK: &str = "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nContent-Length: 46\r\nConnection: close\r\n\r\ndata: {\"choices\":[{\"delta\":{\"content\":\"hi\"}}]}";
    const SSE_ERROR: &str = "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nContent-Length: 64\r\nConnection: close\r\n\r\ndata: {\"error\":{\"message\":\"Service temporarily unavailable.\"}}\n\n";

    fn request(url: &str) -> Request<Vec<u8>> {
        Request::post(url).body(b"{}".to_vec()).unwrap()
    }

    #[tokio::test]
    async fn streaming_retries_503_then_succeeds() {
        let (url, hits) = serve(vec![UNAVAILABLE, UNAVAILABLE, SSE_OK]);
        let client = RetryingHttp::with_delays(FAST);
        let resp = client.send_streaming(request(&url)).await.unwrap();
        assert_eq!(resp.status(), 200);
        let body: Vec<_> = resp.into_body().collect().await;
        let text: Vec<u8> = body.into_iter().flat_map(|c| c.unwrap().to_vec()).collect();
        assert!(String::from_utf8(text).unwrap().contains("\"hi\""));
        assert_eq!(hits.load(Ordering::SeqCst), 3);
    }

    #[tokio::test]
    async fn streaming_retries_error_payload_on_200() {
        let (url, hits) = serve(vec![SSE_ERROR, SSE_OK]);
        let client = RetryingHttp::with_delays(FAST);
        let resp = client.send_streaming(request(&url)).await.unwrap();
        assert_eq!(resp.status(), 200);
        assert_eq!(hits.load(Ordering::SeqCst), 2);
    }

    #[tokio::test]
    async fn gives_up_after_the_last_delay() {
        let (url, hits) = serve(vec![UNAVAILABLE, UNAVAILABLE, UNAVAILABLE]);
        let client = RetryingHttp::with_delays(FAST);
        let err = client
            .send_streaming(request(&url))
            .await
            .err()
            .expect("expected an error");
        assert!(err.to_string().contains("503"), "{err}");
        assert_eq!(hits.load(Ordering::SeqCst), 3);
    }

    #[tokio::test]
    async fn client_errors_are_not_retried() {
        let (url, hits) = serve(vec![BAD_REQUEST, SSE_OK]);
        let client = RetryingHttp::with_delays(FAST);
        let err = client
            .send_streaming(request(&url))
            .await
            .err()
            .expect("expected an error");
        assert!(err.to_string().contains("400"), "{err}");
        assert_eq!(hits.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn plain_send_retries_503() {
        let (url, hits) = serve(vec![UNAVAILABLE, BAD_REQUEST]);
        let client = RetryingHttp::with_delays(FAST);
        let err = client
            .send::<Vec<u8>, Bytes>(request(&url))
            .await
            .err()
            .expect("expected an error");
        assert!(err.to_string().contains("400"), "{err}");
        assert_eq!(hits.load(Ordering::SeqCst), 2);
    }

    #[test]
    fn detects_sse_error_payloads() {
        let err = b"data: {\"error\":{\"message\":\"Service temporarily unavailable.\",\"code\":\"unavailable\"}}\n\n";
        assert_eq!(
            sse_error_message(err).as_deref(),
            Some("Service temporarily unavailable.")
        );
        let normal =
            b"data: {\"choices\":[{\"delta\":{\"content\":\"the \\\"error\\\" key\"}}]}\n\n";
        assert_eq!(sse_error_message(normal), None);
        assert_eq!(sse_error_message(b"data: [DONE]\n\n"), None);
    }

    #[test]
    fn transient_status_codes() {
        for c in [408, 429, 500, 502, 503, 504, 529] {
            assert!(transient_status(c), "{c}");
        }
        for c in [400, 401, 403, 404, 422, 501, 505] {
            assert!(!transient_status(c), "{c}");
        }
    }
}
