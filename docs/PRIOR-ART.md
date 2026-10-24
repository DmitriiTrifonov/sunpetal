# 🔎 Существующие рекомпы DS

> Проект: `Sunpetal (рекомп)` · План: [План](PLAN.md) · Дата: 2026-10-24

Поиск по GitHub и вебу. Звёзды и лицензии на 2026-10-24, **перед использованием проверить файл LICENSE в самом репозитории**.

## Что нашлось

| Проект | Язык | Лицензия | Что это |
|---|---|---|---|
| [RetroPortingToolKit/ndsrecomp](https://github.com/mstan/ndsrecomp) (автор mstan, 62★) | C++ | **PolyForm Noncommercial 1.0.0** (основной `LICENSE`; рядом лежат `LICENSE-MIT` и `LICENSE-GPL` для отдельных частей) | Статический рекомпилятор DS: ARM9 (ARMv5TE) и ARM7 → C, лёгкий рантайм (шина, планировщик двух CPU, железо, SDL). Pre-alpha v0.6.10, без стабильного API. Демо: прошивка DS, SM64DS, Mario Kart DS (Wiimmfi), Metroid Prime Hunters. Есть melonDS-оракул для сверки. Код из RAM пока интерпретируется |
| [vs-sr-dev/ndskit](https://github.com/vs-sr-dev/ndskit) (2★) | Python + C++ | MIT | Набор для DS: разбор картриджа, SDAT, декодер ARM/Thumb, поиск функций, рекомпилятор в C++ (программы и оверлеи), рантайм (оба CPU, 2D, 3D на OpenGL, звук), сверка с сейвстейтами melonDS. Порты: Moon, Nostalgia, Lufia |
| [Tekkra-Pi/rushrecomp](https://github.com/Tekkra-Pi/rushrecomp) | C | нет | Sonic Rush на конвейере ndsrecomp |
| [drj613/JUSDecomp](https://github.com/drj613/JUSDecomp) | — | не определена | Jump Ultimate Stars, играбелен на Windows/Linux |
| [AgitationSkeleton/legobattles-pc](https://github.com/AgitationSkeleton/legobattles-pc) | C++ | GPL-3.0 | LEGO Battles, нативный порт |
| [kelpsyberry/dust](https://github.com/kelpsyberry/dust) (344★) | **Rust** | GPL-3.0 | Эмулятор DS на Rust (десктоп и веб), отладка. Не рекомп, но готовая Rust-реализация железа DS |
| [melonDS](https://github.com/melonDS-emu/melonDS) (5144★) | C++ | GPL-3.0 | Эталон поведения, у нас уже стоит |

Готового рекомпа **Flower, Sun, and Rain** не нашлось.

## Выводы

- Идея проекта рабочая, DS-рекомпы существуют, но они все на C++ (и Python), **на Rust готового нет**. Рантайм на Rust придётся писать самим или опираться на логику `dust` (GPL-3.0!).
- `ndskit` под MIT: можно читать, брать идеи и при необходимости код (с сохранением авторства). Самый полезный образец: поиск функций в коде CodeWarrior, оверлеи, сверка с сейвстейтами melonDS.
- `ndsrecomp` под PolyForm Noncommercial (коммерческое использование запрещено, это не OSI-лицензия): код копировать нельзя, читать и учиться можно. В README сказано, что документация в основном внутренняя.
- Оба проекта подтверждают подход: генерация C из ARM/Thumb, сверка с melonDS, оверлеи как отдельные модули.

## Лицензия Sunpetal

- Если **не берём код** из GPL-проектов (melonDS, dust, legobattles-pc): **Apache-2.0** (выбрано; MIT OR Apache-2.0 тоже подходит), обычная пара для Rust. Совместимо с мобильными сторами и с `ndskit` (MIT).
- Если берём код `dust` или melonDS: придётся **GPL-3.0**. GPL плохо сочетается с App Store (Apple запрещает условия, несовместимые с GPL), на iOS останется только сайдлоад.
- Идеи и форматы (описание железа DS по GBATEK и документации melonDS) копированием кода не считаются, их использовать можно.
- Рекомендация: **MIT OR Apache-2.0**, железо DS писать заново по GBATEK и сверять с melonDS как с оракулом, не копируя его код.
- Лицензия проекта не отменяет требование «ром и данные оригинала не раздаём».

## Что дальше

- [ ] Склонировать `ndskit` и `ndsrecomp` в `~/Developer/games/ref/` (вне репозитория Sunpetal), прочитать README и структуру
- [ ] Попробовать `ndskit` на нашем дампе: `python -m ndskit.rom ... --info` и `discover` (посмотреть, как справляется с нашим ARM9 и оверлеями)
- [ ] Решить: переиспользовать Python-рекомпилятор `ndskit` как прототип, а рантайм на Rust, или писать рекомпилятор на Rust сразу
- [x] Лицензия выбрана: **Apache-2.0** (2026-10-24), файл `LICENSE` в репозитории
- [x] Склонированы `ndskit` и `ndsrecomp` в `~/Developer/games/ref/`, `ndskit` прогнан на нашем дампе: [Разведка ndskit](RECON-NDSKIT.md)
