//! Draws the neighbours of one table as boxes joined by lines: the tables that
//! point at it on the left, the tables it points at on the right. The result is
//! a grid of characters with a paint for each, so `ui` only has to colour it.
//! Pure logic.

/// What a cell is, so the caller can choose its colour.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Paint {
    Blank,
    /// Connectors between boxes.
    Line,
    /// A box that is not selected.
    Box,
    /// The selected box.
    Selected,
    /// The table in the middle.
    Center,
    /// Secondary text: the columns of a key.
    Note,
}

/// A neighbour: its name and the columns of the key that links it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Node {
    pub title: String,
    pub note: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Side {
    Left,
    Right,
}

/// One box on a side, counted from the top of that side's full list.
pub type Pick = (Side, usize);

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Canvas {
    pub width: usize,
    pub height: usize,
    pub cells: Vec<Vec<(char, Paint)>>,
}

const UP: u8 = 1;
const DOWN: u8 = 2;
const LEFT: u8 = 4;
const RIGHT: u8 = 8;

/// Rows one neighbour takes: three for its box and one empty.
const PITCH: usize = 4;
const BOX_HEIGHT: usize = 3;

fn joint(bits: u8) -> char {
    match bits {
        0 => ' ',
        b if b & (UP | DOWN) == 0 => '─',
        b if b & (LEFT | RIGHT) == 0 => '│',
        b if b == UP | RIGHT => '└',
        b if b == UP | LEFT => '┘',
        b if b == DOWN | RIGHT => '┌',
        b if b == DOWN | LEFT => '┐',
        b if b == UP | DOWN | RIGHT => '├',
        b if b == UP | DOWN | LEFT => '┤',
        b if b == LEFT | RIGHT | DOWN => '┬',
        b if b == LEFT | RIGHT | UP => '┴',
        _ => '┼',
    }
}

/// `text` cut to `width` characters, with an ellipsis when it was longer.
fn cut(text: &str, width: usize) -> String {
    let count = text.chars().count();
    if count <= width {
        text.to_string()
    } else {
        let mut out: String = text.chars().take(width.saturating_sub(1)).collect();
        out.push('…');
        out
    }
}

impl Canvas {
    fn new(width: usize, height: usize) -> Self {
        Self {
            width,
            height,
            cells: vec![vec![(' ', Paint::Blank); width]; height],
        }
    }

    fn put(&mut self, x: usize, y: usize, ch: char, paint: Paint) {
        if let Some(cell) = self.cells.get_mut(y).and_then(|row| row.get_mut(x)) {
            *cell = (ch, paint);
        }
    }

    fn text(&mut self, x: usize, y: usize, text: &str, paint: Paint) {
        for (i, ch) in text.chars().enumerate() {
            self.put(x + i, y, ch, paint);
        }
    }

    /// A box `width` wide and three rows high, with `title` in the top border.
    fn draw_box(
        &mut self,
        x: usize,
        y: usize,
        width: usize,
        title: &str,
        note: &str,
        paint: Paint,
    ) {
        if width < 4 {
            return;
        }
        let inner = width - 2;
        let title = cut(title, inner.saturating_sub(2));
        let mut top = format!("┌ {title} ");
        while top.chars().count() < width - 1 {
            top.push('─');
        }
        top.push('┐');
        self.text(x, y, &top, paint);
        self.put(x, y + 1, '│', paint);
        self.put(x + width - 1, y + 1, '│', paint);
        let note_paint = if paint == Paint::Selected {
            Paint::Selected
        } else {
            Paint::Note
        };
        let body = cut(note, inner.saturating_sub(1));
        self.text(x + 1, y + 1, &format!(" {body}"), note_paint);
        let mut bottom = String::from("└");
        bottom.push_str(&"─".repeat(inner));
        bottom.push('┘');
        self.text(x, y + 2, &bottom, paint);
    }
}

/// Rows of a side that fit, and where the visible slice starts so that
/// `selected` stays in view.
fn window(count: usize, fit: usize, selected: Option<usize>) -> (usize, usize) {
    let shown = count.min(fit.max(1));
    let start = match selected {
        Some(s) if s >= shown => (s + 1 - shown).min(count - shown),
        _ => 0,
    };
    (start, shown)
}

/// Draws `center` with its neighbours inside `width` x `height` cells.
/// `note` is a line under the title of the middle table, for example when it
/// has no foreign keys.
pub fn render(
    width: usize,
    height: usize,
    center: &str,
    note: &str,
    left: &[Node],
    right: &[Node],
    selected: Option<Pick>,
) -> Canvas {
    let mut canvas = Canvas::new(width, height);
    // Columns: left box, gap, middle box, gap, right box.
    let gap = 7usize.min(width / 8).max(3);
    let center_width = (width / 3)
        .clamp(14, 28)
        .min(width.saturating_sub(2 * gap + 8));
    let side_width = width.saturating_sub(center_width + 2 * gap) / 2;
    if side_width < 8 || height < BOX_HEIGHT {
        canvas.text(0, 0, &cut(center, width), Paint::Center);
        return canvas;
    }
    let left_x = 0;
    let center_x = side_width + gap;
    let right_x = center_x + center_width + gap;
    let right_width = width - right_x;
    let bus_left = side_width + gap / 2;
    let bus_right = center_x + center_width + gap / 2;

    // Rows available. A side that does not fit keeps a row at each end for
    // its "more" marks.
    let longest = left.len().max(right.len());
    let overflow = longest * PITCH > height + 1;
    let usable = if overflow {
        height.saturating_sub(2)
    } else {
        height
    };
    let fit = ((usable + 1) / PITCH).max(1);
    let top_pad = usize::from(overflow);

    let pick = |side: Side| match selected {
        Some((s, i)) if s == side => Some(i),
        _ => None,
    };
    let (left_start, left_shown) = window(left.len(), fit, pick(Side::Left));
    let (right_start, right_shown) = window(right.len(), fit, pick(Side::Right));
    let visible = left_shown.max(right_shown).max(1);

    // The middle box sits halfway down the tallest side.
    let span = visible * PITCH - 1;
    let center_y = top_pad + span.saturating_sub(BOX_HEIGHT) / 2;
    let center_row = center_y + 1;

    let mut masks = vec![vec![0u8; width]; height];
    let link = |from: usize, to: usize, y: usize, masks: &mut Vec<Vec<u8>>| {
        for bits in &mut masks[y][from..=to] {
            *bits |= LEFT | RIGHT;
        }
    };

    // Left side: each box reaches the bus, which reaches the middle box.
    let left_rows: Vec<usize> = (0..left_shown).map(|i| top_pad + i * PITCH + 1).collect();
    let right_rows: Vec<usize> = (0..right_shown).map(|i| top_pad + i * PITCH + 1).collect();
    for (rows, bus, from, to) in [
        (&left_rows, bus_left, side_width, center_x - 1),
        (&right_rows, bus_right, center_x + center_width, right_x - 1),
    ] {
        if rows.is_empty() {
            continue;
        }
        let low = rows
            .iter()
            .chain(std::iter::once(&center_row))
            .min()
            .copied()
            .unwrap_or(0);
        let high = rows
            .iter()
            .chain(std::iter::once(&center_row))
            .max()
            .copied()
            .unwrap_or(0);
        let is_left = bus == bus_left;
        // Each neighbour's row reaches the bus from its own side, and the
        // middle box's row reaches it from the other. Bits are only added, so
        // a neighbour on the middle row still joins up.
        for &row in rows {
            if is_left {
                if bus > from {
                    link(from, bus - 1, row, &mut masks);
                }
                masks[row][bus] |= LEFT;
            } else {
                link(bus + 1, to, row, &mut masks);
                masks[row][bus] |= RIGHT;
            }
        }
        if is_left {
            link(bus + 1, to, center_row, &mut masks);
            masks[center_row][bus] |= RIGHT;
        } else {
            if bus > from {
                link(from, bus - 1, center_row, &mut masks);
            }
            masks[center_row][bus] |= LEFT;
        }
        for (y, row) in masks.iter_mut().enumerate().take(high + 1).skip(low) {
            if y > low {
                row[bus] |= UP;
            }
            if y < high {
                row[bus] |= DOWN;
            }
        }
    }
    for (y, row) in masks.iter().enumerate() {
        for (x, bits) in row.iter().enumerate() {
            if *bits != 0 {
                canvas.put(x, y, joint(*bits), Paint::Line);
            }
        }
    }
    // Arrows run from the table that holds the key to the one it points at.
    if !left_rows.is_empty() {
        canvas.put(center_x - 1, center_row, '▶', Paint::Line);
    }
    for &row in &right_rows {
        canvas.put(right_x - 1, row, '▶', Paint::Line);
    }

    for (i, node) in left.iter().enumerate().skip(left_start).take(left_shown) {
        let paint = if pick(Side::Left) == Some(i) {
            Paint::Selected
        } else {
            Paint::Box
        };
        canvas.draw_box(
            left_x,
            top_pad + (i - left_start) * PITCH,
            side_width,
            &node.title,
            &node.note,
            paint,
        );
    }
    for (i, node) in right.iter().enumerate().skip(right_start).take(right_shown) {
        let paint = if pick(Side::Right) == Some(i) {
            Paint::Selected
        } else {
            Paint::Box
        };
        canvas.draw_box(
            right_x,
            top_pad + (i - right_start) * PITCH,
            right_width,
            &node.title,
            &node.note,
            paint,
        );
    }
    canvas.draw_box(
        center_x,
        center_y,
        center_width,
        center,
        note,
        Paint::Center,
    );

    // A diagram shorter than the room sits in the middle of it.
    if !overflow && span < height {
        canvas.cells.rotate_right((height - span) / 2);
    }

    if overflow {
        for (side, x, w, start, shown, total) in [
            (
                Side::Left,
                left_x,
                side_width,
                left_start,
                left_shown,
                left.len(),
            ),
            (
                Side::Right,
                right_x,
                right_width,
                right_start,
                right_shown,
                right.len(),
            ),
        ] {
            let _ = side;
            if start > 0 {
                canvas.text(x, 0, &cut(&format!("▲ {start} more"), w), Paint::Note);
            }
            let below = total - start - shown;
            if below > 0 {
                canvas.text(
                    x,
                    height - 1,
                    &cut(&format!("▼ {below} more"), w),
                    Paint::Note,
                );
            }
        }
    }
    canvas
}

#[cfg(test)]
mod tests {
    use super::*;

    fn node(title: &str, note: &str) -> Node {
        Node {
            title: title.to_string(),
            note: note.to_string(),
        }
    }

    fn lines(canvas: &Canvas) -> Vec<String> {
        canvas
            .cells
            .iter()
            .map(|row| {
                row.iter()
                    .map(|(c, _)| *c)
                    .collect::<String>()
                    .trim_end()
                    .to_string()
            })
            .collect()
    }

    #[test]
    fn a_table_with_neighbours_on_both_sides_is_joined_by_lines_and_arrows() {
        let left = [
            node("public.items", "(order_id) → (id)"),
            node("public.ledger", "(ref) → (id)"),
        ];
        let right = [node("sales.customers", "(customer_id) → (id)")];
        let canvas = render(80, 12, "public.orders", "", &left, &right, None);
        let out = lines(&canvas).join("\n");
        for expected in [
            "public.items",
            "public.ledger",
            "sales.customers",
            "public.orders",
            "(order_id) → (id)",
            "▶",
        ] {
            assert!(out.contains(expected), "missing {expected:?}:\n{out}");
        }
        // Two arrows leave the middle box's right side toward one table, and
        // the left tables share the one that enters the middle box.
        assert_eq!(out.matches('▶').count(), 2, "{out}");
        assert!(
            out.contains('├') || out.contains('┤') || out.contains('┬'),
            "{out}"
        );
    }

    #[test]
    fn the_middle_box_sits_between_its_neighbours() {
        let left = [node("a.one", ""), node("a.two", ""), node("a.three", "")];
        let canvas = render(80, 14, "m.middle", "", &left, &[], None);
        let rows = lines(&canvas);
        let row_of = |text: &str| rows.iter().position(|l| l.contains(text)).expect(text);
        assert!(row_of("a.one") < row_of("m.middle") && row_of("m.middle") < row_of("a.three"));
    }

    #[test]
    fn the_selected_box_is_painted_apart() {
        let left = [node("a.one", ""), node("a.two", "")];
        let canvas = render(
            80,
            12,
            "m.middle",
            "",
            &left,
            &[node("b.one", "")],
            Some((Side::Left, 1)),
        );
        let painted: Vec<(usize, Paint)> = canvas
            .cells
            .iter()
            .enumerate()
            .filter_map(|(y, row)| row.iter().find(|(c, _)| *c == '┌').map(|(_, p)| (y, *p)))
            .collect();
        assert_eq!(
            painted
                .iter()
                .filter(|(_, p)| *p == Paint::Selected)
                .count(),
            1
        );
    }

    #[test]
    fn a_long_list_scrolls_to_keep_the_selection_in_view_and_counts_the_rest() {
        let left: Vec<Node> = (0..9).map(|n| node(&format!("t.n{n}"), "")).collect();
        let canvas = render(80, 10, "m.middle", "", &left, &[], Some((Side::Left, 7)));
        let out = lines(&canvas).join("\n");
        assert!(out.contains("t.n7"), "{out}");
        assert!(!out.contains("t.n0"), "{out}");
        assert!(out.contains("▲") && out.contains("more"), "{out}");
        let top = render(80, 10, "m.middle", "", &left, &[], Some((Side::Left, 0)));
        let out = lines(&top).join("\n");
        assert!(out.contains("t.n0") && out.contains("▼"), "{out}");
    }

    #[test]
    fn a_table_without_keys_is_just_its_box_and_long_names_are_cut() {
        let canvas = render(
            60,
            8,
            "public.a_very_long_table_name_indeed",
            "no foreign keys",
            &[],
            &[],
            None,
        );
        let out = lines(&canvas).join("\n");
        assert!(
            out.contains("no foreign keys") && out.contains('…'),
            "{out}"
        );
        assert!(!out.contains('▶'));
    }

    #[test]
    fn tiny_areas_do_not_panic() {
        for (w, h) in [(0, 0), (5, 2), (20, 3), (30, 5), (80, 1)] {
            let _ = render(
                w,
                h,
                "m",
                "",
                &[node("a", "")],
                &[node("b", "")],
                Some((Side::Left, 0)),
            );
        }
    }

    #[test]
    fn connectors_join_where_a_neighbour_is_level_with_the_middle_box() {
        let n = |t: &str| node(t, "");
        let canvas = render(
            72,
            12,
            "m.middle",
            "",
            &[n("a.one"), n("a.two"), n("a.three")],
            &[n("b.one"), n("b.two")],
            None,
        );
        let out = lines(&canvas).join("\n");
        // Level with the middle box on the left: a crossing. On the right, a
        // line that comes up from the neighbour above and leaves to the one
        // below.
        assert!(out.contains("┼"), "{out}");
        assert!(out.contains("┴") || out.contains("┬"), "{out}");
        assert!(out.contains("┌──▶"), "{out}");
    }
}
