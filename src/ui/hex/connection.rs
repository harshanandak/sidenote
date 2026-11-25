use super::{EngineState, HexError, Instant, Recording, Tx};
use std::{cell::Cell, time::Duration};

// 10 s covers the lead's 2.694 s CLI delay and several 1.5 s + 750 ms status polls.
const STALE_LIMIT: Duration = Duration::from_secs(10);
pub(super) const UNKNOWN: &str =
    "Hex recording status unknown. Try New meeting to check Hex again.";
pub(super) const SLOW: &str = "Hex is slow to answer, trying again";

#[derive(Default)]
pub(in crate::ui) struct Connection {
    good: Option<EngineState>,
    good_at: Option<Instant>,
    slow_at: Option<Instant>,
    expiry_shown: bool,
    sampled_at: Option<Instant>,
    phase: &'static str,
    blocked: Cell<Option<&'static str>>,
    action: Option<u64>,
    recovering: bool,
    recovery_delay: Duration,
    recovery_at: Option<Instant>,
}

impl Connection {
    pub(crate) fn begin_action(&mut self) -> bool {
        static SEQUENCE: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
        let sequence = SEQUENCE.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        Self::log_action(sequence, "entry");
        if self.action.is_some() {
            Self::log_action(sequence, "completed");
            return false;
        }
        self.action = Some(sequence);
        true
    }
    fn log_action(sequence: u64, stage: &'static str) {
        sidenote::listener::log_process(&format!(
            "hex new_meeting action seq={sequence} stage={stage}"
        ));
    }
    pub(crate) fn action_stage(&self, stage: &'static str) {
        if let Some(sequence) = self.action {
            Self::log_action(sequence, stage);
        }
    }
    pub(crate) fn finish_action(&mut self) {
        self.action_stage("completed");
        self.action = None;
    }

    fn age(&self) -> Duration {
        self.good_at.map_or(Duration::ZERO, |at| {
            at.elapsed()
                .max(self.sampled_at.unwrap_or(at).saturating_duration_since(at))
        })
    }

    fn limit_age(&self) -> Option<Duration> {
        self.good_at.or(self.slow_at).map(|at| {
            at.elapsed()
                .max(self.sampled_at.unwrap_or(at).saturating_duration_since(at))
        })
    }
    pub(super) fn expired(&self) -> bool {
        self.limit_age().is_some_and(|age| age > STALE_LIMIT)
    }
    pub(super) fn begin_recovery(&mut self, now: Instant, tx: &mut Tx) -> bool {
        if self.recovering {
            return false;
        }
        if let Some(at) = self.recovery_at.filter(|&at| at > now) {
            tx.wake_after(at - now);
            return false;
        }
        self.recovering = true;
        true
    }

    pub(super) fn finish_recovery(&mut self, now: Instant) -> Duration {
        self.recovering = false;
        self.recovery_delay =
            (self.recovery_delay * 2).clamp(Duration::from_secs(1), Duration::from_secs(16));
        self.recovery_at = Some(now + self.recovery_delay);
        self.recovery_delay
    }

    pub(super) fn reset_recovery_backoff(&mut self) {
        self.recovery_at = None;
        self.recovery_delay = Duration::ZERO;
    }

    pub(super) fn tick(&mut self, tx: &mut Tx) -> bool {
        let expired = self.expired();
        if !expired && let Some(age) = self.limit_age() {
            tx.wake_after(STALE_LIMIT.saturating_sub(age) + Duration::from_millis(1));
        }
        let changed = self.expiry_shown != expired;
        self.expiry_shown = expired;
        changed
    }
    pub(super) fn state(&self) -> Option<&EngineState> {
        (self.phase == "stale" && self.age() <= STALE_LIMIT)
            .then_some(self.good.as_ref())
            .flatten()
    }

    // An expired observation cannot confirm availability, but must leave Stop accessible.
    pub(super) fn recording(&self) -> Option<&Recording> {
        self.good
            .as_ref()
            .and_then(|state| state.recording.as_ref())
    }

    pub(super) fn slow(&self, status: &Option<Result<EngineState, HexError>>) -> bool {
        matches!(
            status,
            Some(Err(HexError::Timeout | HexError::Failed { .. }))
        )
    }

    pub(super) fn message(&self) -> String {
        if self.expired() {
            return UNKNOWN.into();
        }
        match self.good_at {
            Some(_) => format!("{SLOW} (last answer {} s ago)", self.age().as_secs()),
            None => SLOW.into(),
        }
    }

    pub(super) fn apply(
        &mut self,
        previous: &Option<Result<EngineState, HexError>>,
        result: &Result<EngineState, HexError>,
        now: Instant,
    ) {
        let interval = self
            .sampled_at
            .map_or(Duration::ZERO, |at| now.saturating_duration_since(at));
        self.sampled_at = Some(now);
        if matches!(result, Err(HexError::Timeout | HexError::Failed { .. })) {
            self.slow_at.get_or_insert(now);
        }
        let (phase, reason) = match result {
            Ok(_) => ("ok", "answer"),
            Err(HexError::HexNotRunning) => ("not_running", "not_running"),
            Err(HexError::Unsupported) => ("unavailable", "unsupported"),
            Err(error) => {
                if let Some(Ok(state)) = previous {
                    self.good = Some(state.clone());
                }
                (
                    if self.good_at.is_some() && self.age() > STALE_LIMIT {
                        "expired"
                    } else {
                        "stale"
                    },
                    if matches!(error, HexError::Timeout) {
                        "timeout"
                    } else {
                        "transport"
                    },
                )
            }
        };
        if phase != self.phase {
            sidenote::listener::log(&format!(
                "hex connection {} -> {phase} reason={reason} last_ok_age_ms={} sample_interval_ms={}",
                self.phase,
                self.age().as_millis(),
                interval.as_millis()
            ));
        }
        self.phase = phase;
        match result {
            Ok(_) => {
                self.good = None;
                self.good_at = Some(now);
                self.slow_at = None;
            }
            Err(HexError::HexNotRunning | HexError::Unsupported) => {
                self.good = None;
                self.good_at = None;
                self.slow_at = None;
            }
            Err(_) => {}
        }
    }

    pub(super) fn log_blocked(&self, reason: Option<&'static str>) {
        if self.blocked.replace(reason) != reason
            && let Some(reason) = reason
        {
            sidenote::listener::log_process(&format!(
                "hex new_meeting availability reason={reason} last_ok_age_ms={}",
                self.age().as_millis()
            ));
        }
    }
}

#[cfg(test)]
mod tests;

#[cfg(test)]
mod stop_tests;
