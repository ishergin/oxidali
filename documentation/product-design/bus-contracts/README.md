# Контракты шины и общего состояния

Смысл сообщений внутренней шины и общих типов состояния: семейства, владельцы,
гарантии доставки и правила, которые не выражены в типах.

Границы: канон формы — Rust-типы `dali2rust-contracts::msg`, объявленные через
`declare_bus_payloads!`; `postcard` — формат кадра и бюджет в 128 байт, а не второй
канон. Механика шины (каналы, ёмкости, маршрутизация, backpressure) —
[`../../architecture/03-bus-and-backpressure.md`](../../architecture/03-bus-and-backpressure.md);
рецепт добавления вида — [`../../architecture/11-extension-recipes.md`](../../architecture/11-extension-recipes.md).

| Документ | О чём |
|---|---|
| [`commands.md`](commands.md) | Семейства команд, владельцы, дисциплины ответа, чанковые записи, runtime-команды реестра, конверт |
| [`events.md`](events.md) | Семейства событий, потребители, best-effort против `publish_required` |
| [`semantic-dali-commands.md`](semantic-dali-commands.md) | Семантические DALI-команды control gear и Part 103, правило продуктового пути |
| [`semantic-dali-coverage-matrix.md`](semantic-dali-coverage-matrix.md) | Какой источник какой DALI-payload вправе публиковать |
| [`shared-state-contracts.md`](shared-state-contracts.md) | Перечисления и структуры состояния, наследование override |
| [`dali-attribute-taxonomy.md`](dali-attribute-taxonomy.md) | Классы атрибутов, группы чтения, секции, персист |
| [`snapshots.md`](snapshots.md) | Read-модели и персистентные слайсы |

Что куда кладётся: транспорт — в конверт, бизнес-данные — в payload
([`commands.md`](commands.md) §Конверт); всё, что больше кадра, — серией со staging'ом
или мимо шины (там же, §Чанковые записи и §Вне шины).
