//! Сетка спрайтшита и объединённые области.

pub type Cell = (u32, u32);

/// Координаты области в клетках, правая и нижняя границы включены.
#[derive(Clone, Copy, Debug)]
pub struct MergedArea {
    pub left: u32,
    pub top: u32,
    pub right: u32,
    pub bottom: u32,
}

impl MergedArea {
    pub fn contains(&self, cell: Cell) -> bool {
        cell.0 >= self.left
            && cell.0 <= self.right
            && cell.1 >= self.top
            && cell.1 <= self.bottom
    }

    pub fn cell_count(&self) -> u64 {
        u64::from(self.right - self.left + 1) * u64::from(self.bottom - self.top + 1)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn contains_includes_borders() {
        let area = MergedArea {
            left: 1,
            top: 2,
            right: 3,
            bottom: 4,
        };

        assert!(area.contains((1, 2)));
        assert!(area.contains((3, 4)));
        assert!(!area.contains((0, 2)));
        assert!(!area.contains((1, 5)));
    }

    #[test]
    fn cell_count_counts_all_cells() {
        let area = MergedArea {
            left: 0,
            top: 0,
            right: 1,
            bottom: 2,
        };

        assert_eq!(area.cell_count(), 6);
    }
}
