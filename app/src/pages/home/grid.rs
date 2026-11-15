use crate::ui::{self, px, units};
use gpui::{Bounds, Pixels, Point, Size, point};

pub const GAP: f32 = 16.;
const ROWS: u8 = 7;
const WIDEST: u8 = 4;
const BREAKPOINTS: [f32; 2] = [1280., 1600.];

pub type Span = (u8, u8);

#[derive(Clone, Copy, PartialEq)]
pub struct Grid {
    pub columns: u8,
    column: f32,
    row: f32,
    scale: f32,
}

impl Grid {
    pub fn new(page: Size<Pixels>, viewport: Pixels, scale: f32) -> Self {
        let wider = |edge: &&f32| units(viewport) >= **edge;
        let columns = 2 + BREAKPOINTS.iter().filter(wider).count() as u8;
        let track = |length: Pixels, count: u8| {
            (units(length) - GAP * f32::from(count - 1)) / f32::from(count)
        };
        Self {
            columns,
            column: track(page.width, columns),
            row: track(page.height, ROWS),
            scale: scale * ui::scale(),
        }
    }

    pub fn unit(&self) -> (f32, f32) {
        (self.column + GAP, self.row + GAP)
    }

    pub fn span(&self, stored: Span) -> Span {
        let known = |value: u8, most: u8| {
            if (1..=most).contains(&value) {
                value
            } else {
                1
            }
        };
        (
            known(stored.0, WIDEST).min(self.columns),
            known(stored.1, ROWS),
        )
    }

    pub fn frames(&self, spans: &[Span]) -> (Vec<Bounds<Pixels>>, Pixels) {
        let snap = |value: f32| px((value * self.scale).round() / self.scale);
        let (across, down) = self.unit();
        let frames: Vec<_> = place(spans, self.columns)
            .into_iter()
            .zip(spans)
            .map(|((column, row), (width, height))| {
                let (left, top) = (column as f32 * across, row as f32 * down);
                Bounds::from_corners(
                    point(snap(left), snap(top)),
                    point(
                        snap(left + f32::from(*width) * across - GAP),
                        snap(top + f32::from(*height) * down - GAP),
                    ),
                )
            })
            .collect();
        let height = frames.iter().map(Bounds::bottom).max().unwrap_or_default();
        (frames, height)
    }
}

fn place(spans: &[Span], columns: u8) -> Vec<(usize, usize)> {
    let mut taken: Vec<[bool; WIDEST as usize]> = Vec::new();
    let (mut column, mut row) = (0, 0);
    spans
        .iter()
        .map(|&(width, height)| {
            let (width, height) = (usize::from(width), usize::from(height));
            loop {
                if column + width > usize::from(columns) {
                    (column, row) = (0, row + 1);
                    continue;
                }
                let free = taken
                    .iter()
                    .skip(row)
                    .take(height)
                    .all(|cells| cells[column..column + width].iter().all(|cell| !cell));
                if free {
                    break;
                }
                column += 1;
            }
            if taken.len() < row + height {
                taken.resize(row + height, [false; WIDEST as usize]);
            }
            for cells in &mut taken[row..row + height] {
                cells[column..column + width].fill(true);
            }
            (column, row)
        })
        .collect()
}

pub fn rank(order: &[String], id: &str) -> usize {
    let place = order.iter().position(|entry| entry == id);
    place.unwrap_or(order.len())
}

fn shifted(order: &[String], from: usize, to: usize) -> Vec<String> {
    let mut next = order.to_vec();
    let id = next.remove(from);
    next.insert(to, id);
    next
}

pub fn dropped(order: &[String], id: &str, onto: &str) -> Option<Vec<String>> {
    let from = order.iter().position(|entry| entry == id)?;
    let to = order.iter().position(|entry| entry == onto)?;
    (from != to).then(|| shifted(order, from, to))
}

pub fn stepped(order: &[String], id: &str, delta: isize) -> Option<Vec<String>> {
    let from = order.iter().position(|entry| entry == id)?;
    let to = from.checked_add_signed(delta)?;
    (to < order.len()).then(|| shifted(order, from, to))
}

pub fn merged(saved: &[String], shown: &[String]) -> Vec<String> {
    let mut queue = shown.iter();
    let mut merged: Vec<String> = saved
        .iter()
        .map(|id| match shown.contains(id) {
            true => queue.next().unwrap_or(id).clone(),
            false => id.clone(),
        })
        .collect();
    merged.extend(queue.cloned());
    merged
}

pub fn nudged(stored: Span, (across, down): (i8, i8), columns: u8) -> Span {
    let shown = stored.0.min(columns);
    let column = shown.saturating_add_signed(across).clamp(1, columns);
    let width = if column == shown { stored.0 } else { column };
    (width, stored.1.saturating_add_signed(down).clamp(1, ROWS))
}

pub struct Resize {
    pub id: String,
    pub span: Span,
    stored: Span,
    shown: u8,
    grab: Point<Pixels>,
    size: Size<Pixels>,
    grid: Grid,
}

impl Resize {
    pub fn new(
        id: String,
        stored: Span,
        grab: Point<Pixels>,
        size: Size<Pixels>,
        grid: Grid,
    ) -> Self {
        Self {
            id,
            span: stored,
            stored,
            shown: grid.span(stored).0,
            grab,
            size,
            grid,
        }
    }

    pub fn pull(&mut self, pointer: Point<Pixels>) -> bool {
        let reach = |length: Pixels, unit: f32, most: u8| {
            (((units(length) + GAP) / unit).round() as u8).clamp(1, most)
        };
        let (across, down) = self.grid.unit();
        let stretched = self.size + Size::from(pointer - self.grab);
        let column = reach(stretched.width, across, self.grid.columns);
        let width = if column == self.shown {
            self.stored.0
        } else {
            column
        };
        let span = (width, reach(stretched.height, down, ROWS));
        let changed = span != self.span;
        self.span = span;
        changed
    }
}

#[cfg(test)]
mod tests {
    use gpui::size;

    use super::*;

    fn ids(names: &[&str]) -> Vec<String> {
        names.iter().map(|name| (*name).to_owned()).collect()
    }

    fn reference() -> Grid {
        Grid::new(size(px(1001.67), px(620.06)), px(1280.), 1.)
    }

    #[test]
    fn the_window_width_sets_the_number_of_columns() {
        let columns = |width: f32| Grid::new(size(px(800.), px(600.)), px(width), 1.).columns;
        assert_eq!(
            [940., 1279., 1280., 1599., 1600., 2560.].map(columns),
            [2, 2, 3, 3, 4, 4]
        );
    }

    #[test]
    fn stored_sizes_are_clamped_to_the_columns_and_unknown_ones_fall_back_to_one() {
        let grid = |width: f32| Grid::new(size(px(800.), px(600.)), px(width), 1.);
        assert_eq!(grid(1600.).span((4, 7)), (4, 7));
        assert_eq!(grid(1280.).span((4, 3)), (3, 3));
        assert_eq!(grid(940.).span((3, 2)), (2, 2));
        assert_eq!(grid(1600.).span((5, 8)), (1, 1));
        assert_eq!(grid(1600.).span((0, 0)), (1, 1));
    }

    #[test]
    fn cards_fill_the_grid_row_by_row_like_the_reference_layout() {
        let spans = [(1, 2), (2, 3), (1, 4), (1, 2), (1, 2), (1, 1)];
        assert_eq!(
            place(&spans, 3),
            [(0, 0), (1, 0), (0, 2), (1, 3), (2, 3), (1, 5)]
        );
        assert_eq!(
            place(&[(1, 2), (1, 2), (1, 2)], 1),
            [(0, 0), (0, 2), (0, 4)]
        );
        assert_eq!(
            place(&[(2, 1), (2, 1), (1, 1)], 3),
            [(0, 0), (0, 1), (2, 1)]
        );
    }

    #[test]
    fn frames_match_the_reference_rectangles() {
        let spans = [(1, 2), (2, 3), (1, 4), (1, 2), (1, 2), (1, 1)];
        let (frames, height) = reference().frames(&spans);
        let reference = [
            (254.36, 75.97, 323.22, 165.72),
            (593.58, 75.97, 662.45, 256.58),
            (254.36, 257.69, 323.22, 347.44),
            (593.58, 348.55, 323.22, 165.72),
            (932.8, 348.55, 323.23, 165.72),
            (593.58, 530.27, 323.22, 74.86),
        ];
        for (frame, (left, top, width, height)) in frames.iter().zip(reference) {
            let near = |have: Pixels, want: f32| (f32::from(have) - want).abs() <= 1.;
            assert!(near(frame.left(), left - 254.36), "{frame:?}");
            assert!(near(frame.top(), top - 75.97), "{frame:?}");
            assert!(near(frame.size.width, width), "{frame:?}");
            assert!(near(frame.size.height, height), "{frame:?}");
        }
        assert_eq!(height, px(529.));
        assert_eq!(frames[1].left() - frames[0].right(), px(GAP));
        assert_eq!(frames[2].top() - frames[0].bottom(), px(GAP));
        assert_eq!(frames[5].top() - frames[3].bottom(), px(GAP));
    }

    #[test]
    fn seven_rows_fill_the_content_height() {
        let (frames, height) = reference().frames(&[(1, 7)]);
        assert_eq!(frames[0].size.height, px(620.));
        assert_eq!(height, px(620.));
    }

    #[test]
    fn saved_ids_come_first_and_the_rest_keeps_its_order() {
        let saved = ids(&["c", "gone", "a"]);
        let mut shown = ids(&["a", "b", "c", "d"]);
        shown.sort_by_key(|id| rank(&saved, id));
        assert_eq!(shown, ["c", "a", "b", "d"]);
    }

    #[test]
    fn a_dragged_card_takes_the_place_of_the_card_under_it() {
        let order = ids(&["a", "b", "c", "d"]);
        assert_eq!(dropped(&order, "a", "c").unwrap(), ["b", "c", "a", "d"]);
        assert_eq!(dropped(&order, "d", "b").unwrap(), ["a", "d", "b", "c"]);
        assert_eq!(dropped(&order, "b", "b"), None);
        assert_eq!(dropped(&order, "b", "gone"), None);
    }

    #[test]
    fn a_card_steps_one_place_and_stops_at_the_ends() {
        let order = ids(&["a", "b", "c"]);
        assert_eq!(stepped(&order, "a", 1).unwrap(), ["b", "a", "c"]);
        assert_eq!(stepped(&order, "c", -1).unwrap(), ["a", "c", "b"]);
        assert_eq!(stepped(&order, "a", -1), None);
        assert_eq!(stepped(&order, "c", 1), None);
        assert_eq!(stepped(&order, "gone", 1), None);
    }

    #[test]
    fn a_new_order_keeps_the_places_of_saved_ids_that_are_not_shown() {
        let saved = ids(&["a", "hidden", "b"]);
        assert_eq!(
            merged(&saved, &ids(&["c", "b", "a"])),
            ["c", "hidden", "b", "a"]
        );
        assert_eq!(merged(&[], &ids(&["b", "a"])), ["b", "a"]);
        assert_eq!(merged(&saved, &[]), ["a", "hidden", "b"]);
    }

    #[test]
    fn arrow_keys_change_a_size_within_the_grid() {
        assert_eq!(nudged((1, 2), (1, 0), 3), (2, 2));
        assert_eq!(nudged((3, 2), (1, 0), 3), (3, 2));
        assert_eq!(nudged((4, 2), (1, 0), 3), (4, 2));
        assert_eq!(nudged((4, 2), (0, 1), 3), (4, 3));
        assert_eq!(nudged((4, 2), (-1, 0), 3), (2, 2));
        assert_eq!(nudged((1, 1), (-1, 0), 3), (1, 1));
        assert_eq!(nudged((1, 1), (0, -1), 3), (1, 1));
        assert_eq!(nudged((1, 7), (0, 1), 3), (1, 7));
        assert_eq!(nudged((2, 3), (0, 1), 3), (2, 4));
    }

    #[test]
    fn the_handle_follows_the_pointer_in_whole_columns_and_rows() {
        let grid = reference();
        let (frames, _) = grid.frames(&[(1, 2)]);
        let grab = point(px(320.), px(160.));
        let mut resize = Resize::new("a".into(), (1, 2), grab, frames[0].size, grid);
        let mut pull = |x: f32, y: f32| {
            resize.pull(grab + point(px(x), px(y)));
            resize.span
        };
        assert_eq!(pull(100., 30.), (1, 2));
        assert_eq!(pull(180., 50.), (2, 3));
        assert_eq!(pull(2000., 2000.), (3, 7));
        assert_eq!(pull(-2000., -2000.), (1, 1));
    }

    #[test]
    fn a_stored_width_wider_than_the_grid_survives_a_change_of_height() {
        let grid = reference();
        let (frames, _) = grid.frames(&[grid.span((4, 2))]);
        let grab = point(px(990.), px(160.));
        let mut resize = Resize::new("a".into(), (4, 2), grab, frames[0].size, grid);
        assert!(resize.pull(grab + point(px(0.), px(95.))));
        assert_eq!(resize.span, (4, 3));
        assert!(!resize.pull(grab + point(px(10.), px(100.))));
        assert!(resize.pull(grab + point(px(-340.), px(95.))));
        assert_eq!(resize.span, (2, 3));
    }
}
