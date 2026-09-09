use crate::editor::Position;

pub(crate) const MIN_ZOOM: f64 = 0.25;
pub(crate) const MAX_ZOOM: f64 = 2.5;

pub(crate) fn clamp_zoom(zoom: f64) -> f64 {
    if zoom.is_finite() {
        zoom.clamp(MIN_ZOOM, MAX_ZOOM)
    } else {
        1.0
    }
}

/// Keep the world point under the cursor stationary when the scale changes.
pub(crate) fn zoom_scroll(scroll: Position, anchor: Position, old: f64, new: f64) -> Position {
    let ratio = clamp_zoom(new) / clamp_zoom(old);
    Position::new(
        (scroll.x + anchor.x) * ratio - anchor.x,
        (scroll.y + anchor.y) * ratio - anchor.y,
    )
}

pub(crate) fn canvas_point(local: Position, scroll: Position, zoom: f64) -> Position {
    let zoom = clamp_zoom(zoom);
    Position {
        x: (local.x + scroll.x) / zoom,
        y: (local.y + scroll.y) / zoom,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn zoom_keeps_the_cursor_over_the_same_world_point() {
        let anchor = Position::new(300.0, 250.0);
        let scroll = Position::new(600.0, 500.0);
        let before = canvas_point(anchor, scroll, 1.0);
        for zoom in [0.5, 1.25, 2.5] {
            let after = canvas_point(anchor, zoom_scroll(scroll, anchor, 1.0, zoom), zoom);
            assert_eq!(before, after);
        }
    }

    #[test]
    fn drag_and_drop_coordinates_are_independent_of_zoom() {
        for zoom in [0.25, 0.5, 1.0, 2.5] {
            let scroll = Position::new(30.0, 50.0);
            let client = Position {
                x: 300.0 * zoom - scroll.x,
                y: 400.0 * zoom - scroll.y,
            };
            assert_eq!(
                canvas_point(client, scroll, zoom),
                Position::new(300.0, 400.0)
            );
            let moved = canvas_point(
                Position {
                    x: client.x + 40.0 * zoom,
                    y: client.y + 20.0 * zoom,
                },
                scroll,
                zoom,
            );
            assert_eq!(moved, Position::new(340.0, 420.0));
        }
    }

    #[test]
    fn zoom_is_bounded_and_clamps_to_the_canvas_edges() {
        assert_eq!(clamp_zoom(0.01), MIN_ZOOM);
        assert_eq!(clamp_zoom(90.0), MAX_ZOOM);
        assert_eq!(clamp_zoom(f64::NAN), 1.0);
        assert_eq!(
            zoom_scroll(Position::default(), Position::new(100.0, 100.0), 1.0, 0.5),
            Position::default()
        );
    }
}
