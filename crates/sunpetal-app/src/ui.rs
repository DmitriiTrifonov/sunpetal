//! The settings menu, drawn over the game with egui (F1, or Esc, opens and closes it). The text is
//! in the language of the system (or the one chosen in the menu): English and Russian, which
//! egui's bundled Ubuntu font covers. More languages come with the translation packs (docs/PLAN.md,
//! stage 9).
use crate::config::{self, Lang, Settings};

pub const NAMES: [&str; 12] = ["A", "B", "SELECT", "START", "RIGHT", "LEFT", "UP", "DOWN", "R", "L", "X", "Y"];

/// What the menu asks of the application beyond changing the settings.
#[derive(PartialEq, Debug)]
pub enum Action {
    Quit,
    /// Start the program anew (the settings saved, the game's backup memory written first).
    Restart,
    /// A copy of the game's save to this file.
    ExportSave(std::path::PathBuf),
    /// This file replaces the game's save, and the game starts anew.
    ImportSave(std::path::PathBuf),
}

pub struct Menu {
    pub open: bool,
    /// The button whose key the next key press sets.
    pub rebinding: Option<String>,
    profile_name: String,
    profiles: Vec<String>,
    message: String,
    /// The restart button was pressed once: the next press does it.
    restart_ask: bool,
    /// The import button was pressed once: the next press asks for the file.
    import_ask: bool,
    /// What egui has now (None: the bundled font only), to reload only when it changes.
    font_set: Option<String>,
    /// The short menu (sound, screen, picture, saves) instead of the full one; set by the window from `[ui] full_menu`.
    pub short: bool,
    /// The "main menu" button was pressed once: the next press does it.
    main_ask: bool,
    /// The focus ring (for the pad and the keyboard) was put into egui's style.
    focus_style: bool,
}

/// What the short menu holds, in order; everything else (effects, keys, profiles, debug...) is the full menu's.
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Block {
    Sound,
    Screen,
    Graphics,
    Saves,
    Leave,
}

pub const SHORT: [Block; 5] = [Block::Sound, Block::Screen, Block::Graphics, Block::Saves, Block::Leave];

impl Menu {
    pub fn new() -> Menu {
        Menu { open: false, rebinding: None, profile_name: String::new(), profiles: config::profiles(), message: String::new(), restart_ask: false, import_ask: false, font_set: None, short: false, main_ask: false, focus_style: false }
    }

    /// A line under the settings (what a save export or import did).
    pub fn say(&mut self, text: String) {
        self.message = text;
    }

    pub fn toggle(&mut self) {
        self.open = !self.open;
        self.rebinding = None;
        (self.restart_ask, self.import_ask, self.main_ask) = (false, false, false);
        if self.open {
            self.profiles = config::profiles();
        }
    }

    /// The game's save as a file other emulators read (a desktop's file dialog).
    #[cfg(not(any(target_os = "android", target_os = "ios")))]
    fn save_row(&mut self, ui: &mut egui::Ui, tx: &dyn Fn(&'static str, &'static str) -> String, action: &mut Option<Action>) {
        ui.horizontal(|ui| {
            if ui.button(tx("Export the save…", "Сохранить копию сохранения…")).clicked() {
                if let Some(to) = rfd::FileDialog::new().set_title("Sunpetal").set_file_name("sunpetal.sav").add_filter("save", &["sav"]).save_file() {
                    *action = Some(Action::ExportSave(to));
                }
            }
            let ask = self.import_ask;
            let label = if ask {
                tx("The save is replaced (the old one is kept) and the game restarts. Press again to choose the file", "Сохранение заменится (старое останется копией), игра перезапустится. Нажмите ещё раз и выберите файл")
            } else {
                tx("Import a save…", "Загрузить сохранение из файла…")
            };
            if ui.button(label).clicked() {
                if ask {
                    if let Some(from) = rfd::FileDialog::new().set_title("Sunpetal").add_filter("save", &["sav", "dsv"]).pick_file() {
                        *action = Some(Action::ImportSave(from));
                    }
                }
                self.import_ask = !ask;
            }
        });
    }

    #[cfg(any(target_os = "android", target_os = "ios"))]
    fn save_row(&mut self, _: &mut egui::Ui, _: &dyn Fn(&'static str, &'static str) -> String, _: &mut Option<Action>) {}

    /// The short menu: sound, screen and window, picture, saves and restart, leaving.
    fn short_body(&mut self, ui: &mut egui::Ui, s: &mut Settings, layouts: &[&str], internal: (u32, u32), tx: &dyn Fn(&'static str, &'static str) -> String, action: &mut Option<Action>) {
        let lang = Lang::resolve(&s.interface.language);
        for block in SHORT {
            match block {
                Block::Sound => {
                    ui.heading(tx("Sound", "Звук"));
                    ui.add(egui::Slider::new(&mut s.audio.volume, 0.0..=1.0).text(tx("Volume", "Громкость")));
                    ui.checkbox(&mut s.audio.mute, tx("Mute", "Без звука"));
                }
                Block::Screen => {
                    ui.heading(tx("Screen and window", "Экран и окно"));
                    egui::ComboBox::from_label(tx("Screen layout", "Раскладка экранов")).selected_text(layout_label(&s.video.layout, lang).to_string()).show_ui(ui, |ui| {
                        for l in layouts {
                            ui.selectable_value(&mut s.video.layout, l.to_string(), layout_label(l, lang));
                        }
                    });
                    const SIZES: [[u32; 2]; 8] = [[512, 384], [768, 576], [1024, 768], [1280, 720], [1600, 900], [1920, 1080], [2560, 1440], [512, 768]];
                    let own = tx("own (dragged)", "своё (по перетаскиванию)");
                    let now = SIZES.iter().find(|z| **z == s.video.window).map_or(own, |z| format!("{} × {}", z[0], z[1]));
                    egui::ComboBox::from_label(tx("Window size", "Размер окна")).selected_text(now).show_ui(ui, |ui| {
                        for z in SIZES {
                            ui.selectable_value(&mut s.video.window, z, format!("{} × {}", z[0], z[1]));
                        }
                    });
                    ui.checkbox(&mut s.video.fullscreen, tx("Full screen (F11)", "Полный экран (F11)"));
                }
                Block::Graphics => {
                    ui.heading(tx("Picture", "Картинка"));
                    ui.horizontal(|ui| {
                        ui.label(tx("3D resolution:", "Разрешение 3D:"));
                        ui.add(egui::Slider::new(&mut s.video.res, 1..=8).suffix("x"));
                    });
                    ui.label(format!("{} {}x{}", tx("each screen, pixels:", "каждый экран, точек:"), internal.0, internal.1));
                    egui::ComboBox::from_label(tx("Dialogue font", "Шрифт реплик")).selected_text(match s.video.dialogue_font.as_str() {
                        "rubik" => "Rubik",
                        "ubuntu" => "Ubuntu",
                        _ => "Noto Sans",
                    }).show_ui(ui, |ui| {
                        ui.selectable_value(&mut s.video.dialogue_font, "noto".to_string(), "Noto Sans");
                        ui.selectable_value(&mut s.video.dialogue_font, "rubik".to_string(), "Rubik");
                        ui.selectable_value(&mut s.video.dialogue_font, "ubuntu".to_string(), "Ubuntu");
                    });
                    ui.checkbox(&mut s.video.linear, tx("Smooth when stretched (linear filter)", "Сглаживание при растяжении (линейный фильтр)"));
                }
                Block::Saves => {
                    ui.heading(tx("Saves", "Сохранения"));
                    self.save_row(ui, tx, action);
                    let ask = self.restart_ask;
                    let label = if ask { tx("Progress not saved in the game is lost. Press again to restart", "Несохранённый прогресс пропадёт. Нажмите ещё раз для перезапуска") } else { tx("Restart the game", "Перезапустить игру") };
                    if ui.button(label).clicked() {
                        if ask {
                            *action = Some(Action::Restart);
                        }
                        self.restart_ask = !ask;
                    }
                }
                Block::Leave => {
                    ui.separator();
                    // The machine is the recompiled program with its own threads: leaving to the title is the program started anew.
                    let ask = self.main_ask;
                    let label = if ask { tx("Progress not saved in the game is lost. Press again to go to the main menu", "Несохранённый прогресс пропадёт. Нажмите ещё раз, чтобы выйти в главное меню") } else { tx("Exit to the main menu", "Выйти в главное меню") };
                    if ui.button(label).clicked() {
                        if ask {
                            *action = Some(Action::Restart);
                        }
                        self.main_ask = !ask;
                    }
                    ui.horizontal(|ui| {
                        if ui.button(tx("Close", "Закрыть")).clicked() {
                            self.open = false;
                        }
                        if ui.button(tx("Quit", "Выйти из игры")).clicked() {
                            *action = Some(Action::Quit);
                        }
                    });
                    ui.label(tx("All the settings: full_menu = true under [ui] in the settings file, or --full-menu.", "Все настройки: full_menu = true в разделе [ui] файла настроек или --full-menu."));
                }
            }
        }
    }

    /// Draws the menu; changes go straight into `s`.
    pub fn show(&mut self, ctx: &egui::Context, s: &mut Settings, layouts: &[&str], internal: (u32, u32)) -> Option<Action> {
        // the OS font behind the bundled one: only for a language that has a table, or a chosen file
        let custom_lang = crate::uitext::codes().contains(&crate::uitext::code_of(&s.interface.language));
        let want = if custom_lang || !s.interface.font.is_empty() { Some(s.interface.font.clone()) } else { None };
        if self.font_set != want {
            let bytes = want.as_ref().and_then(|f| crate::fonts::pick(f)).map(|(_, b)| b);
            ctx.set_fonts(crate::fonts::definitions(bytes));
            self.font_set = want;
        }
        if !self.focus_style {
            // the widget the pad or the keys are on: a thick yellow ring
            ctx.all_styles_mut(|st| {
                let ring = egui::Stroke::new(2.5, egui::Color32::from_rgb(255, 214, 64));
                st.visuals.widgets.active.bg_stroke = ring;
                st.visuals.widgets.active.fg_stroke = egui::Stroke::new(2.0, egui::Color32::WHITE);
            });
            self.focus_style = true;
        }
        let lang = Lang::resolve(&s.interface.language);
        // a table in `ui/<code>.toml` (uitext.rs) translates text by text over the built-in two
        let table = crate::uitext::table(&crate::uitext::code_of(&s.interface.language));
        let tl = {
            let table = table.clone();
            move |en: &str| -> String { table.as_ref().and_then(|t| t.get(en)).cloned().unwrap_or_else(|| en.to_string()) }
        };
        let tx = {
            let table = table.clone();
            move |en: &'static str, ru: &'static str| -> String {
                table.as_ref().and_then(|t| t.get(en)).cloned().unwrap_or_else(|| if lang == Lang::Ru { ru } else { en }.to_string())
            }
        };
        let custom = crate::uitext::codes();
        let mut action = None;
        let video_res = s.video.res as u32;
        let mut open = self.open;
        egui::Window::new(tx("Sunpetal: settings", "Sunpetal: настройки")).open(&mut open).resizable(false).collapsible(false).vscroll(true).max_height((ctx.content_rect().height() - 48.0).max(120.0)).show(ctx, |ui| {
            if self.short {
                self.short_body(ui, s, layouts, internal, &tx, &mut action);
                return;
            }
            egui::ComboBox::from_label("Language / Язык")
                .selected_text(match s.interface.language.as_str() {
                    "en" => "English".to_string(),
                    "ru" => "Русский".to_string(),
                    "auto" => tx("system language", "язык системы"),
                    other => other.to_string(),
                })
                .show_ui(ui, |ui| {
                    ui.selectable_value(&mut s.interface.language, "auto".to_string(), tx("system language", "язык системы"));
                    ui.selectable_value(&mut s.interface.language, "en".to_string(), "English");
                    ui.selectable_value(&mut s.interface.language, "ru".to_string(), "Русский");
                    // the tables in ui/ (pl.toml...): the code is the name
                    for c in custom.iter().filter(|c| !matches!(c.as_str(), "en" | "ru")) {
                        ui.selectable_value(&mut s.interface.language, c.clone(), c.as_str());
                    }
                });
            egui::CollapsingHeader::new(tx("Video", "Графика")).default_open(true).show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.label(tx("3D resolution:", "Разрешение 3D:"));
                    ui.add(egui::Slider::new(&mut s.video.res, 1..=8).suffix("x"));
                });
                ui.label(format!("{} {}x{}", tx("each screen, pixels:", "каждый экран, точек:"), internal.0, internal.1));
                let rl = |v: &str| if v == "gpu" { tx("Graphics card (faster at high resolution)", "Видеокарта (быстрее при высоком разрешении)") } else { tx("Processor (the reference)", "Процессор (эталон)") };
                egui::ComboBox::from_label(tx("3D drawn by", "3D рисует")).selected_text(rl(&s.video.renderer)).show_ui(ui, |ui| {
                    for v in ["software", "gpu"] {
                        ui.selectable_value(&mut s.video.renderer, v.to_string(), rl(v));
                    }
                });
                ui.checkbox(&mut s.video.linear, tx("Smooth when stretched (linear filter)", "Сглаживание при растяжении (линейный фильтр)"));
                egui::ComboBox::from_label(tx("Screen layout", "Раскладка экранов")).selected_text(tl(layout_label(&s.video.layout, lang))).show_ui(ui, |ui| {
                    for l in layouts {
                        ui.selectable_value(&mut s.video.layout, l.to_string(), tl(layout_label(l, lang)));
                    }
                });
                egui::ComboBox::from_label(tx("Picture in the window", "Картинка в окне")).selected_text(tl(fit_label(&s.video.fit, lang))).show_ui(ui, |ui| {
                    for m in ["fit", "integer", "fill", "stretch"] {
                        ui.selectable_value(&mut s.video.fit, m.to_string(), tl(fit_label(m, lang)));
                    }
                });
                ui.horizontal_wrapped(|ui| {
                    ui.label(tx("Window shape:", "Форма окна:"));
                    let now = config::shape_name(s.video.window);
                    for (name, r) in config::SHAPES {
                        if ui.selectable_label(now == Some(name), name).clicked() {
                            s.video.window = config::window_of_shape(s.video.window, r);
                        }
                    }
                });
                ui.horizontal(|ui| {
                    ui.label(tx("Pixel width:", "Ширина пикселя:"));
                    ui.add(egui::Slider::new(&mut s.video.pixel_aspect, 0.5..=2.0));
                });
                if s.video.layout == "single" {
                    ui.checkbox(&mut s.video.auto, tx("The screen follows the game (a scene: upper, a menu: lower)", "Экран следует за игрой (сцена: верхний, меню: нижний)"));
                    ui.add_enabled_ui(!s.video.auto, |ui| ui.horizontal(|ui| {
                        ui.label(tx("Screen shown (F8):", "Какой экран (F8):"));
                        ui.selectable_value(&mut s.video.lower, false, tx("upper", "верхний"));
                        ui.selectable_value(&mut s.video.lower, true, tx("lower (touch)", "нижний (касание)"));
                    }));
                }
                ui.checkbox(&mut s.video.counter, tx("Counter on the upper screen when it is alone", "Счётчик на верхнем экране, когда он один"));
                egui::ComboBox::from_label(tx("Dialogue text", "Текст реплик")).selected_text(match s.video.dialogue_text.as_str() {
                    "pixel" => tx("pixel (the game's)", "пиксельный (как в игре)"),
                    "crisp" => tx("crisp", "чёткий"),
                    _ => tx("auto (crisp from 2x)", "авто (чёткий от 2x)"),
                }).show_ui(ui, |ui| {
                    ui.selectable_value(&mut s.video.dialogue_text, "auto".to_string(), tx("auto (crisp from 2x)", "авто (чёткий от 2x)"));
                    ui.selectable_value(&mut s.video.dialogue_text, "crisp".to_string(), tx("crisp", "чёткий"));
                    ui.selectable_value(&mut s.video.dialogue_text, "pixel".to_string(), tx("pixel (the game's)", "пиксельный (как в игре)"));
                });
                egui::ComboBox::from_label(tx("Dialogue font", "Шрифт реплик")).selected_text(match s.video.dialogue_font.as_str() {
                    "rubik" => "Rubik",
                    "ubuntu" => "Ubuntu",
                    _ => "Noto Sans",
                }).show_ui(ui, |ui| {
                    ui.selectable_value(&mut s.video.dialogue_font, "noto".to_string(), "Noto Sans");
                    ui.selectable_value(&mut s.video.dialogue_font, "rubik".to_string(), "Rubik");
                    ui.selectable_value(&mut s.video.dialogue_font, "ubuntu".to_string(), "Ubuntu");
                });
                ui.checkbox(&mut s.video.follow_shape, tx("Window follows the picture's shape", "Окно по форме картинки"));
                ui.checkbox(&mut s.video.logos_in_turn, tx("Logos in turn when one screen is shown", "Логотипы по очереди, когда виден один экран"));
                if matches!(s.video.layout.as_str(), "single" | "overlay") && s.video.auto {
                    egui::ComboBox::from_label(tx("Dial puzzle (both screens)", "Головоломка с диском (оба экрана)")).selected_text(match s.video.puzzle.as_str() {
                        "side" => tx("side by side", "рядом"),
                        _ => tx("lower screen as a panel (PS2)", "нижний экран панелью (как на PS2)"),
                    }).show_ui(ui, |ui| {
                        ui.selectable_value(&mut s.video.puzzle, "inset".to_string(), tx("lower screen as a panel (PS2)", "нижний экран панелью (как на PS2)"));
                        ui.selectable_value(&mut s.video.puzzle, "side".to_string(), tx("side by side", "рядом"));
                    });
                }
                if s.video.layout == "overlay" {
                    ui.checkbox(&mut s.video.panel, tx("Lower screen as a panel (F8)", "Нижний экран панелью (F8)"));
                }
                egui::ComboBox::from_label(tx("Background", "Подложка")).selected_text(match s.video.background.as_str() {
                    "black" => tx("black", "чёрная"),
                    "glow" => tx("glow of the picture", "свечение картинки"),
                    _ => tx("the picture's edge rows run on", "строки края картинки"),
                }).show_ui(ui, |ui| {
                    ui.selectable_value(&mut s.video.background, "edge".to_string(), tx("the picture's edge rows run on", "строки края картинки"));
                    ui.selectable_value(&mut s.video.background, "glow".to_string(), tx("glow of the picture", "свечение картинки"));
                    ui.selectable_value(&mut s.video.background, "black".to_string(), tx("black", "чёрная"));
                });
                ui.checkbox(&mut s.video.wide, tx("Widen the 3D view to the window (one screen)", "Расширять 3D под окно (один экран)"));
                ui.checkbox(&mut s.video.bars_match, tx("Bars take the picture's edge colour (logos)", "Полосы цвета края картинки (логотипы)"));
                // window sizes (logical pixels); the window is also free to drag, and the menu then shows "own"
                const SIZES: [[u32; 2]; 8] = [[512, 384], [768, 576], [1024, 768], [1280, 720], [1600, 900], [1920, 1080], [2560, 1440], [512, 768]];
                let own = tx("own (dragged)", "своё (по перетаскиванию)").to_string();
                let now = SIZES.iter().find(|z| **z == s.video.window).map_or(own, |z| format!("{} × {}", z[0], z[1]));
                egui::ComboBox::from_label(tx("Window size", "Размер окна")).selected_text(now).show_ui(ui, |ui| {
                    for z in SIZES {
                        ui.selectable_value(&mut s.video.window, z, format!("{} × {}", z[0], z[1]));
                    }
                });
                ui.checkbox(&mut s.video.fullscreen, tx("Full screen (F11)", "Полный экран (F11)"));
                // the texture sets beside the file in use (data files with pictures), switched at once
                {
                    let sets = pack_sets(&s.video.texture_pack);
                    let name = |p: &str| std::path::Path::new(p).file_stem().map_or(String::new(), |n| n.to_string_lossy().into_owned());
                    let shown = if s.video.texture_pack.is_empty() { tx("the game's own pictures", "оригинал (без подмены)").to_string() } else { name(&s.video.texture_pack) };
                    egui::ComboBox::from_label(tx("Texture set (the text and the language stay with the start)", "Набор текстур (текст и язык меняются при запуске)")).selected_text(shown).show_ui(ui, |ui| {
                        ui.selectable_value(&mut s.video.texture_pack, String::new(), tx("the game's own pictures", "оригинал (без подмены)"));
                        for p in &sets {
                            ui.selectable_value(&mut s.video.texture_pack, p.clone(), name(p));
                        }
                    });
                }
                ui.horizontal(|ui| {
                    ui.label(tx("Data file or texture folder:", "Файл данных или папка текстур:"));
                    ui.text_edit_singleline(&mut s.video.texture_pack);
                });
                if !s.video.texture_pack.is_empty() && !std::path::Path::new(&s.video.texture_pack).exists() {
                    ui.colored_label(egui::Color32::LIGHT_RED, tx("not found", "не найдено"));
                }
                egui::ComboBox::from_label(tx("Game language (needs the data file)", "Язык игры (нужен файл данных)")).selected_text(match s.game.language.as_str() {
                    "ru" => "Русский".to_string(),
                    _ => tx("the ROM's own", "как в ROM"),
                }).show_ui(ui, |ui| {
                    ui.selectable_value(&mut s.game.language, String::new(), tx("the ROM's own", "как в ROM"));
                    ui.selectable_value(&mut s.game.language, "ru".to_string(), "Русский");
                });
                ui.label(tx("Takes effect the next time the game starts.", "Применится при следующем запуске игры."));
            });
            egui::CollapsingHeader::new(tx("Effects", "Эффекты")).show(ui, |ui| {
                let label = |id: &str| -> &'static str {
                    match (id, lang == Lang::Ru) {
                        ("none", false) => "none",
                        ("none", true) => "без эффектов",
                        ("sharp", false) => "sharp pixels",
                        ("sharp", true) => "чёткие пиксели",
                        ("lcd", false) => "LCD grid",
                        ("lcd", true) => "сетка LCD",
                        ("crt", false) => "CRT",
                        ("crt", true) => "ЭЛТ",
                        ("vivid", false) => "vivid colours",
                        ("vivid", true) => "яркие цвета",
                        ("glow", false) => "soft glow",
                        ("glow", true) => "мягкое свечение",
                        (_, false) => "custom",
                        (_, true) => "свои настройки",
                    }
                };
                egui::ComboBox::from_label(tx("Preset (F6)", "Пресет (F6)")).selected_text(label(&s.effects.preset)).show_ui(ui, |ui| {
                    for p in crate::effects::PRESETS {
                        if ui.selectable_label(s.effects.preset == p, tl(label(p))).clicked() {
                            s.effects = crate::effects::Effects::preset(p);
                        }
                    }
                });
                let before = s.effects.clone();
                let e = &mut s.effects;
                ui.checkbox(&mut e.sharp, tx("Sharp pixels", "Чёткие пиксели"));
                ui.add(egui::Slider::new(&mut e.scanlines, 0.0..=1.0).text(tx("Scanlines", "Строки")));
                ui.add(egui::Slider::new(&mut e.grid, 0.0..=1.0).text(tx("Pixel grid", "Сетка пикселей")));
                ui.add(egui::Slider::new(&mut e.vignette, 0.0..=1.0).text(tx("Vignette", "Виньетка")));
                ui.add(egui::Slider::new(&mut e.saturation, 0.0..=2.0).text(tx("Saturation", "Насыщенность")));
                ui.add(egui::Slider::new(&mut e.contrast, 0.5..=1.5).text(tx("Contrast", "Контраст")));
                ui.add(egui::Slider::new(&mut e.brightness, -0.3..=0.3).text(tx("Brightness", "Яркость")));
                ui.add(egui::Slider::new(&mut e.gamma, 0.5..=2.0).text(tx("Gamma", "Гамма")));
                ui.add(egui::Slider::new(&mut e.bloom, 0.0..=1.0).text(tx("Bloom", "Свечение")));
                ui.add(egui::Slider::new(&mut e.outline, 0.0..=1.0).text(tx("3D outline", "Контур 3D")));
                ui.add_enabled(e.outline > 0.0, egui::Slider::new(&mut e.outline_width, 0.25..=2.0).text(tx("Outline width", "Толщина контура")));
                ui.add(egui::Slider::new(&mut e.shadow, 0.0..=1.0).text(tx("Soft shadows", "Мягкие тени")));
                ui.add_enabled(e.shadow > 0.0, egui::Slider::new(&mut e.shadow_radius, 0.5..=6.0).text(tx("Shadow reach", "Размер теней")));
                let tf = |v: &str| if v == "bilinear" { tx("Bilinear", "Билинейная") } else { tx("Nearest (as the DS)", "Ближайший (как на DS)") };
                egui::ComboBox::from_label(tx("Texture filtering", "Фильтрация текстур")).selected_text(tf(&e.texture_filter)).show_ui(ui, |ui| {
                    for v in ["nearest", "bilinear"] {
                        ui.selectable_value(&mut e.texture_filter, v.to_string(), tf(v));
                    }
                });
                egui::ComboBox::from_label(tx("3D edge anti-aliasing", "Сглаживание краёв 3D"))
                    .selected_text(match e.msaa { 2 => "2x2".to_string(), 4 => "4x4".to_string(), _ => tx("off", "выкл") })
                    .show_ui(ui, |ui| {
                        ui.selectable_value(&mut e.msaa, 0, tx("off", "выкл"));
                        ui.selectable_value(&mut e.msaa, 2, "2x2");
                        ui.selectable_value(&mut e.msaa, 4, "4x4");
                    });
                if e.msaa > 0 {
                    let used = crate::effects::effective_msaa(video_res, e.msaa, s.video.renderer == "gpu");
                    if used != e.msaa {
                        ui.colored_label(egui::Color32::from_rgb(230, 160, 40), if used == 0 {
                            tx("too heavy at this 3D resolution: not drawn (lower the 3D resolution or use 2x2)", "слишком тяжело при таком разрешении 3D: не включено (уменьшите разрешение 3D или возьмите 2x2)").to_string()
                        } else {
                            format!("{} {}x{}", tx("too heavy at this 3D resolution, in the window:", "слишком тяжело при таком разрешении 3D, в окне:"), used, used)
                        });
                    }
                }
                ui.checkbox(&mut e.fxaa, tx("Smooth edges (FXAA)", "Сглаживание краёв (FXAA)"));
                let sl = |v: &str| match v {
                    "smooth" => tx("Smooth", "Гладкое"),
                    "soft" => tx("Soft", "Мягкое"),
                    _ => tx("Off", "Выкл"),
                };
                egui::ComboBox::from_label(tx("2D smoothing", "Сглаживание 2D")).selected_text(sl(&e.smooth_2d)).show_ui(ui, |ui| {
                    for v in ["off", "smooth", "soft"] {
                        ui.selectable_value(&mut e.smooth_2d, v.to_string(), sl(v));
                    }
                });
                ui.add_enabled(e.smooth_2d != "off", egui::Slider::new(&mut e.smooth_strength, 0.0..=1.0).text(tx("2D smoothing strength", "Сила сглаживания 2D")));
                if *e != before {
                    e.preset = "custom".into();
                }
            });
            egui::CollapsingHeader::new(tx("Sound", "Звук")).default_open(true).show(ui, |ui| {
                ui.add(egui::Slider::new(&mut s.audio.volume, 0.0..=1.0).text(tx("Volume", "Громкость")));
                ui.checkbox(&mut s.audio.mute, tx("Mute", "Без звука"));
            });
            egui::CollapsingHeader::new(tx("Controls", "Управление")).show(ui, |ui| {
                egui::Grid::new("keys").num_columns(3).show(ui, |ui| {
                    for name in NAMES {
                        ui.label(name);
                        ui.label(s.input.keys.get(name).cloned().unwrap_or_default());
                        let waiting = self.rebinding.as_deref() == Some(name);
                        if ui.button(if waiting { tx("press a key…", "нажмите клавишу…") } else { tx("change", "изменить") }).clicked() {
                            self.rebinding = if waiting { None } else { Some(name.to_string()) };
                        }
                        ui.end_row();
                    }
                });
                if ui.button(tx("Default keys", "Клавиши по умолчанию")).clicked() {
                    s.input.keys = config::Input::default().keys;
                    self.rebinding = None;
                }
                egui::ComboBox::from_label(tx("On-screen buttons", "Кнопки на экране"))
                    .selected_text(match s.input.touch_controls.as_str() {
                        "on" => tx("always", "всегда"),
                        "off" => tx("never", "никогда"),
                        _ => tx("on a phone", "на телефоне"),
                    })
                    .show_ui(ui, |ui| {
                        ui.selectable_value(&mut s.input.touch_controls, "auto".to_string(), tx("on a phone", "на телефоне"));
                        ui.selectable_value(&mut s.input.touch_controls, "on".to_string(), tx("always", "всегда"));
                        ui.selectable_value(&mut s.input.touch_controls, "off".to_string(), tx("never", "никогда"));
                    });
                ui.checkbox(&mut s.input.dial_swipe, tx("A swipe turns the dial puzzle's dial", "Свайп вращает диск в головоломке с диском"));
                if s.input.dial_swipe {
                    ui.add(egui::Slider::new(&mut s.input.dial_gain, -4.0..=4.0).text(tx("Dial turn per swipe (negative: the other way)", "Поворот диска за свайп (минус: в другую сторону)")));
                }
                ui.add(egui::Slider::new(&mut s.input.stick_threshold, 0.1..=0.9).text(tx("Stick threshold", "Порог стика")));
                ui.label(tx("Pad: the D-pad and the left stick, the face buttons as the DS's (east A, south B, north X, west Y), shoulders L and R, Start, Select. The mouse on the lower screen is the stylus.", "Геймпад: крестовина и левый стик, кнопки как на DS (восток A, юг B, север X, запад Y), плечевые L и R, Start, Select. Мышь на нижнем экране: стилус."));
            });
            egui::CollapsingHeader::new(tx("Emulation", "Эмуляция")).default_open(true).show(ui, |ui| {
                ui.checkbox(&mut s.emulation.pause_in_menu, tx("Pause while the menu is open", "Пауза, пока открыто меню"));
                ui.checkbox(&mut s.emulation.click_skips_logos, tx("Click skips the studio logos at the start", "Клик пропускает логотипы студий в начале"));
                ui.label(tx("F2 layout, F3 filter, F4/F5 3D resolution, F6 effects, F7 skip, F11 full screen, Tab fast forward, F1 or Esc menu.", "F2 раскладка, F3 фильтр, F4/F5 разрешение 3D, F6 эффекты, F7 пропуск, F11 полный экран, Tab ускорение, F1 или Esc меню."));
            });
            egui::CollapsingHeader::new(tx("Debug", "Отладка")).show(ui, |ui| {
                ui.checkbox(&mut s.video.show_stats, tx("Show speed figures", "Показывать скорость"));
                ui.checkbox(&mut s.emulation.fast_key, tx("Tab fast-forwards while held", "Tab ускоряет, пока зажат"));
                ui.checkbox(&mut s.emulation.fast_button, tx("Fast-forward button beside the menu button (phone, no on-screen buttons)", "Кнопка ускорения рядом с кнопкой меню (телефон, без экранных кнопок)"));
                ui.checkbox(&mut s.emulation.skip_button, tx("Skip button beside them (the screen goes dark and the game races on until it waits for you)", "Кнопка пропуска рядом с ними (экран темнеет, игра мчится вперёд, пока не станет ждать тебя)"));
            });
            egui::CollapsingHeader::new(tx("Profiles", "Профили")).show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.text_edit_singleline(&mut self.profile_name);
                    if ui.button(tx("Save current", "Сохранить текущие")).clicked() {
                        self.message = match config::save_profile(&self.profile_name, s) {
                            Ok(()) => {
                                self.profiles = config::profiles();
                                format!("{} «{}»", tx("profile saved:", "профиль сохранён:"), config::clean_name(&self.profile_name))
                            }
                            Err(e) => format!("{} {}", tx("not saved:", "не сохранён:"), e),
                        };
                    }
                });
                let mut load = None;
                let mut delete = None;
                for p in &self.profiles {
                    ui.horizontal(|ui| {
                        ui.label(p);
                        if ui.button(tx("load", "загрузить")).clicked() {
                            load = Some(p.clone());
                        }
                        if ui.button(tx("delete", "удалить")).clicked() {
                            delete = Some(p.clone());
                        }
                    });
                }
                if let Some(p) = load {
                    self.message = match config::load_profile(&p) {
                        Ok(n) => {
                            *s = n;
                            format!("{} «{}»", tx("profile loaded:", "профиль загружен:"), p)
                        }
                        Err(e) => format!("{} {}", tx("not loaded:", "не загружен:"), e),
                    };
                }
                if let Some(p) = delete {
                    config::delete_profile(&p);
                    self.profiles = config::profiles();
                }
                if !self.message.is_empty() {
                    ui.label(&self.message);
                }
            });
            ui.separator();
            self.save_row(ui, &tx, &mut action);
            ui.horizontal(|ui| {
                if ui.button(tx("Reset all", "Сбросить всё")).clicked() {
                    *s = Settings::default();
                    self.rebinding = None;
                    self.message = tx("settings reset", "настройки сброшены").into();
                }
                if ui.button(tx("Close", "Закрыть")).clicked() {
                    self.open = false;
                }
                let ask = self.restart_ask;
                let label = if ask { tx("Progress not saved in the game is lost. Press again to restart", "Несохранённый прогресс пропадёт. Нажмите ещё раз для перезапуска") } else { tx("Restart the game", "Перезапустить игру") };
                if ui.button(label).clicked() {
                    if ask {
                        action = Some(Action::Restart);
                    }
                    self.restart_ask = !ask;
                }
                if ui.button(tx("Quit", "Выйти из игры")).clicked() {
                    action = Some(Action::Quit);
                }
            });
        });
        if !open {
            self.open = false;
        }
        action
    }
}

/// A layout's name as the menu shows it (the id stays what the settings file holds).
pub fn fit_label(id: &str, lang: Lang) -> &str {
    match (id, lang) {
        ("fit", Lang::En) => "fit, with bars",
        ("integer", Lang::En) => "whole pixels (sharp)",
        ("fill", Lang::En) => "fill, cut the edges",
        ("stretch", Lang::En) => "stretch",
        ("fit", Lang::Ru) => "вписать, с полосами",
        ("integer", Lang::Ru) => "целые пиксели (чётко)",
        ("fill", Lang::Ru) => "заполнить, обрезать края",
        ("stretch", Lang::Ru) => "растянуть",
        (other, _) => other,
    }
}

pub fn layout_label(id: &str, lang: Lang) -> &str {
    match (id, lang) {
        ("stacked", Lang::Ru) => "друг под другом",
        ("side by side", Lang::Ru) => "рядом",
        ("single", Lang::Ru) => "один экран",
        ("overlay", Lang::Ru) => "один экран и панель",
        ("upper only", Lang::Ru) => "только верхний",
        ("lower only", Lang::Ru) => "только нижний",
        (other, _) => other,
    }
}

/// The data files in the folder of `current` that hold texture pictures (a `TEXS` section), by name; kept for two
/// seconds so that the menu does not read the folder at every frame.
fn pack_sets(current: &str) -> Vec<String> {
    use std::cell::RefCell;
    use std::path::{Path, PathBuf};
    use std::time::{Duration, Instant};
    thread_local! {
        static CACHE: RefCell<Option<(PathBuf, Instant, Vec<String>)>> = const { RefCell::new(None) };
    }
    // with no pack chosen ("the game's own pictures") the folder is the one the last pack was in
    let last = CACHE.with(|c| c.borrow().as_ref().map(|(d, _, _)| d.clone()));
    let Some(dir) = Path::new(current).parent().filter(|d| !d.as_os_str().is_empty()).map(Path::to_path_buf).or(last) else { return Vec::new() };
    CACHE.with(|c| {
        let mut c = c.borrow_mut();
        if let Some((d, t, v)) = c.as_ref() {
            if *d == dir && t.elapsed() < Duration::from_secs(2) {
                return v.clone();
            }
        }
        let mut found: Vec<String> = std::fs::read_dir(&dir)
            .into_iter()
            .flatten()
            .flatten()
            .map(|e| e.path())
            .filter(|p| p.extension().is_some_and(|x| x == "data" || x == "spdata") && has_textures(p))
            .map(|p| p.display().to_string())
            .collect();
        found.sort();
        *c = Some((dir, Instant::now(), found.clone()));
        found
    })
}

/// Whether a data file has a texture section: its header lists the sections (`tools/ps2/spdata.py`).
fn has_textures(path: &std::path::Path) -> bool {
    use std::io::Read;
    let Ok(mut f) = std::fs::File::open(path) else { return false };
    let mut head = [0u8; 12];
    if f.read_exact(&mut head).is_err() || &head[..6] != b"SPDATA" {
        return false;
    }
    let n = u32::from_le_bytes([head[8], head[9], head[10], head[11]]).min(64) as usize;
    let mut table = vec![0u8; n * 20];
    f.read_exact(&mut table).is_ok() && table.chunks_exact(20).any(|t| &t[..4] == b"TEXS")
}

#[cfg(test)]
mod pack_tests {
    use super::*;

    fn data_file(tags: &[&[u8; 4]]) -> std::path::PathBuf {
        let mut b = b"SPDATA\0\x01".to_vec();
        b.extend((tags.len() as u32).to_le_bytes());
        for t in tags {
            b.extend(*t);
            b.extend([0u8; 16]);
        }
        let p = std::env::temp_dir().join(format!("sunpetal-test-{}-{}.data", std::process::id(), tags.len() * 7 + tags[0][0] as usize));
        std::fs::write(&p, b).unwrap();
        p
    }

    #[test]
    fn only_a_data_file_with_textures_is_a_texture_set() {
        let with = data_file(&[b"META", b"TEXS", b"TRAN"]);
        let without = data_file(&[b"META", b"TRAN"]);
        assert!(has_textures(&with));
        assert!(!has_textures(&without));
        assert!(!has_textures(std::path::Path::new("/no/such/file.data")));
        let _ = (std::fs::remove_file(with), std::fs::remove_file(without));
    }
}

#[cfg(test)]
mod short_tests {
    use super::*;

    #[test]
    fn the_short_menu_holds_the_five_blocks_the_author_chose() {
        assert_eq!(SHORT, [Block::Sound, Block::Screen, Block::Graphics, Block::Saves, Block::Leave]);
    }

    #[test]
    fn the_short_body_has_none_of_the_full_menus_other_blocks() {
        let src = include_str!("ui.rs");
        let a = src.find("fn short_body").unwrap();
        let b = src.find("/// Draws the menu; changes go straight").unwrap();
        let body = &src[a..b];
        for gone in ["Effects", "Debug", "Profiles", "Controls", "Emulation", "Reset all", "texture_pack"] {
            assert!(!body.contains(gone), "{} is for the full menu", gone);
        }
        for kept in ["Volume", "Window size", "Screen layout", "3D resolution", "Dialogue font", "self.save_row", "Exit to the main menu"] {
            assert!(body.contains(kept), "{}", kept);
        }
    }

    #[test]
    fn toggling_forgets_a_half_done_confirmation() {
        let mut m = Menu::new();
        m.main_ask = true;
        m.restart_ask = true;
        m.toggle();
        assert!(!m.main_ask && !m.restart_ask);
    }
}
