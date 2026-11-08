//! `SpeechTap`: an `EventSink` that lets the speaker see chat events on their way to the UI.
//! The engine and orchestrator emit through this; the speaker emits through the inner sink
//! (so its own events never loop back into it).

use std::sync::Arc;

use super::Speaker;
use crate::events::{CoreEvent, EventSink};

pub struct SpeechTap {
    inner: Arc<dyn EventSink>,
    speaker: Arc<dyn Speaker>,
}

impl SpeechTap {
    pub fn new(inner: Arc<dyn EventSink>, speaker: Arc<dyn Speaker>) -> Self {
        Self { inner, speaker }
    }
}

impl EventSink for SpeechTap {
    fn emit(&self, event: CoreEvent) {
        if matches!(event, CoreEvent::ChatDelta(_) | CoreEvent::ChatMessage(_)) {
            self.speaker.observe(&event);
        }
        self.inner.emit(event);
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};

    use super::*;
    use crate::conversation::ChatDelta;
    use crate::events::{CollectingSink, CoreNotice};

    #[derive(Default)]
    struct CountingSpeaker(AtomicUsize);
    impl Speaker for CountingSpeaker {
        fn observe(&self, _e: &CoreEvent) {
            self.0.fetch_add(1, Ordering::SeqCst);
        }
        fn arm_next_reply(&self) {}
        fn stop(&self) {}
        fn is_speaking(&self) -> bool {
            false
        }
        fn ready(&self) -> bool {
            false
        }
    }

    #[test]
    fn forwards_everything_and_shows_only_chat_to_speaker() {
        let inner = Arc::new(CollectingSink::default());
        let speaker = Arc::new(CountingSpeaker::default());
        let tap = SpeechTap::new(inner.clone(), speaker.clone());
        tap.emit(CoreEvent::ChatDelta(ChatDelta {
            message_id: "m".into(),
            delta: "Hi.".into(),
        }));
        tap.emit(CoreEvent::Notice(CoreNotice {
            key: "k".into(),
            detail: None,
        }));
        assert_eq!(inner.take().len(), 2);
        assert_eq!(speaker.0.load(Ordering::SeqCst), 1);
    }
}
