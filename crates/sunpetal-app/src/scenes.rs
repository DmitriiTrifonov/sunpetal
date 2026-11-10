//! Таблица сцен из сценариев игры (`docs/native-scenes.csv`, K-163): что видно в тексте сцены без картинки.
//! Генерируется `tools/script/scene_table.py`; сюда попадают только числа и вид сцены, текста реплик нет.
//! Вид (`kind`) — подсказка для политик START и раскладок вместо эвристик по картинке; политика
//! по умолчанию остаётся за `start.rs`, пока автор не проверил виды на сценах.

#![allow(dead_code)] // потребители (политики START и раскладок) подключаются после проверки автором

const CSV: &str = include_str!("../../../docs/native-scenes.csv");

/// Строка таблицы: вид сцены и признаки, нужные политикам.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Scene<'a> {
    pub kind: &'a str,
    pub dialogue: u32,
    pub device: u32,
}

/// Строка для сцены `name` (например `C02_12`), если она есть в таблице.
pub fn scene(name: &str) -> Option<Scene<'static>> {
    CSV.lines().skip(1).find_map(|l| {
        let c: Vec<&str> = l.split(',').collect();
        (c.len() == 12 && c[0] == name).then(|| Scene { kind: c[11], dialogue: c[2].parse().unwrap_or(0), device: c[3].parse().unwrap_or(0) })
    })
}

/// Вид сцены: `shooter`, `movie`, `credits`, `device`, `talk` или `walk`.
pub fn kind(name: &str) -> Option<&'static str> {
    scene(name).map(|s| s.kind)
}

/// Что раскладке делать с видом сцены (решение автора 2026-11-10, K-163). Живая раскладка пока идёт по картинке
/// (`layout.rs`); таблица видов подключается к ней отдельной карточкой.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Policy {
    /// устройство Катарины: верхний экран с предметом, нижний (головоломка) панелью поверх
    Panel,
    /// ролик или титры: показывается один экран, нижний не нужен
    SingleScreen,
    /// разговор, ходьба, стрелялка: как решает картинка (нижний экран по меню и касанию)
    ByPicture,
}

/// Политика раскладки для вида сцены из `kind`.
pub fn policy(kind: &str) -> Policy {
    match kind {
        "device" => Policy::Panel,
        "movie" | "credits" => Policy::SingleScreen,
        _ => Policy::ByPicture,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_row_has_a_known_kind_and_the_scenes_of_the_run_are_in_the_table() {
        let kinds = ["shooter", "movie", "credits", "device", "talk", "walk"];
        let rows: Vec<&str> = CSV.lines().skip(1).collect();
        assert!(rows.len() > 1000);
        assert!(rows.iter().all(|l| l.split(',').count() == 12 && kinds.contains(&l.rsplit(',').next().unwrap())));
        // сцены прогона наблюдателя (K-162): разговор, устройство, титул
        assert_eq!(kind("C12_03"), Some("talk"));
        assert_eq!(kind("C02_12"), Some("device"));
        assert!(kind("C19_01").is_some());
        assert_eq!(kind("C99_99"), None);
    }

    #[test]
    fn the_kinds_get_the_policy_the_author_chose() {
        assert_eq!(policy("device"), Policy::Panel);
        assert_eq!(policy("movie"), Policy::SingleScreen);
        assert_eq!(policy("credits"), Policy::SingleScreen);
        for k in ["talk", "walk", "shooter"] {
            assert_eq!(policy(k), Policy::ByPicture);
        }
        // каждый вид таблицы имеет политику, а устройства и ролики в ней есть
        let rows: Vec<&str> = CSV.lines().skip(1).collect();
        assert!(rows.iter().any(|l| policy(l.rsplit(',').next().unwrap()) == Policy::Panel));
        assert!(rows.iter().any(|l| policy(l.rsplit(',').next().unwrap()) == Policy::SingleScreen));
    }

    #[test]
    fn the_start_table_scenes_are_in_the_data() {
        assert!(crate::start::SCENES.iter().all(|(n, _, _)| kind(n).is_some()));
    }
}
