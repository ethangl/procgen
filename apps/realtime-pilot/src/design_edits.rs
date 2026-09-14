//! Debounced design revisions. Invalid drafts also cancel pending publications.
use procgen_realtime_pilot::{PlanetDesignConfig, PlanetDesignField};
use std::{
    sync::Arc,
    time::{Duration, Instant},
};

pub const EDIT_DELAY: Duration = Duration::from_millis(350);

pub struct DesignEdits {
    pub config: PlanetDesignConfig,
    pub seed_text: String,
    observed: PlanetDesignConfig,
    observed_seed: String,
    changed_at: Instant,
    revision: u64,
    submitted: u64,
    pub validation: Option<String>,
}
impl DesignEdits {
    pub fn new(config: PlanetDesignConfig) -> Self {
        let seed_text = config.seed.to_string();
        Self {
            observed: config.clone(),
            observed_seed: seed_text.clone(),
            config,
            seed_text,
            changed_at: Instant::now(),
            revision: 0,
            submitted: 0,
            validation: None,
        }
    }
    pub fn revision(&self) -> u64 {
        self.revision
    }
    pub fn load(&mut self, config: PlanetDesignConfig) {
        self.seed_text = config.seed.to_string();
        self.config = config;
    }
    pub fn validated(&self) -> Result<PlanetDesignField, String> {
        let mut config = self.config.clone();
        config.seed = self
            .seed_text
            .parse()
            .map_err(|_| "Seed must be an unsigned 64-bit integer.".to_string())?;
        config.validate().map_err(|e| e.to_string())
    }
    pub fn observe(&mut self, now: Instant) -> bool {
        if self.observed == self.config && self.observed_seed == self.seed_text {
            return false;
        }
        self.observed = self.config.clone();
        self.observed_seed = self.seed_text.clone();
        self.changed_at = now;
        self.revision += 1;
        self.validation = self.validated().err();
        true
    }
    pub fn request(&mut self, now: Instant) -> Option<(u64, Arc<PlanetDesignField>)> {
        if self.submitted == self.revision || now.duration_since(self.changed_at) < EDIT_DELAY {
            return None;
        }
        let field = Arc::new(self.validated().ok()?);
        self.submitted = self.revision;
        Some((self.revision, field))
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn rapid_edits_coalesce_and_invalid_drafts_invalidate_in_flight_work() {
        let mut edits = DesignEdits::new(PlanetDesignConfig::starter(42));
        let now = Instant::now();
        edits.config.octaves[0].amplitude_m = 123.0;
        assert!(edits.observe(now));
        assert!(edits.request(now).is_none());
        edits.config.octaves[1].enabled = false;
        edits.observe(now + EDIT_DELAY);
        assert!(edits.request(now + EDIT_DELAY).is_none());
        let (revision, field) = edits.request(now + EDIT_DELAY * 2).unwrap();
        assert_eq!(field.config().octaves[0].amplitude_m, 123.0);
        assert!(!field.config().octaves[1].enabled);
        assert!(edits.request(now + EDIT_DELAY * 3).is_none());
        edits.seed_text = "invalid".into();
        edits.observe(now + EDIT_DELAY * 3);
        assert_ne!(revision, edits.revision());
        assert!(edits.validation.is_some());
        assert!(edits.request(now + EDIT_DELAY * 4).is_none());
        edits.seed_text = "43".into();
        edits.observe(now + EDIT_DELAY * 4);
        let (_, field) = edits.request(now + EDIT_DELAY * 5).unwrap();
        assert_eq!(field.config().seed, 43);
        assert!(!edits.observe(now + EDIT_DELAY * 6));
    }
}
