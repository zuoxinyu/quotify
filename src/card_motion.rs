use std::time::{Duration, Instant};

use gpui::{
    AnyElement, App, Bounds, Element, ElementId, GlobalElementId, InspectorElementId, IntoElement,
    LayoutId, Pixels, SharedString, Window, point, px,
};

/// Duration of a provider card's FLIP transition after its logical slot changes.
pub const CARD_REORDER_ANIMATION_DURATION: Duration = Duration::from_millis(160);

const SETTLE_EPSILON: f32 = 0.01;

/// Window-space geometry used to pick a logical drop slot independently of the
/// cards' temporary animated offsets.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CardSlot {
    pub left: f32,
    pub right: f32,
    pub center_y: f32,
}

impl CardSlot {
    pub fn from_bounds(bounds: Bounds<Pixels>) -> Self {
        Self {
            left: bounds.left() / px(1.0),
            right: bounds.right() / px(1.0),
            center_y: bounds.center().y / px(1.0),
        }
    }
}

/// Returns the nearest logical slot while the pointer remains within the card
/// column. This avoids animated hitboxes causing a reorder to immediately
/// reverse while two cards cross each other.
pub fn drop_target_index(slots: &[CardSlot], pointer_x: f32, pointer_y: f32) -> Option<usize> {
    let first = slots.first()?;
    let (left, right) = slots.iter().fold((first.left, first.right), |edges, slot| {
        (edges.0.min(slot.left), edges.1.max(slot.right))
    });

    if pointer_x < left || pointer_x > right {
        return None;
    }

    slots
        .iter()
        .enumerate()
        .min_by(|(_, a), (_, b)| {
            (a.center_y - pointer_y)
                .abs()
                .total_cmp(&(b.center_y - pointer_y).abs())
        })
        .map(|(index, _)| index)
}

#[derive(Clone, Copy, Debug, PartialEq)]
struct OffsetAnimation {
    offset_y: f32,
    start_offset_y: f32,
    elapsed: Duration,
    duration: Duration,
}

impl Default for OffsetAnimation {
    fn default() -> Self {
        Self {
            offset_y: 0.0,
            start_offset_y: 0.0,
            elapsed: Duration::ZERO,
            duration: Duration::ZERO,
        }
    }
}

impl OffsetAnimation {
    fn offset_y(&self) -> f32 {
        self.offset_y
    }

    fn is_animating(&self) -> bool {
        self.duration != Duration::ZERO
    }

    fn retarget(&mut self, offset_y: f32, animations_enabled: bool) {
        if !animations_enabled || offset_y.abs() <= SETTLE_EPSILON {
            self.finish();
            return;
        }

        self.offset_y = offset_y;
        self.start_offset_y = offset_y;
        self.elapsed = Duration::ZERO;
        self.duration = CARD_REORDER_ANIMATION_DURATION;
    }

    fn advance(&mut self, delta: Duration) -> bool {
        if !self.is_animating() {
            return false;
        }

        self.elapsed = self.elapsed.saturating_add(delta).min(self.duration);
        let linear =
            (self.elapsed.as_secs_f64() / self.duration.as_secs_f64()).clamp(0.0, 1.0) as f32;
        self.offset_y = self.start_offset_y * (1.0 - ease_out_quint(linear));

        if self.elapsed >= self.duration || self.offset_y.abs() <= SETTLE_EPSILON {
            self.finish();
            false
        } else {
            true
        }
    }

    fn finish(&mut self) {
        self.offset_y = 0.0;
        self.start_offset_y = 0.0;
        self.elapsed = Duration::ZERO;
        self.duration = Duration::ZERO;
    }
}

/// Clock-independent FLIP state for one provider card.
///
/// `natural_y` is the card's current layout position. When the order revision
/// changes, the current visual position is preserved and becomes the starting
/// point for a transition to the new natural position.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
struct CardLayoutMotion {
    natural_y: Option<f32>,
    order_revision: u64,
    animation: OffsetAnimation,
}

impl CardLayoutMotion {
    fn advance(&mut self, delta: Duration) -> bool {
        self.animation.advance(delta)
    }

    fn sync_layout(&mut self, natural_y: f32, order_revision: u64, animations_enabled: bool) {
        if !animations_enabled {
            self.animation.finish();
        }

        if let Some(previous_natural_y) = self.natural_y {
            if order_revision != self.order_revision {
                let previous_visual_y = previous_natural_y + self.animation.offset_y();
                self.animation
                    .retarget(previous_visual_y - natural_y, animations_enabled);
            }
        } else {
            self.animation.finish();
        }

        self.natural_y = Some(natural_y);
        self.order_revision = order_revision;
    }

    fn offset_y(&self) -> f32 {
        self.animation.offset_y()
    }

    fn is_animating(&self) -> bool {
        self.animation.is_animating()
    }
}

struct CardMotionElementState {
    layout: CardLayoutMotion,
    frame_at: Instant,
}

/// An element wrapper that applies a paint/hitbox offset without changing the
/// card's natural flex layout. Each provider must use a stable, unique ID.
pub struct CardMotionElement {
    id: ElementId,
    order_revision: u64,
    animations_enabled: bool,
    child: AnyElement,
}

impl CardMotionElement {
    pub fn new(
        provider: impl Into<SharedString>,
        order_revision: u64,
        animations_enabled: bool,
        child: impl IntoElement,
    ) -> Self {
        Self {
            id: ElementId::Name(format!("provider-card-motion-{}", provider.into()).into()),
            order_revision,
            animations_enabled,
            child: child.into_any_element(),
        }
    }
}

impl IntoElement for CardMotionElement {
    type Element = Self;

    fn into_element(self) -> Self::Element {
        self
    }
}

impl Element for CardMotionElement {
    type RequestLayoutState = ();
    type PrepaintState = ();

    fn id(&self) -> Option<ElementId> {
        Some(self.id.clone())
    }

    fn source_location(&self) -> Option<&'static core::panic::Location<'static>> {
        None
    }

    fn request_layout(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (LayoutId, Self::RequestLayoutState) {
        (self.child.request_layout(window, cx), ())
    }

    fn prepaint(
        &mut self,
        global_id: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        _: &mut Self::RequestLayoutState,
        window: &mut Window,
        cx: &mut App,
    ) -> Self::PrepaintState {
        let Some(global_id) = global_id else {
            self.child.prepaint(window, cx);
            return;
        };

        let now = Instant::now();
        let natural_y = bounds.origin.y / px(1.0);
        let order_revision = self.order_revision;
        let animations_enabled = self.animations_enabled;
        let offset_y = window.with_element_state(
            global_id,
            |state: Option<CardMotionElementState>, window| {
                let mut state = state.unwrap_or_else(|| CardMotionElementState {
                    layout: CardLayoutMotion::default(),
                    frame_at: now,
                });
                let delta = now.saturating_duration_since(state.frame_at);
                state.frame_at = now;
                state.layout.advance(delta);
                state
                    .layout
                    .sync_layout(natural_y, order_revision, animations_enabled);

                if state.layout.is_animating() {
                    window.request_animation_frame();
                }

                (state.layout.offset_y(), state)
            },
        );

        // Snap every intermediate offset to a physical pixel so animated text
        // stays crisp on fractional Windows scale factors.
        let scale_factor = window.scale_factor().max(1.0);
        let snapped_offset_y = (offset_y * scale_factor).round() / scale_factor;
        window.with_element_offset(point(px(0.0), px(snapped_offset_y)), |window| {
            self.child.prepaint(window, cx);
        });
    }

    fn paint(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        _: Bounds<Pixels>,
        _: &mut Self::RequestLayoutState,
        _: &mut Self::PrepaintState,
        window: &mut Window,
        cx: &mut App,
    ) {
        self.child.paint(window, cx);
    }
}

fn ease_out_quint(value: f32) -> f32 {
    1.0 - (1.0 - value.clamp(0.0, 1.0)).powi(5)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn assert_close(actual: f32, expected: f32) {
        assert!(
            (actual - expected).abs() < 0.001,
            "expected {expected}, got {actual}"
        );
    }

    #[test]
    fn first_layout_does_not_animate() {
        let mut motion = CardLayoutMotion::default();
        motion.sync_layout(120.0, 7, true);

        assert_close(motion.offset_y(), 0.0);
        assert!(!motion.is_animating());
    }

    #[test]
    fn reorder_preserves_visual_position_then_settles() {
        let mut motion = CardLayoutMotion::default();
        motion.sync_layout(20.0, 0, true);
        motion.sync_layout(140.0, 1, true);

        assert_close(140.0 + motion.offset_y(), 20.0);
        assert!(motion.is_animating());

        motion.advance(CARD_REORDER_ANIMATION_DURATION);
        assert_close(motion.offset_y(), 0.0);
        assert!(!motion.is_animating());
    }

    #[test]
    fn interrupted_reorder_is_position_continuous() {
        let mut motion = CardLayoutMotion::default();
        motion.sync_layout(0.0, 0, true);
        motion.sync_layout(100.0, 1, true);
        motion.advance(Duration::from_millis(60));
        let visual_before = 100.0 + motion.offset_y();

        motion.sync_layout(0.0, 2, true);
        let visual_after = motion.offset_y();

        assert_close(visual_after, visual_before);
        assert!(motion.is_animating());
    }

    #[test]
    fn equal_elapsed_time_is_frame_rate_independent() {
        let mut one_step = CardLayoutMotion::default();
        one_step.sync_layout(0.0, 0, true);
        one_step.sync_layout(100.0, 1, true);
        one_step.advance(Duration::from_millis(96));

        let mut several_steps = CardLayoutMotion::default();
        several_steps.sync_layout(0.0, 0, true);
        several_steps.sync_layout(100.0, 1, true);
        for _ in 0..6 {
            several_steps.advance(Duration::from_millis(16));
        }

        assert_close(one_step.offset_y(), several_steps.offset_y());
    }

    #[test]
    fn layout_changes_without_reorder_do_not_restart_motion() {
        let mut motion = CardLayoutMotion::default();
        motion.sync_layout(0.0, 0, true);
        motion.sync_layout(100.0, 1, true);
        motion.advance(Duration::from_millis(40));
        let offset = motion.offset_y();

        motion.sync_layout(84.0, 1, true);

        assert_close(motion.offset_y(), offset);
    }

    #[test]
    fn disabled_client_animations_settle_immediately() {
        let mut motion = CardLayoutMotion::default();
        motion.sync_layout(0.0, 0, true);
        motion.sync_layout(100.0, 1, false);

        assert_close(motion.offset_y(), 0.0);
        assert!(!motion.is_animating());
    }

    #[test]
    fn drop_target_uses_logical_slot_centers_and_column_width() {
        let slots = [
            CardSlot {
                left: 20.0,
                right: 380.0,
                center_y: 100.0,
            },
            CardSlot {
                left: 20.0,
                right: 380.0,
                center_y: 260.0,
            },
            CardSlot {
                left: 20.0,
                right: 380.0,
                center_y: 520.0,
            },
        ];

        assert_eq!(drop_target_index(&slots, 200.0, 210.0), Some(1));
        assert_eq!(drop_target_index(&slots, 200.0, 430.0), Some(2));
        assert_eq!(drop_target_index(&slots, 10.0, 260.0), None);
        assert_eq!(drop_target_index(&[], 200.0, 100.0), None);
    }
}
