use thiserror::Error;

#[derive(Debug, Error)]
pub enum M17Codec2Error {
    #[error("selected card '{0}' does not exist or is in use")]
    CardUnavailable(String),

    #[error("default output card is unavailable")]
    DefaultCardUnavailable,

    #[error("selected card '{0}' failed to list available output configs: '{1}'")]
    OutputConfigsUnavailable(String, #[source] cpal::SupportedStreamConfigsError),

    #[error("selected card '{0}' did not offer a compatible output config type, either due to hardware limitations or because it is currently in use")]
    SupportedOutputUnavailable(String),

    #[error("selected card '{0}' was unable to build an output stream: '{1}'")]
    OutputStreamBuildError(String, #[source] cpal::BuildStreamError),

    #[error("selected card '{0}' was unable to play an output stream: '{1}'")]
    OutputStreamPlayError(String, #[source] cpal::PlayStreamError),
}
