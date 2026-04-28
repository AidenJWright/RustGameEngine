//! Grid navigation helpers for CTF point-and-click movement.

use std::cmp::Ordering;
use std::collections::{BinaryHeap, VecDeque};

use crate::game::ctf::resources::NavigationGrid;
use crate::game::ctf::{PATH_GRID_CELL_SIZE, PLAYER_RADIUS};

/// Build a navigation grid from wall AABBs covering the given arena bounds.
pub fn build_navigation_grid(
    walls: &[(f32, f32, f32, f32)],
    arena_width: f32,
    arena_height: f32,
) -> NavigationGrid {
    let cell_size = PATH_GRID_CELL_SIZE;
    let cols = (arena_width / cell_size).ceil() as usize;
    let rows = (arena_height / cell_size).ceil() as usize;
    let mut walkable = vec![true; cols * rows];

    for row in 0..rows {
        for col in 0..cols {
            let (x, y) = cell_center(cell_size, col, row);
            let blocked = walls.iter().any(|(wall_x, wall_y, half_w, half_h)| {
                x >= wall_x - half_w - PLAYER_RADIUS
                    && x <= wall_x + half_w + PLAYER_RADIUS
                    && y >= wall_y - half_h - PLAYER_RADIUS
                    && y <= wall_y + half_h + PLAYER_RADIUS
            });
            walkable[row * cols + col] = !blocked;
        }
    }

    NavigationGrid {
        cell_size,
        cols,
        rows,
        walkable,
        arena_width,
        arena_height,
    }
}

/// Find a smoothed world-space path between two world positions.
pub fn find_path(
    grid: &NavigationGrid,
    start: (f32, f32),
    goal: (f32, f32),
) -> Option<Vec<(f32, f32)>> {
    let start_cell = nearest_walkable_cell(grid, world_to_cell(grid, start))?;
    let goal_cell = nearest_walkable_cell(grid, world_to_cell(grid, goal))?;
    if start_cell == goal_cell {
        return Some(vec![cell_to_world(grid, goal_cell)]);
    }

    let cell_path = astar(grid, start_cell, goal_cell)?;
    let mut waypoints = cell_path
        .into_iter()
        .skip(1)
        .map(|cell| cell_to_world(grid, cell))
        .collect::<Vec<_>>();

    if let Some(last) = waypoints.last_mut() {
        let target = clamp_world(grid, goal);
        if line_of_sight(grid, *last, target) {
            *last = target;
        }
    }

    Some(smooth_path(grid, start, waypoints))
}

/// Returns true if a straight segment stays on walkable cells.
pub fn line_of_sight(grid: &NavigationGrid, from: (f32, f32), to: (f32, f32)) -> bool {
    let dx = to.0 - from.0;
    let dy = to.1 - from.1;
    let dist = (dx * dx + dy * dy).sqrt();
    let steps = (dist / (grid.cell_size * 0.5)).ceil().max(1.0) as usize;

    (0..=steps).all(|idx| {
        let t = idx as f32 / steps as f32;
        let x = from.0 + dx * t;
        let y = from.1 + dy * t;
        let cell = world_to_cell(grid, (x, y));
        grid.is_walkable_index(index(grid, cell.0, cell.1))
    })
}

fn smooth_path(
    grid: &NavigationGrid,
    start: (f32, f32),
    waypoints: Vec<(f32, f32)>,
) -> Vec<(f32, f32)> {
    if waypoints.len() <= 1 {
        return waypoints;
    }

    let mut result = Vec::new();
    let mut anchor = start;
    let mut idx = 0;
    while idx < waypoints.len() {
        let mut furthest = idx;
        for candidate in (idx + 1)..waypoints.len() {
            if line_of_sight(grid, anchor, waypoints[candidate]) {
                furthest = candidate;
            }
        }
        let next = waypoints[furthest];
        result.push(next);
        anchor = next;
        idx = furthest + 1;
    }
    result
}

fn astar(
    grid: &NavigationGrid,
    start: (usize, usize),
    goal: (usize, usize),
) -> Option<Vec<(usize, usize)>> {
    let total = grid.cols * grid.rows;
    let mut open = BinaryHeap::new();
    let mut came_from: Vec<Option<usize>> = vec![None; total];
    let mut g_score = vec![u32::MAX; total];
    let start_idx = index(grid, start.0, start.1);
    let goal_idx = index(grid, goal.0, goal.1);

    g_score[start_idx] = 0;
    open.push(Node {
        index: start_idx,
        cost: heuristic(start, goal),
    });

    while let Some(Node { index: current, .. }) = open.pop() {
        if current == goal_idx {
            return Some(reconstruct_path(grid, came_from, current));
        }

        let (col, row) = coords(grid, current);
        for (next_col, next_row, step_cost) in neighbors(grid, col, row) {
            let next_idx = index(grid, next_col, next_row);
            let tentative = g_score[current].saturating_add(step_cost);
            if tentative >= g_score[next_idx] {
                continue;
            }
            came_from[next_idx] = Some(current);
            g_score[next_idx] = tentative;
            open.push(Node {
                index: next_idx,
                cost: tentative.saturating_add(heuristic((next_col, next_row), goal)),
            });
        }
    }

    None
}

fn reconstruct_path(
    grid: &NavigationGrid,
    came_from: Vec<Option<usize>>,
    mut current: usize,
) -> Vec<(usize, usize)> {
    let mut path = vec![coords(grid, current)];
    while let Some(previous) = came_from[current] {
        current = previous;
        path.push(coords(grid, current));
    }
    path.reverse();
    path
}

fn neighbors(grid: &NavigationGrid, col: usize, row: usize) -> Vec<(usize, usize, u32)> {
    let mut out = Vec::with_capacity(8);
    for dy in -1_i32..=1 {
        for dx in -1_i32..=1 {
            if dx == 0 && dy == 0 {
                continue;
            }
            let next_col = col as i32 + dx;
            let next_row = row as i32 + dy;
            if next_col < 0
                || next_row < 0
                || next_col >= grid.cols as i32
                || next_row >= grid.rows as i32
            {
                continue;
            }

            let next_col = next_col as usize;
            let next_row = next_row as usize;
            if !grid.is_walkable_index(index(grid, next_col, next_row)) {
                continue;
            }

            let diagonal = dx != 0 && dy != 0;
            if diagonal {
                let horizontal = index(grid, next_col, row);
                let vertical = index(grid, col, next_row);
                if !grid.is_walkable_index(horizontal) || !grid.is_walkable_index(vertical) {
                    continue;
                }
            }

            out.push((next_col, next_row, if diagonal { 14 } else { 10 }));
        }
    }
    out
}

fn nearest_walkable_cell(
    grid: &NavigationGrid,
    start: (usize, usize),
) -> Option<(usize, usize)> {
    let start_idx = index(grid, start.0, start.1);
    if grid.is_walkable_index(start_idx) {
        return Some(start);
    }

    let mut visited = vec![false; grid.cols * grid.rows];
    let mut queue = VecDeque::new();
    visited[start_idx] = true;
    queue.push_back(start);

    while let Some((col, row)) = queue.pop_front() {
        for (next_col, next_row, _) in all_neighbors(grid, col, row) {
            let next_idx = index(grid, next_col, next_row);
            if visited[next_idx] {
                continue;
            }
            if grid.is_walkable_index(next_idx) {
                return Some((next_col, next_row));
            }
            visited[next_idx] = true;
            queue.push_back((next_col, next_row));
        }
    }

    None
}

fn all_neighbors(grid: &NavigationGrid, col: usize, row: usize) -> Vec<(usize, usize, u32)> {
    let mut out = Vec::with_capacity(8);
    for dy in -1_i32..=1 {
        for dx in -1_i32..=1 {
            if dx == 0 && dy == 0 {
                continue;
            }
            let next_col = col as i32 + dx;
            let next_row = row as i32 + dy;
            if next_col >= 0
                && next_row >= 0
                && next_col < grid.cols as i32
                && next_row < grid.rows as i32
            {
                out.push((next_col as usize, next_row as usize, 10));
            }
        }
    }
    out
}

fn heuristic(a: (usize, usize), b: (usize, usize)) -> u32 {
    let dx = a.0.abs_diff(b.0) as u32;
    let dy = a.1.abs_diff(b.1) as u32;
    10 * dx.max(dy) + 4 * dx.min(dy)
}

fn world_to_cell(grid: &NavigationGrid, world: (f32, f32)) -> (usize, usize) {
    let world = clamp_world(grid, world);
    let col = (world.0 / grid.cell_size).floor() as usize;
    let row = (world.1 / grid.cell_size).floor() as usize;
    (col.min(grid.cols - 1), row.min(grid.rows - 1))
}

fn cell_to_world(grid: &NavigationGrid, cell: (usize, usize)) -> (f32, f32) {
    cell_center(grid.cell_size, cell.0, cell.1)
}

fn cell_center(cell_size: f32, col: usize, row: usize) -> (f32, f32) {
    (
        col as f32 * cell_size + cell_size * 0.5,
        row as f32 * cell_size + cell_size * 0.5,
    )
}

fn clamp_world(grid: &NavigationGrid, world: (f32, f32)) -> (f32, f32) {
    (
        world.0.clamp(0.0, grid.arena_width),
        world.1.clamp(0.0, grid.arena_height),
    )
}

fn index(grid: &NavigationGrid, col: usize, row: usize) -> usize {
    row * grid.cols + col
}

fn coords(grid: &NavigationGrid, index: usize) -> (usize, usize) {
    (index % grid.cols, index / grid.cols)
}

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
struct Node {
    index: usize,
    cost: u32,
}

impl Ord for Node {
    fn cmp(&self, other: &Self) -> Ordering {
        other
            .cost
            .cmp(&self.cost)
            .then_with(|| other.index.cmp(&self.index))
    }
}

impl PartialOrd for Node {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn path_avoids_expanded_wall() {
        let grid = build_navigation_grid(&[(100.0, 100.0, 10.0, 60.0)], 1280.0, 720.0);
        let path = find_path(&grid, (40.0, 100.0), (180.0, 100.0)).expect("path");
        assert!(path.len() > 1);
        assert!(path.iter().all(|point| line_of_sight(&grid, *point, *point)));
    }

    #[test]
    fn blocked_target_resolves_to_walkable_cell() {
        let grid = build_navigation_grid(&[(100.0, 100.0, 20.0, 20.0)], 1280.0, 720.0);
        let path = find_path(&grid, (40.0, 100.0), (100.0, 100.0)).expect("path");
        let last = *path.last().expect("last waypoint");
        assert!(line_of_sight(&grid, last, last));
    }
}
