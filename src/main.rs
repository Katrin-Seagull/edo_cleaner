// =========================================================
// EDO Cache Cleaner for Yandex Browser
// Target domains: tportal.vaz.ru, tportale.vaz.ru
// =========================================================

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use eframe::egui;
use rusqlite::Connection;
use std::env;
use std::fs;
use std::os::windows::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::mpsc::{self, Receiver};
use std::thread;
use std::time::Duration;
use walkdir::WalkDir;
use winreg::enums::*;
use winreg::RegKey;

const CREATE_NO_WINDOW: u32 = 0x08000000;
const APP_REG_NAME: &str = "EDOCacheCleaner";
const APP_SETTINGS_KEY: &str = r"Software\EDOCacheCleaner";

// Эта строка вшивается прямо внутрь скомпилированного .exe файла
#[used]
static AUTHOR_SIGNATURE: &str = "EDO Cache Cleaner | made by Antonova Ekaterina";

struct CleanStats {
    profiles_checked: usize,
    cookies_deleted: usize,
    files_deleted: usize,
}

fn main() -> Result<(), eframe::Error> {
    let args: Vec<String> = env::args().collect();

    // Фоновый режим при старте ПК: без окон, почистил и закрылся
    if args.iter().any(|arg| arg == "--silent") {
        let _ = clean_edo_data();
        return Ok(());
    }

    let autostart_active = init_autostart_state();

    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([360.0, 250.0])
            .with_resizable(false)
            .with_maximize_button(false),
        ..Default::default()
    };

    eframe::run_native(
        "Очистка ЭДО",
        options,
        Box::new(move |cc| {
            setup_interface_style(&cc.egui_ctx);
            Box::new(CleanerApp {
                autostart: autostart_active,
                status_text: "Готов к очистке кеша tportal / tportale".to_string(),
                cleaned_now: false,
                show_confirm: false,
                is_cleaning: false,
                clean_rx: None,
            })
        }),
    )
}

struct CleanerApp {
    autostart: bool,
    status_text: String,
    cleaned_now: bool,
    show_confirm: bool,
    is_cleaning: bool,
    clean_rx: Option<Receiver<Result<CleanStats, String>>>,
}

impl CleanerApp {
    fn start_cleaning(&mut self, ctx: &egui::Context) {
        self.is_cleaning = true;
        self.cleaned_now = false;
        self.status_text = "Закрытие браузера и очистка...".to_string();

        let (tx, rx) = mpsc::channel();
        self.clean_rx = Some(rx);
        let ctx_clone = ctx.clone();

        thread::spawn(move || {
            let result = clean_edo_data();
            let _ = tx.send(result);
            ctx_clone.request_repaint();
        });
    }
}

impl eframe::App for CleanerApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        // Проверяем завершение фонового потока очистки
        if let Some(rx) = &self.clean_rx {
            if let Ok(result) = rx.try_recv() {
                self.is_cleaning = false;
                self.clean_rx = None;
                match result {
                    Ok(stats) => {
                        self.cleaned_now = true;
                        self.status_text = format!(
                            "Готово! Удалено: куки — {}, файлов — {}",
                            stats.cookies_deleted, stats.files_deleted
                        );
                    }
                    Err(err) => {
                        self.cleaned_now = false;
                        self.status_text = err;
                    }
                }
            } else if self.is_cleaning {
                ctx.request_repaint_after(Duration::from_millis(100));
            }
        }

        egui::CentralPanel::default().show(ctx, |ui| {
            ui.vertical_centered(|ui| {
                ui.add_space(28.0);

                let btn_color = if self.is_cleaning {
                    egui::Color32::from_gray(90)
                } else if self.cleaned_now {
                    egui::Color32::from_rgb(40, 167, 69)
                } else {
                    egui::Color32::from_rgb(0, 120, 215)
                };

                let btn_label = if self.is_cleaning {
                    "Очистка..."
                } else {
                    "Очистить кеш ЭДО"
                };

                let main_btn = egui::Button::new(
                    egui::RichText::new(btn_label)
                        .size(21.0)
                        .strong()
                        .color(egui::Color32::WHITE),
                )
                .min_size(egui::vec2(260.0, 95.0))
                .fill(btn_color)
                .rounding(18.0);

                if ui
                    .add_enabled(!self.is_cleaning && !self.show_confirm, main_btn)
                    .clicked()
                {
                    self.show_confirm = true;
                }

                ui.add_space(16.0);

                ui.label(
                    egui::RichText::new(&self.status_text)
                        .size(13.0)
                        .strong()
                        .color(egui::Color32::from_gray(220)),
                );
            });

            // Нижний блок: прижимаем к низу
            ui.with_layout(egui::Layout::bottom_up(egui::Align::Center), |ui| {
                ui.label(
                    egui::RichText::new("created by Antonova Ekaterina")
                        .size(9.0)
                        .color(egui::Color32::from_gray(55)),
                );

                ui.add_space(22.0);

                ui.horizontal(|ui| {
                    let label_text = "Автоочистка при запуске ПК";
                    let toggle_width = ui.spacing().interact_size.y * 1.8;
                    let gap = 8.0;
                    let text_width = ui.fonts(|f| {
                        f.layout_no_wrap(
                            label_text.to_string(),
                            egui::FontId::proportional(12.0),
                            egui::Color32::WHITE,
                        )
                        .size()
                        .x
                    });
                    let content_width =
                        toggle_width + gap + text_width + ui.spacing().item_spacing.x * 2.0;
                    let left_margin = (ui.available_width() - content_width) / 2.0;
                    if left_margin > 0.0 {
                        ui.add_space(left_margin);
                    }

                    if toggle_switch(ui, &mut self.autostart).changed() {
                        if let Err(e) = set_autostart(self.autostart) {
                            self.status_text = format!("Ошибка реестра: {}", e);
                            self.autostart = !self.autostart;
                        }
                    }
                    ui.add_space(gap);

                    ui.label(
                        egui::RichText::new(label_text)
                            .size(12.0)
                            .strong()
                            .color(egui::Color32::from_gray(220)),
                    );
                });
            });
        });

        // Модальное окно подтверждения перед ручной очисткой
        if self.show_confirm {
            egui::Window::new("Подтверждение")
                .collapsible(false)
                .resizable(false)
                .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
                .fixed_size([280.0, 110.0])
                .show(ctx, |ui| {
                    ui.vertical_centered(|ui| {
                        ui.add_space(6.0);
                        ui.label(
                            egui::RichText::new("Очистка закроет Яндекс Браузер.\nПродолжить?")
                                .size(13.0)
                                .color(egui::Color32::from_gray(230)),
                        );
                        ui.add_space(14.0);
                        ui.horizontal(|ui| {
                            let ok_w = 92.0;
                            let cancel_w = 88.0;
                            let gap = 8.0;
                            let buttons_width =
                                ok_w + cancel_w + gap + ui.spacing().item_spacing.x * 2.0;
                            let margin = (ui.available_width() - buttons_width) / 2.0;
                            if margin > 0.0 {
                                ui.add_space(margin);
                            }

                            let ok_btn = egui::Button::new(
                                egui::RichText::new("Продолжить")
                                    .strong()
                                    .color(egui::Color32::WHITE),
                            )
                            .min_size(egui::vec2(ok_w, 28.0))
                            .fill(egui::Color32::from_rgb(0, 120, 215));

                            let cancel_btn = egui::Button::new("Отмена")
                                .min_size(egui::vec2(cancel_w, 28.0));

                            if ui.add(ok_btn).clicked() {
                                self.show_confirm = false;
                                self.start_cleaning(ctx);
                            }
                            ui.add_space(gap);
                            if ui.add(cancel_btn).clicked() {
                                self.show_confirm = false;
                            }
                        });
                    });
                });
        }
    }
}

fn toggle_switch(ui: &mut egui::Ui, on: &mut bool) -> egui::Response {
    let desired_size = ui.spacing().interact_size.y * egui::vec2(1.8, 0.9);
    let (rect, mut response) = ui.allocate_exact_size(desired_size, egui::Sense::click());
    if response.clicked() {
        *on = !*on;
        response.mark_changed();
    }
    if ui.is_rect_visible(rect) {
        let how_on = ui.ctx().animate_bool(response.id, *on);
        let bg_color = if *on {
            egui::Color32::from_rgb(40, 167, 69)
        } else {
            egui::Color32::from_gray(90)
        };
        let radius = 0.5 * rect.height();
        ui.painter().rect_filled(rect, radius, bg_color);
        let circle_x = egui::lerp((rect.left() + radius)..=(rect.right() - radius), how_on);
        let center = egui::pos2(circle_x, rect.center().y);
        ui.painter()
            .circle_filled(center, 0.75 * radius, egui::Color32::WHITE);
    }
    response
}

fn setup_interface_style(ctx: &egui::Context) {
    let mut visuals = egui::Visuals::dark();
    visuals.panel_fill = egui::Color32::from_rgb(28, 30, 36);
    ctx.set_visuals(visuals);
}

// =========================================================
// ЛОГИКА ОЧИСТКИ КЕША И КУКИ ЭДО (ТОЛЬКО ЯНДЕКС БРАУЗЕР)
// =========================================================
fn close_yandex_browser() {
    // 1. Сначала пробуем закрыть мягко (без /F)
    let first_try = Command::new("taskkill")
        .args(["/IM", "browser.exe", "/T"])
        .creation_flags(CREATE_NO_WINDOW)
        .status();

    // Если браузер не был запущен — выходим сразу без пауз
    if !matches!(first_try, Ok(status) if status.success()) {
        return;
    }

    thread::sleep(Duration::from_millis(1200));

    // 2. Добиваем оставшиеся фоновые процессы, если они ещё висят
    let second_try = Command::new("taskkill")
        .args(["/F", "/IM", "browser.exe", "/T"])
        .creation_flags(CREATE_NO_WINDOW)
        .status();

    if matches!(second_try, Ok(status) if status.success()) {
        thread::sleep(Duration::from_millis(400));
    }
}

fn clean_edo_data() -> Result<CleanStats, String> {
    let local_app_data =
        env::var("LOCALAPPDATA").map_err(|_| "Не найдена папка LOCALAPPDATA".to_string())?;
    let yandex_data = PathBuf::from(local_app_data).join(r"Yandex\YandexBrowser\User Data");

    if !yandex_data.exists() {
        return Err("Яндекс Браузер не найден".to_string());
    }

    close_yandex_browser();

    let mut stats = CleanStats {
        profiles_checked: 0,
        cookies_deleted: 0,
        files_deleted: 0,
    };

    let entries =
        fs::read_dir(&yandex_data).map_err(|_| "Нет доступа к папке профилей".to_string())?;

    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            let name = entry.file_name().to_string_lossy().to_lowercase();
            if name == "default" || name.starts_with("profile") {
                stats.profiles_checked += 1;
                clean_profile(&path, &mut stats);
            }
        }
    }

    if stats.profiles_checked == 0 {
        return Err("Профили Яндекс Браузера не найдены".to_string());
    }

    Ok(stats)
}

fn clean_profile(profile_path: &Path, stats: &mut CleanStats) {
    // 1. Точечное удаление куки ТОЛЬКО для tportal и tportale (остальные сайты не разлогиниваются)
    for cookie_rel in [r"Network\Cookies", "Cookies"] {
        let cookie_db = profile_path.join(cookie_rel);
        if cookie_db.exists() {
            if let Ok(conn) = Connection::open(&cookie_db) {
                if let Ok(deleted) = conn.execute(
                    "DELETE FROM cookies WHERE host_key LIKE '%tportal.vaz.ru%' OR host_key LIKE '%tportale.vaz.ru%'",
                    [],
                ) {
                    stats.cookies_deleted += deleted;
                }
            }
        }
    }

    // 2. Полная очистка папок кеша Яндекс Браузера (гарантированно убирает весь кеш tportal / tportale)
    for cache_dir in [
        "Cache",
        "Code Cache",
        "GPUCache",
        "Media Cache",
        r"Service Worker\CacheStorage",
        r"Service Worker\ScriptCache",
    ] {
        stats.files_deleted += clear_dir_contents(&profile_path.join(cache_dir));
    }

    // 3. Точечная очистка локальных хранилищ для tportal / tportale
    for storage_dir in ["IndexedDB", "File System", "Service Worker", "WebStorage"] {
        let dir_path = profile_path.join(storage_dir);
        if !dir_path.exists() {
            continue;
        }
        let mut it = WalkDir::new(&dir_path).into_iter();
        while let Some(entry_res) = it.next() {
            let Ok(entry) = entry_res else { continue };
            let p = entry.path();
            let name = p
                .file_name()
                .unwrap_or_default()
                .to_string_lossy()
                .to_lowercase();
            if name.contains("tportal") {
                if p.is_dir() {
                    if fs::remove_dir_all(p).is_ok() {
                        stats.files_deleted += 1;
                    }
                    it.skip_current_dir();
                } else if fs::remove_file(p).is_ok() {
                    stats.files_deleted += 1;
                }
            }
        }
    }
}

fn clear_dir_contents(dir: &Path) -> usize {
    let mut removed = 0;
    if let Ok(entries) = fs::read_dir(dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            let ok = if path.is_dir() {
                fs::remove_dir_all(&path).is_ok()
            } else {
                fs::remove_file(&path).is_ok()
            };
            if ok {
                removed += 1;
            }
        }
    }
    removed
}

// =========================================================
// АВТОЗАГРУЗКА (С СОХРАНЕНИЕМ ВЫБОРА ПОЛЬЗОВАТЕЛЯ)
// =========================================================
fn init_autostart_state() -> bool {
    let hkcu = RegKey::predef(HKEY_CURRENT_USER);
    let already_initialized = hkcu
        .open_subkey(APP_SETTINGS_KEY)
        .and_then(|k| k.get_value::<u32, _>("Initialized"))
        .unwrap_or(0)
        == 1;

    if !already_initialized {
        // Первый запуск: включаем автозагрузку по умолчанию и ставим отметку
        let enabled = set_autostart(true).is_ok();
        if let Ok((settings_key, _)) = hkcu.create_subkey(APP_SETTINGS_KEY) {
            let _ = settings_key.set_value("Initialized", &1u32);
        }
        return enabled;
    }

    // Последующие запуски: уважаем выбор пользователя
    let is_enabled = hkcu
        .open_subkey(r"Software\Microsoft\Windows\CurrentVersion\Run")
        .and_then(|k| k.get_value::<String, _>(APP_REG_NAME))
        .is_ok();

    if is_enabled {
        // Если автозагрузка включена, обновляем путь на случай переноса .exe
        let _ = set_autostart(true);
    }

    is_enabled
}

fn set_autostart(enable: bool) -> std::io::Result<()> {
    let hkcu = RegKey::predef(HKEY_CURRENT_USER);
    let (run_key, _) = hkcu.create_subkey(r"Software\Microsoft\Windows\CurrentVersion\Run")?;
    if enable {
        let exe_path = env::current_exe()?;
        let cmd = format!("\"{}\" --silent", exe_path.display());
        run_key.set_value(APP_REG_NAME, &cmd)?;
    } else {
        let _ = run_key.delete_value(APP_REG_NAME);
    }
    Ok(())
}