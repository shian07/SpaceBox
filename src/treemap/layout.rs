use crate::scan::ScanTree;

const MAX_FOCUS_CHILDREN: usize = 48;
const MAX_DEPTH_ONE_CHILDREN: usize = 24;
const MAX_DEPTH_TWO_CHILDREN: usize = 12;
const MIN_NESTED_AREA: f32 = 5_000.0;
const MIN_DEEP_AREA: f32 = 18_000.0;
const MAX_DEPTH: u8 = 3;
const LABEL_HEIGHT: f32 = 15.0;
const LABEL_LARGE_HEIGHT: f32 = 27.0;

#[derive(Clone, Copy, Debug, Default)]
pub struct Rect {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
}

impl Rect {
    pub const ZERO: Rect = Rect {
        x: 0.0,
        y: 0.0,
        w: 0.0,
        h: 0.0,
    };

    pub fn new(x: f32, y: f32, w: f32, h: f32) -> Self {
        Self { x, y, w, h }
    }

    pub fn area(self) -> f32 {
        self.w.max(0.0) * self.h.max(0.0)
    }

    pub fn contains(self, x: f32, y: f32) -> bool {
        x >= self.x && y >= self.y && x < self.x + self.w && y < self.y + self.h
    }
}

#[derive(Clone, Copy, Debug)]
pub struct BoxRect {
    pub node: u32,
    pub rect: Rect,
    pub depth: u8,
    pub synthetic: bool,
    pub other_count: u32,
    pub label_height: f32,
    pub size: u64,
}

pub struct Layout {
    pub boxes: Vec<BoxRect>,
    pub total: u64,
}

impl Layout {
    pub fn empty() -> Self {
        Self {
            boxes: Vec::new(),
            total: 0,
        }
    }

    pub fn hit_test(&self, x: f32, y: f32) -> Option<usize> {
        self.boxes
            .iter()
            .enumerate()
            .rev()
            .find_map(|(index, item)| item.rect.contains(x, y).then_some(index))
    }
}

struct Entry {
    node: Option<u32>,
    size: u64,
    other_count: u32,
}

pub fn build(tree: &ScanTree, focus: u32, width: i32, height: i32, use_alloc: bool) -> Layout {
    let mut layout = Layout {
        boxes: Vec::new(),
        total: tree.size(focus, use_alloc),
    };

    if width < 24 || height < 24 || layout.total == 0 {
        return layout;
    }

    let outer = Rect::new(1.0, 1.0, width as f32 - 2.0, height as f32 - 2.0);
    layout_children(tree, focus, outer, 0, use_alloc, &mut layout.boxes);
    layout
}

fn layout_children(
    tree: &ScanTree,
    parent: u32,
    rect: Rect,
    depth: u8,
    use_alloc: bool,
    out: &mut Vec<BoxRect>,
) {
    if depth >= MAX_DEPTH || rect.w < 12.0 || rect.h < 12.0 {
        return;
    }

    let min_area = match depth {
        0 => 0.0,
        1 => MIN_NESTED_AREA,
        _ => MIN_DEEP_AREA,
    };
    if depth > 0 && rect.area() < min_area {
        return;
    }

    let max_children = match depth {
        0 => MAX_FOCUS_CHILDREN,
        1 => MAX_DEPTH_ONE_CHILDREN,
        _ => MAX_DEPTH_TWO_CHILDREN,
    };

    let mut children: Vec<(u32, u64)> = tree
        .children(parent)
        .filter_map(|id| {
            let size = tree.size(id, use_alloc);
            (size > 0).then_some((id, size))
        })
        .collect();
    if children.is_empty() {
        return;
    }

    children
        .sort_unstable_by(|left, right| right.1.cmp(&left.1).then_with(|| left.0.cmp(&right.0)));

    let mut entries: Vec<Entry> = Vec::new();
    let mut other_size = 0u64;
    let mut other_count = 0u32;
    for (index, (id, size)) in children.iter().enumerate() {
        if index < max_children {
            entries.push(Entry {
                node: Some(*id),
                size: *size,
                other_count: 0,
            });
        } else {
            other_size = other_size.saturating_add(*size);
            other_count += 1;
        }
    }
    if other_count > 0 {
        entries.push(Entry {
            node: None,
            size: other_size,
            other_count,
        });
    }

    let sizes: Vec<f64> = entries.iter().map(|entry| entry.size as f64).collect();
    let rects = squarify(&sizes, rect);

    for (entry, child_rect) in entries.iter().zip(rects) {
        if child_rect.w < 1.0 || child_rect.h < 1.0 {
            continue;
        }

        let label_height = if child_rect.w >= 90.0 && child_rect.h >= 52.0 {
            LABEL_LARGE_HEIGHT
        } else if child_rect.w >= 52.0 && child_rect.h >= 30.0 {
            LABEL_HEIGHT
        } else {
            0.0
        };

        out.push(BoxRect {
            node: entry.node.unwrap_or(parent),
            rect: child_rect,
            depth,
            synthetic: entry.node.is_none(),
            other_count: entry.other_count,
            label_height,
            size: entry.size,
        });

        let Some(node_id) = entry.node else {
            continue;
        };
        if !tree.is_dir(node_id) || tree.is_small_agg(node_id) {
            continue;
        }
        if depth + 1 >= MAX_DEPTH {
            continue;
        }

        let inner = Rect::new(
            child_rect.x + 3.0,
            child_rect.y + 3.0 + label_height,
            (child_rect.w - 6.0).max(0.0),
            (child_rect.h - 6.0 - label_height).max(0.0),
        );
        if inner.w >= 12.0 && inner.h >= 12.0 {
            layout_children(tree, node_id, inner, depth + 1, use_alloc, out);
        }
    }
}

fn worst(row: &[usize], areas: &[f64], side: f64) -> f64 {
    if row.is_empty() || side <= 0.0 {
        return f64::INFINITY;
    }
    let sum: f64 = row.iter().map(|&index| areas[index]).sum();
    if sum <= 0.0 {
        return f64::INFINITY;
    }
    let max = row
        .iter()
        .map(|&index| areas[index])
        .fold(f64::NEG_INFINITY, f64::max);
    let min = row
        .iter()
        .map(|&index| areas[index])
        .fold(f64::INFINITY, f64::min);
    (side * side * max / (sum * sum)).max(sum * sum / (side * side * min))
}

fn worst_with(row: &[usize], index: usize, areas: &[f64], side: f64) -> f64 {
    if row.is_empty() || side <= 0.0 {
        return f64::INFINITY;
    }
    let mut sum = areas[index];
    let mut max = areas[index];
    let mut min = areas[index];
    for &item in row {
        sum += areas[item];
        max = max.max(areas[item]);
        min = min.min(areas[item]);
    }
    if sum <= 0.0 {
        return f64::INFINITY;
    }
    (side * side * max / (sum * sum)).max(sum * sum / (side * side * min))
}

fn layout_row(row: &[usize], areas: &[f64], remaining: &mut Rect, result: &mut [Rect]) {
    if row.is_empty() || remaining.w <= 0.0 || remaining.h <= 0.0 {
        return;
    }
    let sum: f64 = row.iter().map(|&index| areas[index]).sum();
    if sum <= 0.0 {
        return;
    }

    if remaining.w >= remaining.h {
        let strip_w = ((sum / remaining.h as f64) as f32).min(remaining.w);
        if strip_w <= 0.0 {
            return;
        }
        let mut y = remaining.y;
        for (position, &index) in row.iter().enumerate() {
            let is_last = position + 1 == row.len();
            let height = if is_last {
                (remaining.y + remaining.h - y).max(0.0)
            } else {
                ((areas[index] / strip_w as f64) as f32).min(remaining.y + remaining.h - y)
            };
            result[index] = Rect::new(remaining.x, y, strip_w, height);
            y += height;
        }
        remaining.x += strip_w;
        remaining.w = (remaining.w - strip_w).max(0.0);
    } else {
        let strip_h = ((sum / remaining.w as f64) as f32).min(remaining.h);
        if strip_h <= 0.0 {
            return;
        }
        let mut x = remaining.x;
        for (position, &index) in row.iter().enumerate() {
            let is_last = position + 1 == row.len();
            let width = if is_last {
                (remaining.x + remaining.w - x).max(0.0)
            } else {
                ((areas[index] / strip_h as f64) as f32).min(remaining.x + remaining.w - x)
            };
            result[index] = Rect::new(x, remaining.y, width, strip_h);
            x += width;
        }
        remaining.y += strip_h;
        remaining.h = (remaining.h - strip_h).max(0.0);
    }
}

pub fn squarify(values: &[f64], rect: Rect) -> Vec<Rect> {
    let total: f64 = values.iter().sum();
    if values.is_empty() || total <= 0.0 || rect.w <= 0.0 || rect.h <= 0.0 {
        return vec![Rect::ZERO; values.len()];
    }

    let scale = (rect.w as f64 * rect.h as f64) / total;
    let areas: Vec<f64> = values.iter().map(|value| value * scale).collect();
    let mut result = vec![Rect::ZERO; values.len()];
    let mut remaining = rect;
    let mut row: Vec<usize> = Vec::new();

    for index in 0..areas.len() {
        if areas[index] <= 0.0 {
            continue;
        }
        let side = remaining.w.min(remaining.h) as f64;
        if row.is_empty() {
            row.push(index);
            continue;
        }

        let current = worst(&row, &areas, side);
        let with_item = worst_with(&row, index, &areas, side);
        if with_item <= current {
            row.push(index);
        } else {
            layout_row(&row, &areas, &mut remaining, &mut result);
            row.clear();
            row.push(index);
        }
    }
    if !row.is_empty() {
        layout_row(&row, &areas, &mut remaining, &mut result);
    }

    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scan::arena::{flags, TreeArena, NONE};
    use crate::scan::{ScanStats, ScanTree};

    fn rect_area(rect: Rect) -> f64 {
        rect.w.max(0.0) as f64 * rect.h.max(0.0) as f64
    }

    #[test]
    fn squarify_fills_and_preserves_area() {
        let rect = Rect::new(0.0, 0.0, 400.0, 300.0);
        let values = [6.0, 6.0, 4.0, 3.0, 2.0, 2.0, 1.0];
        let rects = squarify(&values, rect);
        let sum: f64 = rects.iter().map(|item| rect_area(*item)).sum();
        assert!((sum - 400.0 * 300.0).abs() < 20.0, "sum was {sum}");
        for item in &rects {
            assert!(item.x >= rect.x - 0.1);
            assert!(item.y >= rect.y - 0.1);
            assert!(item.x + item.w <= rect.x + rect.w + 0.5);
            assert!(item.y + item.h <= rect.y + rect.h + 0.5);
        }
    }

    #[test]
    fn squarify_handles_zero_and_empty() {
        assert_eq!(squarify(&[], Rect::new(0.0, 0.0, 10.0, 10.0)).len(), 0);
        let rects = squarify(&[0.0, 0.0], Rect::new(0.0, 0.0, 10.0, 10.0));
        assert!(rects.iter().all(|item| item.w == 0.0 && item.h == 0.0));
    }

    fn sample_tree() -> ScanTree {
        let mut arena = TreeArena::new();
        let root = arena.add_node(NONE, "root", flags::ROOT | flags::DIR, 0, 0);
        let big = arena.add_node(root, "big", flags::DIR, 0, 0);
        let medium = arena.add_node(root, "medium", flags::DIR, 0, 0);
        let file = arena.add_node(root, "movie.mp4", 0, 600, 600);
        arena.nodes[file as usize].files = 1;
        let nested = arena.add_node(big, "nested.bin", 0, 400, 400);
        arena.nodes[nested as usize].files = 1;
        arena.nodes[big as usize].alloc = 400;
        arena.nodes[big as usize].logical = 400;
        arena.nodes[medium as usize].alloc = 300;
        arena.nodes[medium as usize].logical = 300;
        arena.nodes[root as usize].alloc = 1300;
        arena.nodes[root as usize].logical = 1300;
        ScanTree {
            arena,
            root: "/tmp/root".into(),
            stats: ScanStats::default(),
            errors: Vec::new(),
            top_files: vec![file],
            top_folders: vec![big],
        }
    }

    #[test]
    fn builds_nested_boxes_and_hit_tests_deepest() {
        let tree = sample_tree();
        let layout = build(&tree, 0, 600, 400, true);
        assert!(!layout.boxes.is_empty());
        assert!(layout.boxes.iter().any(|item| item.depth == 1));
        let nested = layout
            .boxes
            .iter()
            .find(|item| item.node == 1 && item.depth == 0)
            .unwrap();
        let point_x = nested.rect.x + nested.rect.w * 0.5;
        let point_y = nested.rect.y + nested.rect.h * 0.5;
        let hit = layout.hit_test(point_x, point_y).unwrap();
        assert!(layout.boxes[hit].depth >= 1);
    }

    #[test]
    fn caps_children_and_adds_other_box() {
        let mut tree = sample_tree();
        let root = tree.root_id();
        for index in 0..60 {
            let id = tree
                .arena
                .add_node(root, &format!("file-{index}.bin"), 0, 10, 10);
            tree.arena.nodes[id as usize].files = 1;
        }
        tree.arena.nodes[root as usize].alloc += 600;
        let layout = build(&tree, 0, 900, 600, true);
        let focus_boxes: Vec<_> = layout.boxes.iter().filter(|item| item.depth == 0).collect();
        assert!(focus_boxes.len() <= MAX_FOCUS_CHILDREN + 1);
        assert!(focus_boxes.iter().any(|item| item.synthetic));
    }
}
