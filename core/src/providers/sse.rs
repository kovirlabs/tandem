//! Split a streaming HTTP body into lines (NDJSON for Ollama, SSE for Anthropic/OpenAI).

use futures::{Stream, StreamExt};

use super::ProviderError;

pub fn lines<S, B>(body: S) -> impl Stream<Item = Result<String, ProviderError>> + Send
where
    S: Stream<Item = Result<B, reqwest::Error>> + Send + 'static,
    B: AsRef<[u8]> + Send,
{
    let state = (Box::pin(body), Vec::<u8>::new(), false);
    futures::stream::unfold(state, |(mut body, mut buf, mut done)| async move {
        loop {
            if let Some(pos) = buf.iter().position(|b| *b == b'\n') {
                let line: Vec<u8> = buf.drain(..=pos).collect();
                let text = String::from_utf8_lossy(&line)
                    .trim_end_matches(['\r', '\n'])
                    .to_string();
                return Some((Ok(text), (body, buf, done)));
            }
            if done {
                if buf.is_empty() {
                    return None;
                }
                let text = String::from_utf8_lossy(&buf).trim_end().to_string();
                buf.clear();
                return Some((Ok(text), (body, buf, done)));
            }
            match body.next().await {
                Some(Ok(chunk)) => buf.extend_from_slice(chunk.as_ref()),
                Some(Err(e)) => {
                    buf.clear();
                    return Some((Err(e.into()), (body, buf, true)));
                }
                None => done = true,
            }
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn splits_across_chunk_boundaries() {
        let chunks: Vec<Result<Vec<u8>, reqwest::Error>> = vec![
            Ok(b"{\"a\":1}\n{\"b\"".to_vec()),
            Ok(b":2}\r\n\n{\"c\":3}".to_vec()),
        ];
        let out: Vec<String> = lines(futures::stream::iter(chunks))
            .map(|r| r.unwrap_or_default())
            .collect()
            .await;
        assert_eq!(out, vec!["{\"a\":1}", "{\"b\":2}", "", "{\"c\":3}"]);
    }
}
