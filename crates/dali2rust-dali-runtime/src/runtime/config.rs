use dali2rust_domain::dali::ses::RetryPolicy;

pub const DEFAULT_DISCOVERY_STEP_RETRIES: u8 = 2;
pub const DEFAULT_TARGET_STATE_SEQUENCE_RETRIES: u8 = 1;
const DEFAULT_CONTENT_CONFIRM_MAX_SAMPLES: u8 = 5;
const CONTENT_CONFIRM_MIN_SAMPLES: u8 = 2;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ContentConfirmPolicy {
    pub enabled: bool,
    pub max_samples: u8,
}

impl ContentConfirmPolicy {
    pub const fn new(enabled: bool, max_samples: u8) -> Self {
        Self {
            enabled,
            max_samples,
        }
    }

    pub const fn effective_max_samples(&self) -> u8 {
        if self.max_samples < CONTENT_CONFIRM_MIN_SAMPLES {
            CONTENT_CONFIRM_MIN_SAMPLES
        } else {
            self.max_samples
        }
    }
}

impl Default for ContentConfirmPolicy {
    fn default() -> Self {
        Self::new(true, DEFAULT_CONTENT_CONFIRM_MAX_SAMPLES)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DaliRuntimeConfig {
    pub retry: RetryPolicy,
    pub discovery_step_retries: u8,
    pub content_confirm: ContentConfirmPolicy,
    pub target_state_sequence_retries: u8,
}

impl DaliRuntimeConfig {
    pub const fn new(
        retry: RetryPolicy,
        discovery_step_retries: u8,
        content_confirm: ContentConfirmPolicy,
    ) -> Self {
        Self {
            retry,
            discovery_step_retries,
            content_confirm,
            target_state_sequence_retries: DEFAULT_TARGET_STATE_SEQUENCE_RETRIES,
        }
    }

    pub const fn with_target_state_sequence_retries(mut self, retries: u8) -> Self {
        self.target_state_sequence_retries = retries;
        self
    }
}

impl Default for DaliRuntimeConfig {
    fn default() -> Self {
        Self::new(
            RetryPolicy::default(),
            DEFAULT_DISCOVERY_STEP_RETRIES,
            ContentConfirmPolicy::default(),
        )
    }
}
