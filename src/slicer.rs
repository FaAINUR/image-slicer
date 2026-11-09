//! Нарезка изображения и сохранение тайлов.

use std::fs;
use std::path::{Path, PathBuf};

use image::RgbaImage;

use crate::grid::{Cell, MergedArea};

pub type SaveResult = Result<(u64, PathBuf), String>;

/// Прямоугольник изображения, который нужно сохранить как один PNG.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TilePlan {
    pub x: u32,
    pub y: u32,
    pub width: u32,
    pub height: u32,
    pub merged: bool,
}

/// Составляет план нарезки без обращения к файловой системе.
pub fn plan_tiles(
    width: u32,
    height: u32,
    tile_size: [u32; 2],
    merged: &[MergedArea],
    only_selected: &[Cell],
) -> Result<Vec<TilePlan>, String> {
    let [tile_width, tile_height] = tile_size;

    if tile_width == 0 || tile_height == 0 {
        return Err("Размер клетки должен быть больше нуля.".into());
    }

    if width == 0 || height == 0 {
        return Err("Изображение пустое.".into());
    }

    let columns = width.div_ceil(tile_width);
    let rows = height.div_ceil(tile_height);
    let selected: std::collections::BTreeSet<Cell> = only_selected.iter().copied().collect();
    let mut plans = Vec::new();

    for row in 0..rows {
        for column in 0..columns {
            let cell = (column, row);
            let group = merged.iter().find(|area| area.contains(cell));

            if !selected.is_empty() {
                let selected_area = group.filter(|area| {
                    selected
                        .iter()
                        .any(|selected_cell| area.contains(*selected_cell))
                });

                let is_selected = selected.contains(&cell) || selected_area.is_some();

                if !is_selected {
                    continue;
                }
            }

            let (right, bottom, merged) = if let Some(area) = group {
                // Объединённую область добавляем в план только один раз.
                if column != area.left || row != area.top {
                    continue;
                }

                (area.right, area.bottom, true)
            } else {
                (column, row, false)
            };

            let x = column * tile_width;
            let y = row * tile_height;

            let end_x = ((u64::from(right) + 1) * u64::from(tile_width))
                .min(u64::from(width)) as u32;
            let end_y = ((u64::from(bottom) + 1) * u64::from(tile_height))
                .min(u64::from(height)) as u32;

            plans.push(TilePlan {
                x,
                y,
                width: end_x - x,
                height: end_y - y,
                merged,
            });
        }
    }

    Ok(plans)
}

pub fn create_output_folder(root: &Path) -> Result<PathBuf, String> {
    fs::create_dir_all(root).map_err(|error| error.to_string())?;

    for index in 1..=1_000_000u32 {
        let folder = root.join(format!("slice_{index:04}"));

        match fs::create_dir(&folder) {
            Ok(()) => return Ok(folder),
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
                continue;
            }
            Err(error) => return Err(error.to_string()),
        }
    }

    Err("Не удалось создать папку результата.".into())
}

pub fn save_tiles(
    image: &RgbaImage,
    output: &Path,
    tile_size: [u32; 2],
    merged: &[MergedArea],
    only_selected: &[Cell],
) -> SaveResult {
    let plans = plan_tiles(
        image.width(),
        image.height(),
        tile_size,
        merged,
        only_selected,
    )?;

    let folder = create_output_folder(output)?;
    let mut count = 0u64;

    for plan in plans {
        let tile = image::imageops::crop_imm(
            image,
            plan.x,
            plan.y,
            plan.width,
            plan.height,
        )
        .to_image();

        let prefix = if plan.merged { "merged" } else { "tile" };
        let file = folder.join(format!(
            "{prefix}_{count:05}_x{}_y{}.png",
            plan.x, plan.y
        ));

        tile.save_with_format(&file, image::ImageFormat::Png)
            .map_err(|error| {
                format!(
                    "Не удалось сохранить {}: {error}. \
                     Частичный результат: {}",
                    file.display(),
                    folder.display()
                )
            })?;

        count += 1;
    }

    Ok((count, folder))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plan_tiles_splits_image_into_cells() {
        let plans = plan_tiles(100, 100, [32, 32], plan_tiles(100, 100, [32, 32], plan_tiles(100, 100, [32, 32], plan_tiles(100, 100, [32, 32], &[])[], &[])[], &[])[], &[]).unwrap();

        assert_eq!(plans.len(), 16);

        let last = plans.last().unwrap();
        assert_eq!(last.x, 96);
        assert_eq!(last.y, 96);
        assert_eq!(last.width, 4);
        assert_eq!(last.height, 4);
        assert!(!last.merged);
    }

    #[test]
    fn plan_tiles_keeps_merged_area_once() {
        let merged = [MergedArea {
            left: 0,
            top: 0,
            right: 1,
            bottom: 1,
        }];

        let plans = plan_tiles(96, 96, [32, 32], &merged, &[]).unwrap();

        assert_eq!(plans.len(), 6);

        let merged_plan = plans
            .iter()
            .find(|plan| plan.merged)
            .expect("объединённая область должна быть в плане");

        assert_eq!(merged_plan.x, 0);
        assert_eq!(merged_plan.y, 0);
        assert_eq!(merged_plan.width, 64);
        assert_eq!(merged_plan.height, 64);
    }

    #[test]
    fn plan_tiles_keeps_only_selected_cells() {
        let selected = [(0, 0), (2, 2)];

        let plans = plan_tiles(96, 96, [32, 32], &[], &selected).unwrap();

        assert_eq!(plans.len(), 2);
        assert!(plans.iter().all(|plan| !plan.merged));
    }

    #[test]
    fn plan_tiles_keeps_merged_area_with_selected_cell() {
        let merged = [MergedArea {
            left: 0,
            top: 0,
            right: 1,
            bottom: 1,
        }];
        let selected = [(1, 1)];

        let plans = plan_tiles(96, 96, [32, 32], &merged, &selected).unwrap();

        assert_eq!(plans.len(), 1);
        assert!(plans[0].merged);
        assert_eq!(plans[0].width, 64);
        assert_eq!(plans[0].height, 64);
    }

    #[test]
    fn plan_tiles_rejects_zero_tile_size() {
        let error = plan_tiles(32, 32, [0, 32], plan_tiles(32, 32, [0, 32], plan_tiles(32, 32, [0, 32], &[])[], &[])[], &[]).unwrap_err();

        assert_eq!(error, "Размер клетки должен быть больше нуля.");
    }
}
