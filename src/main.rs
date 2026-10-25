#![cfg_attr(
    all(target_os = "windows", not(debug_assertions)),
    windows_subsystem = "windows"
)]

use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver};
use std::sync::Arc;

use eframe::egui;
use egui_file_dialog::FileDialog;
use image::RgbaImage;

type Cell = (u32, u32);
type SaveResult = Result<(u64, PathBuf), String>;

fn main() -> eframe::Result {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("Image Slicer")
            .with_inner_size([1200.0, 800.0])
            .with_min_inner_size([850.0, 550.0]),
        ..Default::default()
    };

    eframe::run_native(
        "Image Slicer",
        options,
        Box::new(|cc| {
            cc.egui_ctx.set_visuals(egui::Visuals::dark());
            Ok(Box::new(ImageSlicerApp::default()))
        }),
    )
}

#[derive(Clone, Copy)]
enum DialogTarget {
    Image,
    Folder,
}

// Координаты области в клетках, правая и нижняя границы включены.
#[derive(Clone, Copy, Debug)]
struct MergedArea {
    left: u32,
    top: u32,
    right: u32,
    bottom: u32,
}

impl MergedArea {
    fn contains(&self, cell: Cell) -> bool {
        cell.0 >= self.left && cell.0 <= self.right && cell.1 >= self.top && cell.1 <= self.bottom
    }

    fn cell_count(&self) -> u64 {
        u64::from(self.right - self.left + 1) * u64::from(self.bottom - self.top + 1)
    }
}

struct ImageSlicerApp {
    file_dialog: FileDialog,
    dialog_target: DialogTarget,

    input_path: Option<PathBuf>,
    output_folder: Option<PathBuf>,

    source: Option<Arc<RgbaImage>>,
    texture: Option<egui::TextureHandle>,
    image_size: [u32; 2],

    tile_width: String,
    tile_height: String,

    zoom: f32,
    show_grid: bool,
    checkerboard: bool,

    selected: BTreeSet<Cell>,
    merged: Vec<MergedArea>,
    active_merged: Option<usize>,

    status: String,
    save_receiver: Option<Receiver<SaveResult>>,
}

impl Default for ImageSlicerApp {
    fn default() -> Self {
        Self {
            file_dialog: FileDialog::default(),
            dialog_target: DialogTarget::Image,

            input_path: None,
            output_folder: None,

            source: None,
            texture: None,
            image_size: [0, 0],

            tile_width: "32".into(),
            tile_height: "32".into(),

            zoom: 1.0,
            show_grid: true,
            checkerboard: true,

            selected: BTreeSet::new(),
            merged: Vec::new(),
            active_merged: None,

            status: "Выбери изображение.".into(),
            save_receiver: None,
        }
    }
}

impl eframe::App for ImageSlicerApp {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        self.poll_save();

        egui::Panel::left("settings")
            .exact_size(320.0)
            .show_inside(ui, |ui| {
                egui::ScrollArea::vertical()
                    .id_salt("settings_scroll")
                    .show(ui, |ui| {
                        self.settings(ui);
                    });
            });

        egui::CentralPanel::default().show_inside(ui, |ui| {
            self.preview(ui);
        });

        self.file_dialog.update(ui);

        if let Some(path) = self.file_dialog.take_picked() {
            match self.dialog_target {
                DialogTarget::Image => self.load_image(path, ui.ctx()),
                DialogTarget::Folder => {
                    self.output_folder = Some(path);
                    self.status = "Папка результата выбрана.".into();
                }
            }
        }

        if self.save_receiver.is_some() {
            ui.ctx()
                .request_repaint_after(std::time::Duration::from_millis(100));
        }
    }
}

impl ImageSlicerApp {
    fn tile_size(&self) -> Option<[u32; 2]> {
        let width = self.tile_width.trim().parse::<u32>().ok()?;
        let height = self.tile_height.trim().parse::<u32>().ok()?;

        if width == 0 || height == 0 {
            None
        } else {
            Some([width, height])
        }
    }

    fn clear_selection(&mut self) {
        self.selected.clear();
        self.active_merged = None;
    }

    fn reset_merges(&mut self) {
        self.clear_selection();
        self.merged.clear();
    }

    fn settings(&mut self, ui: &mut egui::Ui) {
        let busy = self.save_receiver.is_some();

        ui.heading("Настройки");
        ui.separator();

        if ui
            .add_enabled(!busy, egui::Button::new("Выбрать изображение"))
            .clicked()
        {
            self.dialog_target = DialogTarget::Image;
            self.file_dialog.pick_file();
        }

        if let Some(path) = &self.input_path {
            let name = path.file_name().unwrap_or_default().to_string_lossy();

            ui.label(format!("Файл: {name}"));
        }

        ui.add_space(10.0);

        if ui
            .add_enabled(!busy, egui::Button::new("Выбрать папку результата"))
            .clicked()
        {
            self.dialog_target = DialogTarget::Folder;
            self.file_dialog.pick_directory();
        }

        if let Some(path) = &self.output_folder {
            ui.label(path.display().to_string());
        } else {
            ui.weak("Автоматически: рядом с картинкой");
        }

        if ui
            .add_enabled(
                !busy && self.output_folder.is_some(),
                egui::Button::new("Сбросить папку"),
            )
            .clicked()
        {
            self.output_folder = None;
        }

        ui.separator();
        ui.heading("Размер базовой клетки");

        let old_width = self.tile_width.clone();
        let old_height = self.tile_height.clone();

        ui.add_enabled_ui(!busy, |ui| {
            egui::Grid::new("size_fields").show(ui, |ui| {
                ui.label("Ширина:");
                ui.add(egui::TextEdit::singleline(&mut self.tile_width).desired_width(90.0));
                ui.end_row();

                ui.label("Высота:");
                ui.add(egui::TextEdit::singleline(&mut self.tile_height).desired_width(90.0));
                ui.end_row();
            });

            ui.horizontal(|ui| {
                for size in [16, 32, 64, 128] {
                    if ui.button(size.to_string()).clicked() {
                        self.tile_width = size.to_string();
                        self.tile_height = size.to_string();
                    }
                }
            });
        });

        if old_width != self.tile_width || old_height != self.tile_height {
            self.reset_merges();
            self.status = "Размер клетки изменён. Объединения сброшены.".into();
        }

        ui.weak("Изменение размера клетки сбрасывает объединения.");

        ui.separator();
        ui.heading("Объединение");

        ui.label("Щёлкай по соседним клеткам, затем нажми «Объединить».");
        ui.label(format!("Выбрано клеток: {}", self.selected.len()));
        ui.label(format!("Объединённых областей: {}", self.merged.len()));

        if ui
            .add_enabled(
                !busy && self.selected.len() >= 2,
                egui::Button::new("Объединить выбранные"),
            )
            .clicked()
        {
            self.merge_selected();
        }

        if ui.button("Снять выделение").clicked() {
            self.clear_selection();
        }

        if ui
            .add_enabled(
                !busy && self.active_merged.is_some(),
                egui::Button::new("Разъединить выбранную область"),
            )
            .clicked()
        {
            if let Some(index) = self.active_merged.take() {
                self.merged.remove(index);
                self.status = "Область разъединена.".into();
            }
        }

        if ui
            .add_enabled(
                !busy && !self.merged.is_empty(),
                egui::Button::new("Сбросить все объединения"),
            )
            .clicked()
        {
            self.reset_merges();
            self.status = "Все объединения сброшены.".into();
        }

        ui.weak("Клик по синей области выбирает её для разъединения.");
        ui.weak("Объединять можно только полностью заполненный прямоугольник.");

        ui.separator();
        ui.heading("Отображение");

        ui.checkbox(&mut self.show_grid, "Показывать границы");
        ui.checkbox(&mut self.checkerboard, "Шахматный фон");

        if let Some([tw, th]) = self.tile_size() {
            if self.source.is_some() {
                let columns = self.image_size[0].div_ceil(tw);
                let rows = self.image_size[1].div_ceil(th);

                let base_count = u64::from(columns) * u64::from(rows);

                let removed: u64 = self.merged.iter().map(|area| area.cell_count() - 1).sum();

                ui.label(format!("Будет сохранено PNG: {}", base_count - removed));

                if self.image_size[0] % tw != 0 || self.image_size[1] % th != 0 {
                    ui.colored_label(
                        egui::Color32::YELLOW,
                        "Крайние клетки будут меньше заданного размера.",
                    );
                }
            }
        } else {
            ui.colored_label(egui::Color32::YELLOW, "Введи целые размеры больше нуля.");
        }

        ui.add_space(10.0);

        if ui
            .add_enabled(
                !busy && self.source.is_some() && self.tile_size().is_some(),
                egui::Button::new("Нарезать и сохранить PNG").min_size(egui::vec2(285.0, 40.0)),
            )
            .clicked()
        {
            self.start_save();
        }

        ui.weak("Перед сохранением нажми «Объединить»: жёлтое выделение само по себе не объединяет клетки.");
        ui.weak("Рамки и фон в PNG не сохраняются.");
        ui.weak("Объединения действуют только в текущем сеансе.");

        ui.separator();

        if busy {
            ui.horizontal(|ui| {
                ui.spinner();
                ui.label("Сохраняю...");
            });
        }

        ui.label(&self.status);
    }

    fn merge_selected(&mut self) {
        if self.selected.len() < 2 {
            return;
        }

        let left = self.selected.iter().map(|cell| cell.0).min().unwrap();
        let right = self.selected.iter().map(|cell| cell.0).max().unwrap();
        let top = self.selected.iter().map(|cell| cell.1).min().unwrap();
        let bottom = self.selected.iter().map(|cell| cell.1).max().unwrap();

        let area = MergedArea {
            left,
            top,
            right,
            bottom,
        };

        if area.cell_count() != self.selected.len() as u64 {
            self.status = "Выделение должно заполнять прямоугольник без пропусков.".into();
            return;
        }

        if self.merged.iter().any(|existing| {
            !(area.right < existing.left
                || area.left > existing.right
                || area.bottom < existing.top
                || area.top > existing.bottom)
        }) {
            self.status = "Новая область пересекается с существующим объединением.".into();
            return;
        }

        self.merged.push(area);
        self.selected.clear();
        self.active_merged = Some(self.merged.len() - 1);

        self.status = "Клетки объединены. Синяя область сохранится одним PNG.".into();
    }

    fn preview(&mut self, ui: &mut egui::Ui) {
        ui.heading("Предпросмотр и выбор клеток");

        if self.texture.is_none() {
            ui.label("Выбери изображение слева.");
            return;
        }

        let [width, height] = self.image_size;
        ui.label(format!("Изображение: {width} × {height}"));

        let mut fit = false;

        ui.horizontal_wrapped(|ui| {
            if ui.button("−").clicked() {
                self.zoom = (self.zoom / 1.25).clamp(0.1, 8.0);
            }
            if ui.button("+").clicked() {
                self.zoom = (self.zoom * 1.25).clamp(0.1, 8.0);
            }
            if ui.button("100%").clicked() {
                self.zoom = 1.0;
            }
            if ui.button("Вписать").clicked() {
                fit = true;
            }

            ui.add(
                egui::Slider::new(&mut self.zoom, 0.1..=8.0)
                    .logarithmic(true)
                    .text("Масштаб"),
            );

            ui.label(format!("{:.0}%", self.zoom * 100.0));
        });

        ui.horizontal_wrapped(|ui| {
            ui.colored_label(egui::Color32::GREEN, "Зелёный: обычные клетки");
            ui.colored_label(egui::Color32::YELLOW, "Жёлтый: выделение");
            ui.colored_label(egui::Color32::from_rgb(40, 170, 255), "Синий: объединение");
        });

        ui.separator();

        if fit {
            let available = ui.available_size();

            self.zoom = (available.x.max(1.0) / width as f32)
                .min(available.y.max(1.0) / height as f32)
                .clamp(0.1, 8.0);
        }

        let texture_id = self.texture.as_ref().unwrap().id();
        let tile_size = self.tile_size();
        let zoom = self.zoom;
        let mut clicked_cell = None;

        egui::ScrollArea::both()
            .id_salt("preview_scroll")
            .auto_shrink([false, false])
            .show(ui, |ui| {
                let size = egui::vec2(width as f32 * zoom, height as f32 * zoom);

                let (response, painter) = ui.allocate_painter(size, egui::Sense::click());

                let rect = response.rect;
                let visible = rect.intersect(painter.clip_rect());

                if !visible.is_positive() {
                    return;
                }

                if self.checkerboard {
                    draw_checkerboard(&painter, rect, visible);
                } else {
                    painter.rect_filled(visible, 0.0, egui::Color32::from_gray(65));
                }

                painter.image(
                    texture_id,
                    rect,
                    egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
                    egui::Color32::WHITE,
                );

                let Some([tw, th]) = tile_size else {
                    return;
                };

                let columns = width.div_ceil(tw);
                let rows = height.div_ceil(th);

                let step_x = tw as f32 * zoom;
                let step_y = th as f32 * zoom;

                if response.clicked() && self.save_receiver.is_none() {
                    if let Some(pos) = response.interact_pointer_pos() {
                        let column = ((pos.x - rect.left()) / step_x).floor() as u32;
                        let row = ((pos.y - rect.top()) / step_y).floor() as u32;

                        if column < columns && row < rows {
                            clicked_cell = Some((column, row));
                        }
                    }
                }

                let area_rect = |area: MergedArea| {
                    let right_px =
                        ((u64::from(area.right) + 1) * u64::from(tw)).min(u64::from(width)) as f32;

                    let bottom_px = ((u64::from(area.bottom) + 1) * u64::from(th))
                        .min(u64::from(height)) as f32;

                    egui::Rect::from_min_max(
                        egui::pos2(
                            rect.left() + area.left as f32 * step_x,
                            rect.top() + area.top as f32 * step_y,
                        ),
                        egui::pos2(rect.left() + right_px * zoom, rect.top() + bottom_px * zoom),
                    )
                };

                // Ограничиваем обход видимой частью изображения.
                let first_column = ((visible.left() - rect.left()) / step_x).floor() as u32;
                let last_column =
                    (((visible.right() - rect.left()) / step_x).ceil() as u32).min(columns);

                let first_row = ((visible.top() - rect.top()) / step_y).floor() as u32;
                let last_row = (((visible.bottom() - rect.top()) / step_y).ceil() as u32).min(rows);

                for row in first_row..last_row {
                    for column in first_column..last_column {
                        let cell = (column, row);

                        // У объединений внутренние линии не рисуются.
                        if self.merged.iter().any(|area| area.contains(cell)) {
                            continue;
                        }

                        let cell_rect = area_rect(MergedArea {
                            left: column,
                            right: column,
                            top: row,
                            bottom: row,
                        });

                        let selected = self.selected.contains(&cell);

                        if selected {
                            painter.rect_filled(
                                cell_rect,
                                0.0,
                                egui::Color32::from_rgba_unmultiplied(255, 220, 0, 65),
                            );
                        }

                        if self.show_grid || selected {
                            painter.rect_stroke(
                                cell_rect,
                                0.0,
                                egui::Stroke::new(
                                    2.0,
                                    if selected {
                                        egui::Color32::YELLOW
                                    } else {
                                        egui::Color32::GREEN
                                    },
                                ),
                                egui::StrokeKind::Inside,
                            );
                        }
                    }
                }

                for (index, area) in self.merged.iter().enumerate() {
                    let merged_rect = area_rect(*area);

                    if !merged_rect.intersects(visible) {
                        continue;
                    }

                    let active = self.active_merged == Some(index);

                    if active {
                        painter.rect_filled(
                            merged_rect,
                            0.0,
                            egui::Color32::from_rgba_unmultiplied(30, 140, 255, 55),
                        );
                    }

                    if self.show_grid || active {
                        painter.rect_stroke(
                            merged_rect,
                            0.0,
                            egui::Stroke::new(
                                if active { 4.0 } else { 2.0 },
                                egui::Color32::from_rgb(40, 170, 255),
                            ),
                            egui::StrokeKind::Inside,
                        );
                    }
                }
            });

        if let Some(cell) = clicked_cell {
            self.handle_cell_click(cell);
        }
    }

    fn handle_cell_click(&mut self, cell: Cell) {
        if let Some(index) = self.merged.iter().position(|area| area.contains(cell)) {
            self.selected.clear();
            self.active_merged = Some(index);
            self.status = "Выбрана объединённая область. Её можно разъединить.".into();
            return;
        }

        self.active_merged = None;

        if !self.selected.insert(cell) {
            self.selected.remove(&cell);
        }
    }

    fn load_image(&mut self, path: PathBuf, ctx: &egui::Context) {
        let image = match image::open(&path) {
            Ok(image) => image.to_rgba8(),
            Err(error) => {
                self.status = format!("Ошибка открытия: {error}");
                return;
            }
        };

        let width = image.width();
        let height = image.height();

        if width == 0 || height == 0 {
            self.status = "Пустое изображение.".into();
            return;
        }

        let max_side = (ctx.input(|input| input.max_texture_side) as u32).clamp(1, 4096);

        let preview = if width > max_side || height > max_side {
            image::DynamicImage::ImageRgba8(image.clone())
                .thumbnail(max_side, max_side)
                .to_rgba8()
        } else {
            image.clone()
        };

        let color_image = egui::ColorImage::from_rgba_unmultiplied(
            [preview.width() as usize, preview.height() as usize],
            preview.as_raw(),
        );

        self.texture =
            Some(ctx.load_texture("source_image", color_image, egui::TextureOptions::NEAREST));

        self.source = Some(Arc::new(image));
        self.input_path = Some(path);
        self.image_size = [width, height];
        self.zoom = 1.0;

        self.reset_merges();
        self.status = "Изображение загружено. Выбирай клетки мышью.".into();
    }

    fn start_save(&mut self) {
        if self.save_receiver.is_some() {
            return;
        }

        let Some(image) = self.source.clone() else {
            return;
        };
        let Some(path) = self.input_path.as_ref() else {
            return;
        };
        let Some(tile_size) = self.tile_size() else {
            return;
        };

        let output = self.output_folder.clone().unwrap_or_else(|| {
            let name = path.file_stem().unwrap_or_default().to_string_lossy();

            path.with_file_name(format!("{name}_tiles"))
        });

        let merged = self.merged.clone();
        let (sender, receiver) = mpsc::channel();

        self.save_receiver = Some(receiver);
        self.status = "Сохраняю PNG с учётом объединений...".into();

        std::thread::spawn(move || {
            let result = save_tiles(&image, &output, tile_size, &merged);
            let _ = sender.send(result);
        });
    }

    fn poll_save(&mut self) {
        let result = match &self.save_receiver {
            Some(receiver) => receiver.try_recv(),
            None => return,
        };

        match result {
            Ok(Ok((count, folder))) => {
                self.save_receiver = None;
                self.status = format!("Готово! PNG: {count}\nПапка: {}", folder.display());
            }
            Ok(Err(error)) => {
                self.save_receiver = None;
                self.status = format!("Ошибка: {error}");
            }
            Err(mpsc::TryRecvError::Empty) => {}
            Err(mpsc::TryRecvError::Disconnected) => {
                self.save_receiver = None;
                self.status = "Поток сохранения завершился с ошибкой.".into();
            }
        }
    }
}

fn draw_checkerboard(painter: &egui::Painter, image_rect: egui::Rect, visible: egui::Rect) {
    let cell = 12.0;

    let first_x = ((visible.left() - image_rect.left()) / cell).floor() as usize;
    let last_x = ((visible.right() - image_rect.left()) / cell).ceil() as usize;

    let first_y = ((visible.top() - image_rect.top()) / cell).floor() as usize;
    let last_y = ((visible.bottom() - image_rect.top()) / cell).ceil() as usize;

    for row in first_y..last_y {
        for column in first_x..last_x {
            let color = if (column + row) % 2 == 0 {
                egui::Color32::from_gray(210)
            } else {
                egui::Color32::from_gray(145)
            };

            let x = image_rect.left() + column as f32 * cell;
            let y = image_rect.top() + row as f32 * cell;

            let rect = egui::Rect::from_min_size(egui::pos2(x, y), egui::vec2(cell, cell))
                .intersect(visible);

            if rect.is_positive() {
                painter.rect_filled(rect, 0.0, color);
            }
        }
    }
}

fn create_output_folder(root: &Path) -> Result<PathBuf, String> {
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

fn save_tiles(
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
