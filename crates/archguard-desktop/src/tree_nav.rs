//! The keyboard's Left key in a flattened tree (Matrix, Calls): the row of
//! each row's parent, derived from the depths of the visible rows (the
//! Explorer's `TreeRow.parent` comes from its tree instead).

/// For each row the index of the nearest row above it with a smaller depth;
/// -1 when none is shown (a root, or a parent on an earlier page).
pub fn parents(depths: &[i32]) -> Vec<i32> {
    // The chain of open ancestors: (row, depth), depths strictly rising.
    let mut chain: Vec<(usize, i32)> = Vec::new();
    depths
        .iter()
        .enumerate()
        .map(|(i, &d)| {
            while chain.last().is_some_and(|&(_, cd)| cd >= d) {
                chain.pop();
            }
            let parent = chain.last().map_or(-1, |&(r, _)| r as i32);
            chain.push((i, d));
            parent
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::parents;

    #[test]
    fn a_row_points_at_the_nearest_shallower_row_above() {
        // a / a.x / a.x.1 / a.y / b / b.z
        assert_eq!(parents(&[0, 1, 2, 1, 0, 1]), [-1, 0, 1, 0, -1, 4]);
    }

    #[test]
    fn a_page_starting_below_the_root_has_no_parent_for_its_first_rows() {
        assert_eq!(parents(&[2, 2, 1, 2]), [-1, -1, -1, 2]);
    }
}
