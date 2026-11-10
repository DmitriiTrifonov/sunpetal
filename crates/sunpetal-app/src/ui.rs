//! The settings menu, drawn over the game with egui (Esc or Start: the short one; F1: the full one). The text is
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
    /// Ask the Android launcher to show the file picker the next time the app starts.
    PickGame,
    /// A copy of the game's save to this file (made by the file dialog, which iOS has not).
    #[cfg_attr(target_os = "ios", allow(dead_code))]
    ExportSave(std::path::PathBuf),
    /// This file replaces the game's save, and the game starts anew.
    #[cfg_attr(target_os = "ios", allow(dead_code))]
    ImportSave(std::path::PathBuf),
}

pub struct Menu {
    pub open: bool,
    /// The button whose key the next key press sets.
    pub rebinding: Option<String>,
    profile_name: String,
    /// The widget that had the focus last frame: a move of it scrolls the menu to it.
    last_focus: Option<egui::Id>,
    /// The game's own blips for the menu (from the ROM) and where to play them; none: silent.
    pub sound: Option<(std::sync::Arc<crate::audio::Ring>, crate::uisound::Cues)>,
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
    /// The 2D pack's groups of variant pictures and their variants (set by the window from the emulation).
    pub pack_groups: std::collections::BTreeMap<String, Vec<String>>,
    /// The game's own pictures instead of the packs (F12; the window copies it from and back to the emulation).
    pub originals: bool,
    /// Debug: the 2D dumps (the window copies them from and back to the emulation).
    pub dump_2d_new: bool,
    pub dump_on_touch: bool,
    /// Opened with F1: the full menu for this opening, whatever `[ui] full_menu` says.
    pub forced_full: bool,
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
        Menu {
            open: false,
            rebinding: None,
            profile_name: String::new(),
            last_focus: None,
            sound: None,
            profiles: config::profiles(),
            message: String::new(),
            restart_ask: false,
            import_ask: false,
            font_set: None,
            short: false,
            main_ask: false,
            focus_style: false,
            pack_groups: Default::default(),
            originals: false,
            dump_2d_new: false,
            dump_on_touch: false,
            forced_full: false,
        }
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
            self.play(|c| &c.open);
        }
    }

    fn play(&self, pick: impl Fn(&crate::uisound::Cues) -> &std::sync::Arc<crate::uisound::Cue>) {
        if let Some((ring, cues)) = &self.sound {
            ring.play(pick(cues));
        }
    }

    /// The game's save as a file other emulators read (a desktop's file dialog).
    #[cfg(not(any(target_os = "android", target_os = "ios")))]
    fn save_row(&mut self, ui: &mut egui::Ui, tx: &dyn Fn(&'static str, &'static str) -> String, action: &mut Option<Action>) {
        ui.horizontal(|ui| {
            if ui.button(tx("Export the save…", "Экспорт сохранения…")).clicked() {
                if let Some(to) = rfd::FileDialog::new().set_title("Sunpetal").set_file_name("sunpetal.sav").add_filter("save", &["sav"]).save_file() {
                    *action = Some(Action::ExportSave(to));
                }
            }
            let ask = self.import_ask;
            let label = if ask {
                tx(
                    "The save is replaced (the old one is kept) and the game restarts. Press again to choose the file",
                    "Импорт заменит сохранение (старое останется копией), игра перезапустится. Нажмите ещё раз и выберите файл",
                )
            } else {
                tx("Import a save…", "Импорт сохранения…")
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
        // no dialog, for the pad: the config folder is where the file goes and where it is taken from
        ui.horizontal_wrapped(|ui| {
            if ui.button(tx("Export to the config folder", "Экспорт в папку настроек")).clicked() {
                *action = Some(Action::ExportSave(config::export_file()));
            }
            for f in config::save_files() {
                let name = f.file_name().map_or(String::new(), |n| n.to_string_lossy().into_owned());
                if ui.button(format!("{} {name}", tx("Import", "Импорт"))).clicked() {
                    if self.import_ask {
                        *action = Some(Action::ImportSave(f));
                    }
                    self.import_ask = !self.import_ask;
                }
            }
        });
    }

    /// A choice per group of the 2D pack's variant pictures (the dial's digits: which font), and the game's own art.
    fn variant_rows(&mut self, ui: &mut egui::Ui, s: &mut Settings, tx: &dyn Fn(&'static str, &'static str) -> String) {
        for (group, names) in &self.pack_groups {
            let label = match group.as_str() {
                "digits" => tx("Device digits", "Цифры устройства"),
                "labels" => tx("Device labels", "Надписи устройства"),
                g => g.to_string(),
            };
            let shown = |v: &str| if v == "original" { tx("original (pixel)", "оригинал (пиксельные)") } else { variant_label(v) };
            let now = s.video.pack_variants.get(group).cloned().unwrap_or_else(|| names.first().cloned().unwrap_or_default());
            let mut pick = now.clone();
            let combo_r = Spin::from_label(label).selected_text(shown(&now)).show(ui);
            arrow_pick(ui, &combo_r, &mut pick, &{
                let mut o = names.clone();
                o.push("original".to_string());
                o
            });
            if pick != now {
                s.video.pack_variants.insert(group.clone(), pick);
            }
        }
    }

    /// The graphics level: the game's own pictures, the packs, or (later) the packs with the PS2 models. `originals` is
    /// what the machine reads (F12 swaps it for the session); the choice is kept in `video.graphics`.
    fn graphics_row(&mut self, ui: &mut egui::Ui, s: &mut Settings, tx: &dyn Fn(&'static str, &'static str) -> String) {
        let now = if self.originals {
            "standard"
        } else if s.video.graphics == "ultimate" {
            "ultimate"
        } else {
            "enhanced"
        };
        let name = |v: &str| match v {
            "standard" => tx("Standard DS", "Стандартный DS"),
            "ultimate" => tx("Ultimate", "Ультимативный"),
            _ => tx("Enhanced", "Улучшенный"),
        };
        let mut pick = now.to_string();
        let combo_r = Spin::from_label(tx("Graphics (F12: standard / enhanced)", "Графика (F12: стандартный / улучшенный)")).selected_text(name(now)).show(ui);
        arrow_pick(ui, &combo_r, &mut pick, &["standard".to_string(), "enhanced".to_string()]);
        if pick != now {
            self.originals = pick == "standard";
            s.video.graphics = pick;
        }
    }

    #[cfg(any(target_os = "android", target_os = "ios"))]
    fn save_row(&mut self, _: &mut egui::Ui, _: &dyn Fn(&'static str, &'static str) -> String, _: &mut Option<Action>) {}

    /// The short menu: sound, screen and window, picture, saves and restart, leaving.
    fn short_body(
        &mut self,
        ui: &mut egui::Ui,
        s: &mut Settings,
        layouts: &[&str],
        internal: (u32, u32),
        tx: &dyn Fn(&'static str, &'static str) -> String,
        action: &mut Option<Action>,
    ) {
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
                    let combo_r = Spin::from_label(tx("Screen layout", "Раскладка экранов")).selected_text(layout_label(&s.video.layout, lang).to_string()).show(ui);
                    arrow_pick(ui, &combo_r, &mut s.video.layout, &layouts.iter().map(|l| l.to_string()).collect::<Vec<_>>());
                    const SIZES: [[u32; 2]; 8] = [[512, 384], [768, 576], [1024, 768], [1280, 720], [1600, 900], [1920, 1080], [2560, 1440], [512, 768]];
                    let own = tx("own (dragged)", "своё (по перетаскиванию)");
                    let now = SIZES.iter().find(|z| **z == s.video.window).map_or(own, |z| format!("{} × {}", z[0], z[1]));
                    let combo_r = Spin::from_label(tx("Window size", "Размер окна")).selected_text(now).show(ui);
                    arrow_pick(ui, &combo_r, &mut s.video.window, &SIZES);
                    ui.checkbox(&mut s.video.fullscreen, tx("Full screen (F11)", "Полный экран (F11)"));
                }
                Block::Graphics => {
                    ui.heading(tx("Picture", "Картинка"));
                    ui.horizontal(|ui| {
                        ui.label(tx("3D resolution:", "Разрешение 3D:"));
                        ui.add(egui::Slider::new(&mut s.video.res, 1..=8).suffix("x"));
                    });
                    ui.label(format!("{} {}x{}", tx("each screen, pixels:", "каждый экран, точек:"), internal.0, internal.1));
                    let combo_r = Spin::from_label(tx("Dialogue font", "Шрифт реплик"))
                        .selected_text(match s.video.dialogue_font.as_str() {
                            "rubik" => "Rubik",
                            "ubuntu" => "Ubuntu",
                            "noto" => "Noto Sans",
                            _ => "Nunito",
                        })
                        .show(ui);
                    arrow_pick(ui, &combo_r, &mut s.video.dialogue_font, &["nunito".to_string(), "noto".to_string(), "rubik".to_string(), "ubuntu".to_string()]);
                    ui.checkbox(&mut s.video.linear, tx("Smooth when stretched (linear filter)", "Сглаживание при растяжении (линейный фильтр)"));
                    self.variant_rows(ui, s, tx);
                    self.graphics_row(ui, s, tx);
                }
                Block::Saves => {
                    ui.heading(tx("Saves", "Сохранения"));
                    self.save_row(ui, tx, action);
                    let ask = self.restart_ask;
                    let label = if ask {
                        tx("Progress not saved in the game is lost. Press again to restart", "Несохранённый прогресс пропадёт. Нажмите ещё раз для перезапуска")
                    } else {
                        tx("Restart the game", "Перезапустить игру")
                    };
                    if ui.button(label).clicked() {
                        if ask {
                            *action = Some(Action::Restart);
                        }
                        self.restart_ask = !ask;
                    }
                    #[cfg(target_os = "android")]
                    if ui.button(tx("Change the game file…", "Сменить файл игры…")).clicked() {
                        *action = Some(Action::PickGame);
                    }
                }
                Block::Leave => {
                    ui.separator();
                    // The machine is the recompiled program with its own threads: leaving to the title is the program started anew.
                    let ask = self.main_ask;
                    let label = if ask {
                        tx(
                            "Progress not saved in the game is lost. Press again to go to the main menu",
                            "Несохранённый прогресс пропадёт. Нажмите ещё раз, чтобы выйти в главное меню",
                        )
                    } else {
                        tx("Exit to the main menu", "Выйти в главное меню")
                    };
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
                    // the full menu for this opening, as F1 gives it (the window takes `forced_full` into `short` each frame)
                    if ui.button(tx("All settings…", "Все настройки…")).clicked() {
                        self.forced_full = true;
                    }
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
            crate::theme::apply(ctx);
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
        // the whole window, not a window in the window: a column of at most 720 points in the middle
        let screen = ctx.content_rect();
        ctx.layer_painter(egui::LayerId::new(egui::Order::Background, egui::Id::new("sunpetal-menu-back"))).rect_filled(screen, 0.0, crate::theme::BACK);
        // bigger than a desktop window's controls: readable from a sofa and under a thumb
        let zoom = (screen.height() / 540.0).clamp(1.4, 2.4);
        let width = (screen.width() - 24.0).clamp(120.0, 720.0 * zoom / 1.4);
        let top_left = egui::pos2(screen.center().x - width / 2.0, screen.min.y + 12.0);
        egui::Area::new(egui::Id::new("sunpetal-menu")).order(egui::Order::Middle).fixed_pos(top_left).show(ctx, |ui| {
            crate::theme::zoom_style(ui.style_mut(), zoom);
            ui.set_width(width);
            ui.set_height((screen.height() - 24.0).max(60.0));
            ui.horizontal(|ui| {
                ui.heading(tx("Sunpetal: settings", "Sunpetal: настройки"));
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui.button(tx("Close", "Закрыть")).clicked() {
                        open = false;
                    }
                });
            });
            ui.separator();
            egui::ScrollArea::vertical().auto_shrink([false, false]).max_height((screen.max.y - ui.cursor().min.y - 12.0).max(60.0)).show(ui, |ui| {
            let focus = ctx.memory(|m| m.focused());
            if ctx.input(|i| i.key_pressed(egui::Key::Enter)) && focus.is_some() {
                self.play(|c| &c.select);
            }
            if focus != self.last_focus {
                // the pad moved the focus: the list follows it (the least scroll that shows the widget)
                if let Some(r) = focus.and_then(|id| ctx.read_response(id)) {
                    ui.scroll_to_rect(r.rect.expand(8.0), None);
                }
                if focus.is_some() {
                    self.play(|c| &c.cursor);
                }
                self.last_focus = focus;
            }
            if self.short {
                self.short_body(ui, s, layouts, internal, &tx, &mut action);
                return;
            }
            let combo_r = Spin::from_label("Language / Язык")
                .selected_text(match s.interface.language.as_str() {
                    "en" => "English".to_string(),
                    "ru" => "Русский".to_string(),
                    "auto" => tx("system language", "язык системы"),
                    other => other.to_string(),
                })
                .show(ui);
arrow_pick(ui, &combo_r, &mut s.interface.language, &{ let mut o: Vec<String> = ["auto", "en", "ru"].iter().map(|x| x.to_string()).collect(); o.extend(custom.iter().filter(|c| !matches!(c.as_str(), "en" | "ru")).cloned()); o });
            egui::CollapsingHeader::new(tx("Video", "Графика")).default_open(true).show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.label(tx("3D resolution:", "Разрешение 3D:"));
                    ui.add(egui::Slider::new(&mut s.video.res, 1..=8).suffix("x"));
                });
                ui.label(format!("{} {}x{}", tx("each screen, pixels:", "каждый экран, точек:"), internal.0, internal.1));
                let rl = |v: &str| if v == "gpu" { tx("Graphics card (faster at high resolution)", "Видеокарта (быстрее при высоком разрешении)") } else { tx("Processor (the reference)", "Процессор (эталон)") };
                let combo_r = Spin::from_label(tx("3D drawn by", "3D рисует")).selected_text(rl(&s.video.renderer)).show(ui);
arrow_pick(ui, &combo_r, &mut s.video.renderer, &["software".to_string(), "gpu".to_string()]);
                ui.checkbox(&mut s.video.linear, tx("Smooth when stretched (linear filter)", "Сглаживание при растяжении (линейный фильтр)"));
                let combo_r = Spin::from_label(tx("Screen layout", "Раскладка экранов")).selected_text(tl(layout_label(&s.video.layout, lang))).show(ui);
arrow_pick(ui, &combo_r, &mut s.video.layout, &layouts.iter().map(|l| l.to_string()).collect::<Vec<_>>());
                let combo_r = Spin::from_label(tx("Picture in the window", "Картинка в окне")).selected_text(tl(fit_label(&s.video.fit, lang))).show(ui);
arrow_pick(ui, &combo_r, &mut s.video.fit, &["fit".to_string(), "integer".to_string(), "fill".to_string(), "stretch".to_string()]);
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
                let combo_r = Spin::from_label(tx("Dialogue text", "Текст реплик")).selected_text(match s.video.dialogue_text.as_str() {
                    "pixel" => tx("pixel (the game's)", "пиксельный (как в игре)"),
                    "crisp" => tx("crisp", "чёткий"),
                    _ => tx("auto (crisp from 2x)", "авто (чёткий от 2x)"),
                }).show(ui);
arrow_pick(ui, &combo_r, &mut s.video.dialogue_text, &["auto".to_string(), "crisp".to_string(), "pixel".to_string()]);
                let combo_r = Spin::from_label(tx("Dialogue font", "Шрифт реплик")).selected_text(match s.video.dialogue_font.as_str() {
                    "rubik" => "Rubik",
                    "ubuntu" => "Ubuntu",
                    "noto" => "Noto Sans",
                    _ => "Nunito",
                }).show(ui);
arrow_pick(ui, &combo_r, &mut s.video.dialogue_font, &["nunito".to_string(), "noto".to_string(), "rubik".to_string(), "ubuntu".to_string()]);
                self.variant_rows(ui, s, &tx);
                ui.checkbox(&mut s.video.follow_shape, tx("Window follows the picture's shape", "Окно по форме картинки"));
                ui.checkbox(&mut s.video.logos_in_turn, tx("Logos in turn when one screen is shown", "Логотипы по очереди, когда виден один экран"));
                if matches!(s.video.layout.as_str(), "single" | "overlay") && s.video.auto {
                    let combo_r = Spin::from_label(tx("Dial puzzle (both screens)", "Головоломка с диском (оба экрана)")).selected_text(match s.video.puzzle.as_str() {
                        "side" => tx("side by side", "рядом"),
                        _ => tx("lower screen as a panel (PS2)", "нижний экран панелью (как на PS2)"),
                    }).show(ui);
arrow_pick(ui, &combo_r, &mut s.video.puzzle, &["inset".to_string(), "side".to_string()]);
                }
                if s.video.layout == "overlay" {
                    ui.checkbox(&mut s.video.panel, tx("Lower screen as a panel (F8)", "Нижний экран панелью (F8)"));
                }
                let combo_r = Spin::from_label(tx("Background", "Подложка")).selected_text(match s.video.background.as_str() {
                    "black" => tx("black", "чёрная"),
                    "glow" => tx("glow of the picture", "свечение картинки"),
                    _ => tx("the picture's edge rows run on", "строки края картинки"),
                }).show(ui);
arrow_pick(ui, &combo_r, &mut s.video.background, &["edge".to_string(), "glow".to_string(), "black".to_string()]);
                ui.checkbox(&mut s.video.wide, tx("Widen the 3D view to the window (one screen)", "Расширять 3D под окно (один экран)"));
                if s.video.wide {
                    // how wide the 3D may get: the window's shape, or at most one of the common ones (bars take the rest)
                    const LIMITS: [(f32, &str, &str); 4] = [(0.0, "as the window", "по окну"), (16.0 / 10.0, "16:10", "16:10"), (16.0 / 9.0, "16:9", "16:9"), (21.0 / 9.0, "21:9", "21:9")];
                    let now = LIMITS.iter().find(|l| (l.0 - s.video.wide_limit).abs() < 0.01).map_or_else(|| format!("{:.2}", s.video.wide_limit), |l| tx(l.1, l.2).to_string());
                    let combo_r = Spin::from_label(tx("Widest 3D view", "Макс. ширина 3D")).selected_text(now).show(ui);
arrow_pick(ui, &combo_r, &mut s.video.wide_limit, &[0.0, 16.0 / 10.0, 16.0 / 9.0, 21.0 / 9.0]);
                }
                ui.checkbox(&mut s.video.bars_match, tx("Bars take the picture's edge colour (logos)", "Полосы цвета края картинки (логотипы)"));
                // window sizes (logical pixels); the window is also free to drag, and the menu then shows "own"
                const SIZES: [[u32; 2]; 8] = [[512, 384], [768, 576], [1024, 768], [1280, 720], [1600, 900], [1920, 1080], [2560, 1440], [512, 768]];
                let own = tx("own (dragged)", "своё (по перетаскиванию)").to_string();
                let now = SIZES.iter().find(|z| **z == s.video.window).map_or(own, |z| format!("{} × {}", z[0], z[1]));
                let combo_r = Spin::from_label(tx("Window size", "Размер окна")).selected_text(now).show(ui);
arrow_pick(ui, &combo_r, &mut s.video.window, &SIZES);
                ui.checkbox(&mut s.video.fullscreen, tx("Full screen (F11)", "Полный экран (F11)"));
                // the texture sets beside the file in use (data files with pictures), switched at once
                {
                    let sets = pack_sets(&s.video.texture_pack);
                    let name = |p: &str| std::path::Path::new(p).file_stem().map_or(String::new(), |n| n.to_string_lossy().into_owned());
                    let shown = if s.video.texture_pack.is_empty() { tx("the game's own pictures", "оригинал (без подмены)").to_string() } else { name(&s.video.texture_pack) };
                    let combo_r = Spin::from_label(tx("Texture set (the text and the language stay with the start)", "Набор текстур (текст и язык меняются при запуске)")).selected_text(shown).show(ui);
arrow_pick(ui, &combo_r, &mut s.video.texture_pack, &{ let mut o = vec![String::new()]; o.extend(sets.iter().cloned()); o });
                }
                ui.horizontal(|ui| {
                    ui.label(tx("Data file or texture folder:", "Файл данных или папка текстур:"));
                    ui.text_edit_singleline(&mut s.video.texture_pack);
                });
                if !s.video.texture_pack.is_empty() && !std::path::Path::new(&s.video.texture_pack).exists() {
                    ui.colored_label(egui::Color32::LIGHT_RED, tx("not found", "не найдено"));
                }
                ui.horizontal(|ui| {
                    ui.label(tx("2D pictures (folder or data file, empty: from the data file):", "2D-картинки (папка или файл, пусто: из файла данных):"));
                    ui.text_edit_singleline(&mut s.video.pack_2d);
                });
                if !s.video.pack_2d.is_empty() && !std::path::Path::new(&s.video.pack_2d).exists() {
                    ui.colored_label(egui::Color32::LIGHT_RED, tx("not found", "не найдено"));
                }
                let combo_r = Spin::from_label(tx("Game language (needs the data file)", "Язык игры (нужен файл данных)")).selected_text(match s.game.language.as_str() {
                    "ru" => "Русский".to_string(),
                    _ => tx("the ROM's own", "как в ROM"),
                }).show(ui);
arrow_pick(ui, &combo_r, &mut s.game.language, &["".to_string(), "ru".to_string()]);
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
                let combo_r = Spin::from_label(tx("Preset (F6)", "Пресет (F6)")).selected_text(label(&s.effects.preset)).show(ui);
{ let mut cur = s.effects.preset.clone(); let order: Vec<String> = crate::effects::PRESETS.iter().map(|p| p.to_string()).collect(); arrow_pick(ui, &combo_r, &mut cur, &order); if cur != s.effects.preset { s.effects = crate::effects::Effects::preset(&cur); } }
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
                let combo_r = Spin::from_label(tx("Texture filtering", "Фильтрация текстур")).selected_text(tf(&e.texture_filter)).show(ui);
arrow_pick(ui, &combo_r, &mut e.texture_filter, &["nearest".to_string(), "bilinear".to_string()]);
                let combo_r = Spin::from_label(tx("3D edge anti-aliasing", "Сглаживание краёв 3D"))
                    .selected_text(match e.msaa { 2 => "2x2".to_string(), 4 => "4x4".to_string(), _ => tx("off", "выкл") })
                    .show(ui);
arrow_pick(ui, &combo_r, &mut e.msaa, &[0, 2, 4]);
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
                let combo_r = Spin::from_label(tx("2D smoothing", "Сглаживание 2D")).selected_text(sl(&e.smooth_2d)).show(ui);
arrow_pick(ui, &combo_r, &mut e.smooth_2d, &["off".to_string(), "smooth".to_string(), "soft".to_string()]);
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
                let combo_r = Spin::from_label(tx("On-screen buttons", "Кнопки на экране"))
                    .selected_text(match s.input.touch_controls.as_str() {
                        "on" => tx("always", "всегда"),
                        "off" => tx("never", "никогда"),
                        _ => tx("on a phone", "на телефоне"),
                    })
                    .show(ui);
arrow_pick(ui, &combo_r, &mut s.input.touch_controls, &["auto".to_string(), "on".to_string(), "off".to_string()]);
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
                ui.label(tx("F2 layout, F3 filter, F4/F5 3D resolution, F6 effects, F7 skip, F11 full screen, Tab fast forward, F1 full menu, Esc short menu.", "F2 раскладка, F3 фильтр, F4/F5 разрешение 3D, F6 эффекты, F7 пропуск, F11 полный экран, Tab ускорение, F1 полное меню, Esc короткое."));
            });
            egui::CollapsingHeader::new(tx("Debug", "Отладка")).show(ui, |ui| {
                ui.checkbox(&mut s.video.show_stats, tx("Show speed figures", "Показывать скорость"));
                ui.checkbox(&mut self.dump_2d_new, tx("Write each new 2D picture with its key to artifacts/shots/new_2d", "Писать каждую новую 2D-картинку с ключом в artifacts/shots/new_2d"));
                ui.checkbox(&mut self.dump_on_touch, tx("Write the frames after each touch to artifacts/shots/touch_N (the pressed knob)", "Писать кадры после каждого касания в artifacts/shots/touch_N (нажатая ручка)"));
                ui.checkbox(&mut s.emulation.fast_key, tx("Tab fast-forwards while held", "Tab ускоряет, пока зажат"));
                ui.checkbox(&mut s.emulation.fast_button, tx("Fast-forward button beside the menu button (phone, no on-screen buttons)", "Кнопка ускорения рядом с кнопкой меню (телефон, без экранных кнопок)"));
                ui.checkbox(&mut s.emulation.skip_button, tx("Skip button beside them (the screen goes dark and the game races on until it waits for you)", "Кнопка пропуска рядом с ними (экран темнеет, игра мчится вперёд, пока не станет ждать тебя)"));
            });
            egui::CollapsingHeader::new(tx("Profiles", "Профили")).show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.text_edit_singleline(&mut self.profile_name);
                    if ui.button(tx("Save current", "Сохранить текущие")).clicked() {
                        if config::clean_name(&self.profile_name).is_empty() {
                            self.profile_name = config::next_profile_name(&self.profiles);
                        }
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
                    self.message = tx("settings reset", "настройки сброшены");
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
        });
        crate::theme::paint_cursor(ctx);
        if !open {
            self.open = false;
        }
        action
    }
}

/// A choice among a few values (K-207): `[<] value [>] label`, no popup. The value is the focusable middle; the arrows
/// are clickable marks. A click on an arrow leaves a step in the memory for `arrow_pick`, which owns the order.
pub struct Spin {
    label: String,
    text: String,
}

impl Spin {
    pub fn from_label(label: impl Into<String>) -> Self {
        Self { label: label.into(), text: String::new() }
    }

    pub fn selected_text(mut self, text: impl Into<String>) -> Self {
        self.text = text.into();
        self
    }

    /// The middle's response (the focus and the id `arrow_pick` goes by).
    pub fn show(self, ui: &mut egui::Ui) -> egui::Response {
        let mark = |ui: &mut egui::Ui, text: &str| {
            let size = egui::vec2(ui.spacing().interact_size.y, ui.spacing().interact_size.y);
            let (rect, r) = ui.allocate_exact_size(size, egui::Sense::CLICK);
            let fill = if r.hovered() { crate::theme::TILE_HOT } else { crate::theme::TILE };
            ui.painter().rect_filled(rect, 0.0, fill);
            ui.painter().text(rect.center(), egui::Align2::CENTER_CENTER, text, egui::TextStyle::Button.resolve(ui.style()), crate::theme::ORANGE);
            r
        };
        ui.horizontal(|ui| {
            let left = mark(ui, "<");
            let mid = ui.add(egui::Button::new(self.text).min_size(egui::vec2(ui.spacing().interact_size.x * 4.6, 0.0)));
            let right = mark(ui, ">");
            ui.label(self.label);
            // a click (on the value or a mark) puts the keyboard on the value: the arrows then stay with it
            if mid.clicked() || left.clicked() || right.clicked() {
                mid.request_focus();
            }
            let wheel = if mid.hovered() || left.hovered() || right.hovered() { ui.input(|i| i.smooth_scroll_delta.x) } else { 0.0 };
            let step = right.clicked() as i32 - left.clicked() as i32
                + if wheel < -4.0 {
                    1
                } else if wheel > 4.0 {
                    -1
                } else {
                    0
                };
            if step != 0 {
                ui.data_mut(|d| d.insert_temp(mid.id, step));
            }
            mid
        })
        .inner
    }
}

/// Memory keys: the `Spin` in focus, and the arrow steps `gfx.rs` took out of the input for it.
pub const SPIN_FOCUS: &str = "spin_focus";
pub const SPIN_STEP: &str = "spin_step";

/// A `Spin` moves by the arrows (the pad's left and right, the keys, a click on the marks, a sideways wheel): to the
/// neighbour of the value in `order`, round the ends.
fn arrow_pick<T: PartialEq + Clone>(ui: &egui::Ui, r: &egui::Response, cur: &mut T, order: &[T]) {
    let mut step = ui.data_mut(|d| d.remove_temp::<i32>(r.id)).unwrap_or(0);
    if r.has_focus() {
        // gfx.rs took the arrows out before egui walked the focus with them: they are here
        step += ui.data_mut(|d| d.remove_temp::<i32>(egui::Id::new(SPIN_STEP))).unwrap_or(0);
        ui.data_mut(|d| d.insert_temp(egui::Id::new(SPIN_FOCUS), r.id.value()));
        // the arrows are the widget's now: the focus does not walk off with them
        ui.memory_mut(|m| m.set_focus_lock_filter(r.id, egui::EventFilter { horizontal_arrows: true, ..Default::default() }));
        step += ui.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::ArrowRight) as i32 - i.consume_key(egui::Modifiers::NONE, egui::Key::ArrowLeft) as i32);
    }
    if !r.has_focus() && ui.data(|d| d.get_temp::<u64>(egui::Id::new(SPIN_FOCUS))) == Some(r.id.value()) {
        ui.data_mut(|d| d.remove_temp::<u64>(egui::Id::new(SPIN_FOCUS)));
    }
    if step == 0 {
        return;
    }
    if let Some(v) = order.get(spin_to(order.iter().position(|v| v == cur), order.len(), step)) {
        *cur = v.clone();
    }
}

/// The index a step of `step` from `at` (the value's place, if it is in the list) leads to: round the ends.
fn spin_to(at: Option<usize>, len: usize, step: i32) -> usize {
    (at.unwrap_or(0) as i32 + step.signum()).rem_euclid(len.max(1) as i32) as usize
}

/// A variant's name as the menu shows it: the font names written out, others as they are.
fn variant_label(v: &str) -> String {
    match v {
        "rubik" => "Rubik Black".into(),
        "nunito" => "Nunito Black".into(),
        "philosopher" => "Philosopher".into(),
        "fira" => "Fira Sans".into(),
        _ => v.to_string(),
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
    f.read_exact(&mut table).is_ok() && table.as_chunks::<20>().0.iter().any(|t| &t[..4] == b"TEXS")
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
    fn a_spin_goes_round_the_ends() {
        assert_eq!(spin_to(Some(2), 3, 1), 0);
        assert_eq!(spin_to(Some(0), 3, -1), 2);
        assert_eq!(spin_to(Some(1), 3, 1), 2);
        assert_eq!(spin_to(None, 3, 1), 1);
        assert_eq!(spin_to(Some(0), 1, 1), 0);
    }

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
        for kept in ["Volume", "Window size", "Screen layout", "3D resolution", "Dialogue font", "self.graphics_row", "self.save_row", "Exit to the main menu"] {
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
