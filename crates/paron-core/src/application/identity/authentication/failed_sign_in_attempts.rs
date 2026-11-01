use std::time::Duration;

use super::methods::AuthenticationMethod;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ThrottledMethod {
    Password,
    Totp,
    RecoveryCode,
}

impl From<ThrottledMethod> for AuthenticationMethod {
    fn from(method: ThrottledMethod) -> Self {
        match method {
            ThrottledMethod::Password => Self::Password,
            ThrottledMethod::Totp => Self::Totp,
            ThrottledMethod::RecoveryCode => Self::RecoveryCode,
        }
    }
}

impl TryFrom<AuthenticationMethod> for ThrottledMethod {
    type Error = AuthenticationMethod;

    fn try_from(method: AuthenticationMethod) -> Result<Self, Self::Error> {
        match method {
            AuthenticationMethod::Password => Ok(Self::Password),
            AuthenticationMethod::Totp => Ok(Self::Totp),
            AuthenticationMethod::RecoveryCode => Ok(Self::RecoveryCode),
            AuthenticationMethod::Passkey => Err(method),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AttemptKey {
    pub method: ThrottledMethod,
    pub user_id: uuid::Uuid,
    pub device_id: Option<uuid::Uuid>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FailedSignInAttempts {
    pub key: AttemptKey,
    pub count: u32,
    pub next_attempt_at: jiff::Timestamp,
    pub expires_at: jiff::Timestamp,
}

#[derive(Debug, Clone, Copy)]
pub struct FailureDelay {
    pub free_failures: u32,
    pub first_delay: Duration,
    pub max_delay: Duration,
    pub forget_after: Duration,
}

impl FailureDelay {
    pub fn delay(&self, count: u32) -> Duration {
        count
            .checked_sub(self.free_failures)
            .map_or(Duration::ZERO, |delayed_failures| {
                self.first_delay
                    .saturating_mul(2_u32.saturating_pow(delayed_failures))
                    .min(self.max_delay)
            })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const DELAY: FailureDelay = FailureDelay {
        free_failures: 2,
        first_delay: Duration::from_secs(1),
        max_delay: Duration::from_secs(100),
        forget_after: Duration::from_secs(1000),
    };

    #[test]
    fn delay_within_the_free_failures_returns_zero() {
        let delay = DELAY.delay(1);

        assert_eq!(delay, Duration::ZERO);
    }

    #[test]
    fn delay_after_the_free_failures_doubles() {
        let delays: Vec<_> = (2..=4).map(|count| DELAY.delay(count)).collect();

        assert_eq!(
            delays,
            [
                Duration::from_secs(1),
                Duration::from_secs(2),
                Duration::from_secs(4)
            ]
        );
    }

    #[test]
    fn delay_after_many_failures_returns_the_max_delay() {
        let delay = DELAY.delay(50);

        assert_eq!(delay, Duration::from_secs(100));
    }
}
