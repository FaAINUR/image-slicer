//! Нарезка изображения и сохранение тайлов.

use std::fs;
use std::path::{Path, PathBuf};

use image::RgbaImage;

use crate::grid::MergedArea;

pub type SaveResult = Result<(u64, PathBuf), String>;

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
) -> SaveResult {
    let [tw, th] = tile_size;

    if tw == 0 || th == 0 {
        return Err("Размер клетки должен быть больше нуля.".into());
    }

    let folder = create_output_folder(output)?;
    let columns = image.width().div_ceil(tw);
    let rows = image.height().div_ceil(th);
    let mut count = 0u64;

    for row in 0..rows {
        for column in 0..columns {
            let cell = (column, row);
            let group = merged.iter().find(|area| area.contains(cell));

            let (right, bottom, prefix) = if let Some(area) = group {
                // Объединённую область сохраняем только один раз.
                if column != area.left || row != area.top {
                    continue;
                }

                (area.right, area.bottom, "merged")
            } else {
                (column, row, "tile")
            };

            let x = column * tw;
            let y = row * th;

            let end_x =
                ((u64::from(right) + 1) * u64::from(tw)).min(u64::from(image.width())) as u32;
            let end_y =
                ((u64::from(bottom) + 1) * u64::from(th)).min(u64::from(image.height())) as u32;

            let tile = image::imageops::crop_imm(image, x, y, end_x - x, end_y - y).to_image();

            let file = folder.join(format!("{prefix}_{count:05}_x{x}_y{y}.png"));

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
    }

    Ok((count, folder))
}
