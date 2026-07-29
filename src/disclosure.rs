use std::time::Duration;

/// Full-range duration for disclosure animations.
///
/// Reversed or interrupted transitions scale this duration by the remaining
/// visual distance, so a nearly completed transition never takes another full
/// 170 ms to settle.
pub const DISCLOSURE_ANIMATION_DURATION: Duration = Duration::from_millis(170);

const SETTLE_EPSILON: f32 = 0.000_1;

/// Pure, clock-independent state for an expandable disclosure.
///
/// The caller advances the animation with frame deltas and uses [`Self::progress`]
/// for both clipped height and opacity. Keeping time outside this type makes the
/// state deterministic in tests and lets the GPUI render loop own frame
/// scheduling.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct DisclosureAnimation {
    progress: f32,
    start: f32,
    target: f32,
    elapsed: Duration,
    duration: Duration,
}

impl DisclosureAnimation {
    /// A disclosure whose content is not mounted.
    pub const fn collapsed() -> Self {
        Self {
            progress: 0.0,
            start: 0.0,
            target: 0.0,
            elapsed: Duration::ZERO,
            duration: Duration::ZERO,
        }
    }

    /// A disclosure that is fully visible.
    #[cfg(test)]
    pub const fn expanded() -> Self {
        Self {
            progress: 1.0,
            start: 1.0,
            target: 1.0,
            elapsed: Duration::ZERO,
            duration: Duration::ZERO,
        }
    }

    /// A newly opening disclosure at zero height and opacity.
    ///
    /// The content is considered mounted immediately, but stays invisible until
    /// the caller supplies a non-zero frame delta.
    #[cfg(test)]
    pub fn opening() -> Self {
        let mut animation = Self::collapsed();
        animation.set_open(true);
        animation
    }

    /// Current eased visual progress in the inclusive range `0.0..=1.0`.
    pub fn progress(&self) -> f32 {
        self.progress
    }

    /// Whether the current target is the expanded state.
    pub fn target_is_open(&self) -> bool {
        self.target == 1.0
    }

    /// Whether another animation frame is required.
    pub fn is_animating(&self) -> bool {
        self.duration != Duration::ZERO
    }

    /// Whether the disclosure has reached its current target.
    pub fn is_settled(&self) -> bool {
        !self.is_animating()
    }

    /// Whether the content must remain mounted.
    ///
    /// An opening disclosure mounts at progress zero to prevent a first-frame
    /// flash. A closing disclosure remains mounted until it reaches zero.
    pub fn should_render_content(&self) -> bool {
        self.target_is_open() || self.progress > 0.0
    }

    /// Duration of the current transition after remaining-distance scaling.
    #[cfg(test)]
    pub fn transition_duration(&self) -> Duration {
        self.duration
    }

    /// Changes the target without changing the current visual position.
    ///
    /// Returns `true` when a new transition was started. Asking for the current
    /// target is a no-op and does not restart its easing curve.
    pub fn set_open(&mut self, open: bool) -> bool {
        let target = if open { 1.0 } else { 0.0 };
        if self.target == target {
            return false;
        }

        self.start = self.progress;
        self.target = target;
        self.elapsed = Duration::ZERO;

        let distance = (self.target - self.start).abs();
        if distance <= SETTLE_EPSILON {
            self.finish();
        } else {
            self.duration = scaled_duration(distance);
        }
        true
    }

    /// Reverses the current target without jumping the visual position.
    pub fn toggle(&mut self) {
        self.set_open(!self.target_is_open());
    }

    /// Advances by a caller-supplied frame delta.
    ///
    /// Returns `true` while another frame is still required. Progress follows a
    /// cubic ease-out curve and is clamped exactly to the target on completion.
    pub fn advance(&mut self, delta: Duration) -> bool {
        if self.is_settled() {
            return false;
        }

        self.elapsed = self.elapsed.saturating_add(delta).min(self.duration);
        let linear =
            (self.elapsed.as_secs_f64() / self.duration.as_secs_f64()).clamp(0.0, 1.0) as f32;
        let eased = ease_out_cubic(linear);
        self.progress = (self.start + (self.target - self.start) * eased).clamp(0.0, 1.0);

        if self.elapsed >= self.duration {
            self.finish();
            false
        } else {
            true
        }
    }

    /// Immediately settles at the current target.
    pub fn finish(&mut self) {
        self.progress = self.target;
        self.start = self.target;
        self.elapsed = Duration::ZERO;
        self.duration = Duration::ZERO;
    }
}

impl Default for DisclosureAnimation {
    fn default() -> Self {
        Self::collapsed()
    }
}

fn scaled_duration(distance: f32) -> Duration {
    DISCLOSURE_ANIMATION_DURATION.mul_f32(distance.clamp(0.0, 1.0))
}

fn ease_out_cubic(value: f32) -> f32 {
    1.0 - (1.0 - value.clamp(0.0, 1.0)).powi(3)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn assert_close(actual: f32, expected: f32) {
        assert!(
            (actual - expected).abs() < 0.000_1,
            "expected {expected}, got {actual}"
        );
    }

    #[test]
    fn constructors_are_settled_at_their_endpoints() {
        let collapsed = DisclosureAnimation::collapsed();
        assert_close(collapsed.progress(), 0.0);
        assert!(!collapsed.target_is_open());
        assert!(collapsed.is_settled());
        assert!(!collapsed.should_render_content());

        let expanded = DisclosureAnimation::expanded();
        assert_close(expanded.progress(), 1.0);
        assert!(expanded.target_is_open());
        assert!(expanded.is_settled());
        assert!(expanded.should_render_content());
    }

    #[test]
    fn full_transition_uses_base_duration_and_cubic_ease_out() {
        let mut animation = DisclosureAnimation::opening();
        assert_eq!(
            animation.transition_duration(),
            DISCLOSURE_ANIMATION_DURATION
        );
        assert!(animation.should_render_content());
        assert_close(animation.progress(), 0.0);

        assert!(animation.advance(Duration::from_millis(85)));
        assert_close(animation.progress(), 0.875);

        assert!(!animation.advance(Duration::from_millis(85)));
        assert_close(animation.progress(), 1.0);
        assert!(animation.is_settled());
    }

    #[test]
    fn reverse_duration_scales_with_remaining_visual_distance() {
        let mut animation = DisclosureAnimation::expanded();
        animation.set_open(false);
        animation.advance(Duration::from_millis(85));
        assert_close(animation.progress(), 0.125);

        let before_reverse = animation.progress();
        animation.set_open(true);
        assert_close(animation.progress(), before_reverse);
        assert_eq!(
            animation.transition_duration(),
            DISCLOSURE_ANIMATION_DURATION.mul_f32(0.875)
        );
    }

    #[test]
    fn repeated_fast_reversals_are_position_continuous() {
        let mut animation = DisclosureAnimation::opening();
        animation.advance(Duration::from_millis(20));
        let opening_progress = animation.progress();

        animation.toggle();
        assert_close(animation.progress(), opening_progress);
        animation.advance(Duration::from_millis(7));
        let closing_progress = animation.progress();
        assert!(closing_progress < opening_progress);

        animation.toggle();
        assert_close(animation.progress(), closing_progress);
        animation.advance(Duration::from_millis(5));
        assert!(animation.progress() > closing_progress);
        assert!((0.0..=1.0).contains(&animation.progress()));
    }

    #[test]
    fn setting_the_existing_target_does_not_restart_transition() {
        let mut animation = DisclosureAnimation::opening();
        animation.advance(Duration::from_millis(25));
        let progress = animation.progress();
        let duration = animation.transition_duration();

        assert!(!animation.set_open(true));
        assert_close(animation.progress(), progress);
        assert_eq!(animation.transition_duration(), duration);
    }

    #[test]
    fn closing_content_stays_mounted_until_zero_then_can_be_removed() {
        let mut animation = DisclosureAnimation::expanded();
        animation.set_open(false);
        animation.advance(Duration::from_millis(100));
        assert!(animation.should_render_content());

        animation.advance(Duration::from_secs(1));
        assert_close(animation.progress(), 0.0);
        assert!(animation.is_settled());
        assert!(!animation.should_render_content());
    }

    #[test]
    fn finish_settles_exactly_at_the_current_target() {
        let mut animation = DisclosureAnimation::opening();
        animation.advance(Duration::from_millis(30));
        animation.finish();
        assert_close(animation.progress(), 1.0);
        assert!(animation.is_settled());

        animation.set_open(false);
        animation.advance(Duration::from_millis(10));
        animation.finish();
        assert_close(animation.progress(), 0.0);
        assert!(animation.is_settled());
        assert!(!animation.should_render_content());
    }
}
