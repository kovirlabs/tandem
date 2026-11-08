//! Provider router: picks a provider per modality and enforces the privacy policy
//! before any request leaves the process.

pub mod locality;
pub mod privacy;

use std::sync::{Arc, RwLock};

use crate::error::{CoreError, Result};
use crate::providers::{ChatProvider, ChatRequest, ChatStream, Modality};
use privacy::Consent;

#[derive(Default)]
pub struct Router {
    chat: RwLock<Option<Arc<dyn ChatProvider>>>,
    vision: RwLock<Option<Arc<dyn ChatProvider>>>,
}

impl Router {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn set_chat(&self, provider: Option<Arc<dyn ChatProvider>>) {
        if let Ok(mut g) = self.chat.write() {
            *g = provider;
        }
    }

    pub fn set_vision(&self, provider: Option<Arc<dyn ChatProvider>>) {
        if let Ok(mut g) = self.vision.write() {
            *g = provider;
        }
    }

    pub fn chat_provider(&self) -> Option<Arc<dyn ChatProvider>> {
        self.chat.read().ok().and_then(|g| g.clone())
    }

    pub fn vision_provider(&self) -> Option<Arc<dyn ChatProvider>> {
        self.vision.read().ok().and_then(|g| g.clone())
    }

    /// The only way the core sends chat/vision requests. Image requests go to the vision
    /// provider and are checked against `consent` here, never only in the UI.
    pub async fn chat_stream(&self, req: ChatRequest, consent: Consent) -> Result<ChatStream> {
        let modality = req.modality();
        let provider = match modality {
            Modality::Image => self
                .vision_provider()
                .ok_or(CoreError::NoProvider("vision"))?,
            Modality::Text | Modality::Audio => {
                self.chat_provider().ok_or(CoreError::NoProvider("chat"))?
            }
        };
        privacy::check(modality, provider.locality(), consent)?;
        Ok(provider.chat_stream(req).await?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::providers::mock::MockProvider;
    use crate::providers::{ChatMessage, ImageData, Locality};
    use crate::router::privacy::PrivacyError;

    fn image_request() -> ChatRequest {
        let mut msg = ChatMessage::user("look");
        msg.images.push(ImageData {
            jpeg: vec![1, 2, 3],
        });
        ChatRequest {
            messages: vec![msg],
            ..Default::default()
        }
    }

    #[tokio::test]
    async fn router_blocks_cloud_images_and_never_calls_provider() {
        let router = Router::new();
        let cloud = Arc::new(MockProvider::new(Locality::Cloud).with_text_reply("{}"));
        router.set_vision(Some(cloud.clone()));
        let err = router
            .chat_stream(image_request(), Consent::default())
            .await
            .err();
        assert!(matches!(
            err,
            Some(CoreError::Privacy(PrivacyError::CloudVisionNotAllowed))
        ));
        assert_eq!(
            cloud.request_count(),
            0,
            "blocked request must not reach the provider"
        );
    }

    #[tokio::test]
    async fn router_allows_cloud_images_with_consent() {
        let router = Router::new();
        let cloud = Arc::new(MockProvider::new(Locality::Cloud).with_text_reply("{}"));
        router.set_vision(Some(cloud.clone()));
        let consent = Consent {
            cloud_vision: true,
            cloud_audio: false,
        };
        assert!(router.chat_stream(image_request(), consent).await.is_ok());
        assert_eq!(cloud.request_count(), 1);
    }

    #[tokio::test]
    async fn router_routes_images_to_vision_and_text_to_chat() {
        let router = Router::new();
        let chat = Arc::new(MockProvider::new(Locality::Cloud).with_text_reply("hi"));
        let vision = Arc::new(MockProvider::new(Locality::Local).with_text_reply("{}"));
        router.set_chat(Some(chat.clone()));
        router.set_vision(Some(vision.clone()));
        let text = ChatRequest {
            messages: vec![ChatMessage::user("hello")],
            ..Default::default()
        };
        assert!(router.chat_stream(text, Consent::default()).await.is_ok());
        assert!(router
            .chat_stream(image_request(), Consent::default())
            .await
            .is_ok());
        assert_eq!(chat.request_count(), 1);
        assert_eq!(vision.request_count(), 1);
    }

    #[tokio::test]
    async fn router_errors_without_provider() {
        let router = Router::new();
        assert!(matches!(
            router
                .chat_stream(image_request(), Consent::default())
                .await
                .err(),
            Some(CoreError::NoProvider("vision"))
        ));
        assert!(matches!(
            router
                .chat_stream(ChatRequest::default(), Consent::default())
                .await
                .err(),
            Some(CoreError::NoProvider("chat"))
        ));
    }
}
