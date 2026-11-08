//! Privacy enforcement (SPEC §6.3, CLAUDE.md rule 2).
//!
//! This is the single gate that decides whether a request may go to a provider.
//! The UI is never the only gate. Keep 100% branch coverage on this module.

use thiserror::Error;

use crate::providers::{Locality, Modality};

/// Per-session opt-ins. Both default to off.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Consent {
    /// The user turned on "cloud vision" for this session.
    pub cloud_vision: bool,
    /// The user turned on cloud speech-to-text.
    pub cloud_audio: bool,
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum PrivacyError {
    #[error("camera images can only go to a local provider unless cloud vision is turned on for this session")]
    CloudVisionNotAllowed,
    #[error("microphone audio can only go to a local provider unless cloud speech is turned on")]
    CloudAudioNotAllowed,
}

/// Whether a provider at this locality is treated as on-device/on-network the user controls.
pub fn is_trusted_local(locality: Locality) -> bool {
    match locality {
        Locality::Local => true,
        Locality::Lan { confirmed } => confirmed,
        Locality::Cloud => false,
    }
}

/// Decide whether a request of `modality` may be sent to a provider at `locality`.
pub fn check(modality: Modality, locality: Locality, consent: Consent) -> Result<(), PrivacyError> {
    if is_trusted_local(locality) {
        return Ok(());
    }
    match modality {
        Modality::Text => Ok(()),
        Modality::Image if consent.cloud_vision => Ok(()),
        Modality::Image => Err(PrivacyError::CloudVisionNotAllowed),
        Modality::Audio if consent.cloud_audio => Ok(()),
        Modality::Audio => Err(PrivacyError::CloudAudioNotAllowed),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const LOCAL: Locality = Locality::Local;
    const LAN_OK: Locality = Locality::Lan { confirmed: true };
    const LAN_UNCONFIRMED: Locality = Locality::Lan { confirmed: false };
    const CLOUD: Locality = Locality::Cloud;

    fn consent(cloud_vision: bool, cloud_audio: bool) -> Consent {
        Consent {
            cloud_vision,
            cloud_audio,
        }
    }

    #[test]
    fn trusted_local_classification() {
        assert!(is_trusted_local(LOCAL));
        assert!(is_trusted_local(LAN_OK));
        assert!(!is_trusted_local(LAN_UNCONFIRMED));
        assert!(!is_trusted_local(CLOUD));
    }

    #[test]
    fn everything_allowed_to_trusted_local_without_consent() {
        for loc in [LOCAL, LAN_OK] {
            for m in [Modality::Text, Modality::Image, Modality::Audio] {
                assert_eq!(check(m, loc, Consent::default()), Ok(()), "{m:?} {loc:?}");
            }
        }
    }

    #[test]
    fn text_always_allowed_to_remote() {
        for loc in [LAN_UNCONFIRMED, CLOUD] {
            assert_eq!(check(Modality::Text, loc, Consent::default()), Ok(()));
        }
    }

    #[test]
    fn images_blocked_to_remote_without_cloud_vision() {
        for loc in [LAN_UNCONFIRMED, CLOUD] {
            assert_eq!(
                check(Modality::Image, loc, Consent::default()),
                Err(PrivacyError::CloudVisionNotAllowed)
            );
            // Audio consent does not unlock images.
            assert_eq!(
                check(Modality::Image, loc, consent(false, true)),
                Err(PrivacyError::CloudVisionNotAllowed)
            );
        }
    }

    #[test]
    fn images_allowed_to_remote_with_cloud_vision() {
        for loc in [LAN_UNCONFIRMED, CLOUD] {
            assert_eq!(check(Modality::Image, loc, consent(true, false)), Ok(()));
        }
    }

    #[test]
    fn audio_blocked_to_remote_without_cloud_audio() {
        for loc in [LAN_UNCONFIRMED, CLOUD] {
            assert_eq!(
                check(Modality::Audio, loc, Consent::default()),
                Err(PrivacyError::CloudAudioNotAllowed)
            );
            // Vision consent does not unlock audio.
            assert_eq!(
                check(Modality::Audio, loc, consent(true, false)),
                Err(PrivacyError::CloudAudioNotAllowed)
            );
        }
    }

    #[test]
    fn audio_allowed_to_remote_with_cloud_audio() {
        for loc in [LAN_UNCONFIRMED, CLOUD] {
            assert_eq!(check(Modality::Audio, loc, consent(false, true)), Ok(()));
        }
    }
}
