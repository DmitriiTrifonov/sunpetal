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
}

pub struct Menu {
    pub open: bool,
    /// The button whose key the next key press sets.
    pub rebinding: Option<String>,
    profile_name: String,
    profiles: Vec<String>,
    message: String,
}

impl Menu {
    pub fn new() -> Menu {
        Menu { open: false, rebinding: None, profile_name: String::new(), profiles: config::profiles(), message: String::new() }
    }

    pub fn toggle(&mut self) {
        self.open = !self.open;
        self.rebinding = None;
        if self.open {
            self.profiles = config::profiles();
        }
    }

    /// Draws the menu; changes go straight into `s`.
    pub fn show(&mut self, ctx: &egui::Context, s: &mut Settings, layouts: &[&str], internal: (u32, u32)) -> Option<Action> {
        let lang = Lang::resolve(&s.interface.language);
        let tx = move |en: &'static str, ru: &'static str| if lang == Lang::Ru { ru } else { en };
        let mut action = None;
        let mut open = self.open;
        egui::Window::new(tx("Sunpetal: settings", "Sunpetal: настройки")).open(&mut open).resizable(false).collapsible(false).show(ctx, |ui| {
            egui::ComboBox::from_label("Language / Язык")
                .selected_text(match s.interface.language.as_str() {
                    "en" => "English",
                    "ru" => "Русский",
                    _ => tx("system language", "язык системы"),
                })
                .show_ui(ui, |ui| {
                    ui.selectable_value(&mut s.interface.language, "auto".to_string(), tx("system language", "язык системы"));
                    ui.selectable_value(&mut s.interface.language, "en".to_string(), "English");
                    ui.selectable_value(&mut s.interface.language, "ru".to_string(), "Русский");
                });
            egui::CollapsingHeader::new(tx("Video", "Графика")).default_open(true).show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.label(tx("3D resolution:", "Разрешение 3D:"));
                    ui.add(egui::Slider::new(&mut s.video.res, 1..=8).suffix("x"));
                });
                ui.label(format!("{} {}x{}", tx("each screen, pixels:", "каждый экран, точек:"), internal.0, internal.1));
                ui.checkbox(&mut s.video.linear, tx("Smooth when stretched (linear filter)", "Сглаживание при растяжении (линейный фильтр)"));
                egui::ComboBox::from_label(tx("Screen layout", "Раскладка экранов")).selected_text(layout_label(&s.video.layout, lang)).show_ui(ui, |ui| {
                    for l in layouts {
                        ui.selectable_value(&mut s.video.layout, l.to_string(), layout_label(l, lang));
                    }
                });
                egui::ComboBox::from_label(tx("Background", "Подложка")).selected_text(match s.video.background.as_str() {
                    "black" => tx("black", "чёрная"),
                    _ => tx("glow of the picture", "свечение картинки"),
                }).show_ui(ui, |ui| {
                    ui.selectable_value(&mut s.video.background, "glow".to_string(), tx("glow of the picture", "свечение картинки"));
                    ui.selectable_value(&mut s.video.background, "black".to_string(), tx("black", "чёрная"));
                });
                ui.checkbox(&mut s.video.fullscreen, tx("Full screen (F11)", "Полный экран (F11)"));
                ui.horizontal(|ui| {
                    ui.label(tx("Texture pack folder:", "Папка пакета текстур:"));
                    ui.text_edit_singleline(&mut s.video.texture_pack);
                });
                if !s.video.texture_pack.is_empty() && !std::path::Path::new(&s.video.texture_pack).is_dir() {
                    ui.colored_label(egui::Color32::LIGHT_RED, tx("no such folder", "такой папки нет"));
                }
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
                        (_, false) => "custom",
                        (_, true) => "свои настройки",
                    }
                };
                egui::ComboBox::from_label(tx("Preset (F6)", "Пресет (F6)")).selected_text(label(&s.effects.preset)).show_ui(ui, |ui| {
                    for p in crate::effects::PRESETS {
                        if ui.selectable_label(s.effects.preset == p, label(p)).clicked() {
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
            egui::CollapsingHeader::new(tx("Emulation", "Эмуляция")).show(ui, |ui| {
                ui.checkbox(&mut s.emulation.pause_in_menu, tx("Pause while the menu is open", "Пауза, пока открыто меню"));
                ui.label(tx("F2 layout, F3 filter, F4/F5 3D resolution, F6 effects, F11 full screen, Tab fast forward, F1 or Esc menu.", "F2 раскладка, F3 фильтр, F4/F5 разрешение 3D, F6 эффекты, F11 полный экран, Tab ускорение, F1 или Esc меню."));
            });
            egui::CollapsingHeader::new(tx("Debug", "Отладка")).show(ui, |ui| {
                ui.checkbox(&mut s.video.show_stats, tx("Show speed figures", "Показывать скорость"));
                ui.checkbox(&mut s.emulation.fast_button, tx("Fast-forward button beside the menu button (phone, no on-screen buttons)", "Кнопка ускорения рядом с кнопкой меню (телефон, без экранных кнопок)"));
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
            ui.horizontal(|ui| {
                if ui.button(tx("Reset all", "Сбросить всё")).clicked() {
                    *s = Settings::default();
                    self.rebinding = None;
                    self.message = tx("settings reset", "настройки сброшены").into();
                }
                if ui.button(tx("Close", "Закрыть")).clicked() {
                    self.open = false;
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
pub fn layout_label(id: &str, lang: Lang) -> &str {
    match (id, lang) {
        ("stacked", Lang::Ru) => "друг под другом",
        ("side by side", Lang::Ru) => "рядом",
        ("upper only", Lang::Ru) => "только верхний",
        ("lower only", Lang::Ru) => "только нижний",
        (other, _) => other,
    }
}
