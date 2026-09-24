# Таксономия атрибутов DALI

Как атрибуты control gear делятся на классы, какая группа чтения их наполняет, что из
них записывается и что переживает перезагрузку. Нужна, чтобы не смешивать метаданные
оператора, доказательства с провода и volatile runtime.

Границы: форма секций в REST — [`../rest-api/resources/physical-devices.md`](../rest-api/resources/physical-devices.md);
команды чтения и записи — [`semantic-dali-commands.md`](semantic-dali-commands.md);
состав слайсов — [`snapshots.md`](snapshots.md); атрибуты устройств ввода Part 103 —
[`../runtime-modules/input-devices/README.md`](../runtime-modules/input-devices/README.md).
Перечень полей каждой секции — view-типы в `dali2rust-domain::registry`
(`declare_attribute_sections!`).

## Четыре класса

| Класс | Где | Кто меняет | Переживает перезагрузку |
|---|---|---|---|
| Runtime | `state.*`: питание, уровень, цвет, флаги статуса, сводка отказов, `last_seen_ms`, источник значения, источник последнего DAPC, ошибка отсутствия | только `RegistryRuntimeUpdateCommand` от проектора | нет |
| Доказательства | `attributes.*`, набор типов устройства, биты capability, Tc-диапазон, random address | чтение атрибутов, discovery, readback записи — реестр напрямую | да, кроме секций вне `attributes` (ниже) |
| Записываемая конфигурация | подмножество листьев доказательств | `DaliWriteAttributesCommand`; членство и сцены — своими семействами | да (как доказательство) |
| Метаданные оператора | имя, заметки, `device_type_override` / `color_mode_override`, привязка лампы, гейты Home Assistant | REST PATCH → команды реестра | да |

Правила между классами:

- Обнаруженное и runtime через общий PATCH не пишется: только через свою команду.
- Доказательство липкое: потерянное чтение не стирает capability, набор типов или
  Tc-диапазон — поэтому они и персистятся.
- Заявление человека (override типа и цветового режима) доказательство не стирает;
  как оно подмешивается и что допускает —
  [`../rest-api/resources/physical-devices.md`](../rest-api/resources/physical-devices.md) §`PATCH`.
  Лампа своих заявлений не имеет и наследует всё от привязанного устройства.

## Группы чтения и секции

`DaliReadAttributesCommand` несёт маску групп (`DaliAttributeGroup`, бит = группа) и
пресет банков памяти. Каждая группа — отдельный чанк события, в пределах 128 байт.

| Группа | Что наполняет | Часть IEC | Запись |
|---|---|---|---|
| `runtime_status` | `state.*` через проектор, не `attributes` | 102 (`QUERY STATUS`, `QUERY ACTUAL LEVEL`), DT8-цвет | target-state |
| `common_102` | версия, тип устройства и набор типов, физический минимум, `minLevel`/`maxLevel`, `powerOnLevel`, `systemFailureLevel`, время и скорость фейда, тип источника света | 102 | всё перечисленное, кроме версии, типов и физического минимума |
| `groups` | маска членства устройства | 102 | `DaliProgramGroupMembershipCommand` (apply матрицы) |
| `scenes` | уровни сцен 0..15 | 102 | `DaliProgramSceneCommand` (apply матрицы) |
| `scene_colours` | цвет каждой сцены (ниже) | 209 §9.11.5, §9.12.6 | программирование сцены |
| `dt6_led` | тип и режимы гира, особенности, биты отказов, кривая диммирования, быстрый фейд, расширенная версия | 207 | только кривая диммирования |
| `dt8_color` | тип цвета, значения цвета, байт `GEAR FEATURES/STATUS`, байт `RGBWAF CONTROL` | 209 | Tc-пределы; бит `Automatic Activation` и `RGBWAF CONTROL` утверждает цветовой путь (с разрешения настроек DALI) |
| `extended` | расширенное время фейда, расширенная версия | 102 | расширенное время фейда |

Пресеты банков памяти (`MemoryBankReadPreset`) наполняют секции `memory_*`:

| Секция | Банк | Персист |
|---|---|---|
| `memory_identity` | банк 0 (GTIN, версии, число логических блоков) | да |
| `memory_profile` | банк 1 (OEM-идентичность, байт блокировки) | да |
| `memory_bus_unit` | банк 0 выше `0x1A` | нет |
| `memory_luminaire` | расширение банка 1 по DiiA Part 251 | нет |
| `memory_energy` | банки 202-204 (Part 252: мощность и энергия) | нет |
| `memory_diagnostics` | банки 205-207 (Part 253) | нет |

Секции без персиста — живые измерения или данные, которые восстанавливает одно чтение;
почему они живут вне персистентного вида и что с ними после перезагрузки —
[`../../architecture/06-registry-and-persistence.md`](../../architecture/06-registry-and-persistence.md).
Сырые байты банков не персистятся вовсе — только разобранные значения и сводка
диапазонов. Энергию и диагностику по расписанию читает поллер
([`../runtime-modules/poller/README.md`](../runtime-modules/poller/README.md)).

## `SceneColours`

Хранимый цвет сцены читается через регистры REPORT DT8, **без recall сцены**.

- Только по запросу: полное чтение — шестнадцать транзакций и секунды провода, поэтому
  поллер этот бит не ставит никогда.
- Один чанк на сцену: ответ `QUERY SCENE LEVEL`, тип цвета и значения этого типа
  (Tc в миредах, xy, шесть каналов RGBWAF; primary N записывается, но не оценивается).
- `None` — вопрос остался без ответа, ничего не утверждается и сохранённое остаётся.
  Отвеченный MASK — утверждение: сцена цвета не хранит.
- Те же чанки публикует проверка после программирования сцены, так что оба
  производителя питают одну проекцию.
- Реестр держит наблюдение volatile, рядом с уровнями сцен. Матрица сцены выводит из
  него `applied`-цвет строки, а для строк, чей цвет не читали, берёт эхо записанного;
  сходимость сравнивается в пространстве провода
  ([09 §DT8](../../architecture/09-dali-protocol-rules.md#dt8-colour-part-209)).

## Смыслы, которые легко спутать

- **Три «extended»**: IEC extended fade (`attributes.extended`) ≠ расширенная команда
  (опкоды DT6/DT8 после `ENABLE DEVICE TYPE`) ≠ extended data в банках памяти.
- **Тип и набор типов**: `device_type_discovered` — классификация продукта
  ([`../rest-api/contracts/enums.md`](../rest-api/contracts/enums.md)), набор типов —
  то, что гир поддерживает
  ([09 §Device types](../../architecture/09-dali-protocol-rules.md#device-types-and-memory-banks)).
- **Байты DT8**: записываемый 243 ≠ читаемый 247, ширина значений цвета, sRGB против
  линейных dim level — [09 §DT8](../../architecture/09-dali-protocol-rules.md#dt8-colour-part-209),
  [`ADR-026`](../../architecture/decisions/ADR-026-dt8-colour-activation.md),
  [`ADR-022`](../../architecture/decisions/ADR-022-rgbwaf-channels-are-srgb.md).

## Не в продукте

- Конфигурация DT6 кроме кривой и Part 218 (DT17) —
  [`../../reference/iec62386-conformance-gaps.md`](../../reference/iec62386-conformance-gaps.md)
  §5, §9.2.
- DT8: статус и ограничения цвета, конфигурация primary N, чтение temporary-значений
  как атрибутов.
- DT1-DT5 и DT7 своей секции не имеют — только `common_102` и runtime.
- Флаги `QUERY STATUS` отдельными листьями не выводятся — они внутри `state.status`.
