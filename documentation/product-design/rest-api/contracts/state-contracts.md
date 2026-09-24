# State-контракты

Общие JSON-формы состояния света, которые переиспользуют REST, WebSocket и мост
Home Assistant: что значит каждое поле и какие инварианты держит реестр.

**Границы:** шинная сторона тех же понятий (`LightSetpoint`, `ColorValue`,
`StatusFlags`) — [`../../bus-contracts/shared-state-contracts.md`](../../bus-contracts/shared-state-contracts.md);
как реестр сливает и хранит runtime —
[`../../../architecture/06-registry-and-persistence.md`](../../../architecture/06-registry-and-persistence.md).
Форма полей — Rust DTO (`PhysicalDeviceStateDto`, общий для REST и WebSocket). Здесь —
семантика.

## Дисциплина

- Каждая форма определена один раз; DTO ресурсов собираются композицией.
- DTO живого состояния (виртуальная лампа, физическое устройство, WS, MQTT) несут
  `RuntimeStateContract`. DTO программируемой конфигурации (матрицы групп и сцен) —
  только программируемое подмножество, без runtime-полей.

## `LightSetpoint` — ядро «света»

Питание, уровень и цвет. Встречается в состоянии, в теле target-state и в строке
сцены.

- `power` — `on` / `off` / `unknown`. **Лампа горит, потому что так говорит
  питание**: уровень 0 — это уровень, а не признак выключенности.
- `level` — сырой уровень DALI arc power `0..254`, единственное числовое поле
  яркости в продукте; шкала Home Assistant `brightness 0..255` существует только на
  границе MQTT. `null` — уровень не известен.
- `power: on` без уровня зажигает прибор на его собственном `lastActiveLevel`; уровень
  в состоянии тогда — предсказание реестра, а у лампы, которую ни разу не видели
  горящей, — `null` ([06](../../../architecture/06-registry-and-persistence.md)
  §Merge rules).
- **MASK (255) — не уровень**: чтение, на которое прибор ответил MASK
  ([09](../../../architecture/09-dali-protocol-rules.md) §Reading answers), публикует
  состояние без утверждений — `power: unknown`, без уровня и цвета, — а статусный байт
  как обычно: `lamp_on = false` и есть «света нет».
- Цвет: `color_mode` плюс поле активного режима — `color_temperature_kelvin`, `xy`
  (`0.0..1.0`), `rgb`, для шести каналов — `rgbwaf` на запись и `waf` на чтение.
  Поля неактивных режимов — `null`. Единицы цвета —
  [`../../glossary-and-invariants.md`](../../glossary-and-invariants.md) §Инварианты,
  перевод на границе DALI —
  [`ADR-022`](../../../architecture/decisions/ADR-022-rgbwaf-channels-are-srgb.md).
- Запись только цвета не трогает питание: выключенная лампа остаётся выключенной.

## `RuntimeObservation` — надстройка наблюдения

Добавляется только к DTO живого состояния; в теле записи и в строке сцены её нет.

| Поле | Смысл |
|---|---|
| `status` | Байт `QUERY STATUS`, раскрытый в флаги. Пишется только чтением группы `runtime_status` (вручную или поллером). |
| `failure_status` | Сводка отказа (лампа, гир, связь) с собственным источником. |
| `value_source` | Кто произвёл последний коммит, несущий значение; значения — [`enums.md`](enums.md). |
| `last_seen_ms` | Когда прибор последний раз ответил, в часах контроллера. Возраст считать как `now_ms` ответа минус метка, а не от часов браузера: без SNTP эпоха контроллера своя. |
| `last_dapc_source` | Источник последнего уровня, пришедшего не нашей прямой командой; значения — [`enums.md`](enums.md). |
| `error` | `{code}` без текста. |

Что из правил слияния реестра ([06](../../../architecture/06-registry-and-persistence.md)
§Merge rules) видит клиент:

- `status`, `failure_status`, `last_seen_ms` не стираются чтением, которое их не
  касалось, поэтому `status` отвечает «отвечал ли прибор хоть раз за эту загрузку», а
  не «подтверждено ли текущее значение». Что текущее значение прочитано, а не только
  скомандовано, говорит `value_source` (`poller`); на этом построено кольцо «не
  подтверждено» в UI — [`../../web-ui/lamp-state.md`](../../web-ui/lamp-state.md).
- **В `error` доезжает ровно один код — `device_absent`**; исходы команд
  (`superseded`, `vl_unbound`, …) — ответ вызывающему, в состояние они не пишутся.
  Новый код в `error` требует такого же наблюдаемого пути установки и сброса.

## `RuntimeStateContract` = `LightSetpoint` + `RuntimeObservation`

Плоский объект под ключом `state` в DTO виртуальной лампы и физического устройства
(ядро и строка списка), в событии WS `RuntimeStateChangedEvent` и — после пересчёта
яркости — в state-payload Home Assistant. Непривязанная виртуальная лампа отдаёт
состояние, где всё `null`, кроме `power: unknown` и `color_mode: unknown`.

## Тело target-state

`PUT …/target-state` (виртуальная лампа, группа, физическое устройство):
опциональные поля `LightSetpoint` — `power`, `level`, `color_mode`,
`color_temperature_kelvin`, `xy`, `rgb`, `rgbwaf` — плюс `transition`, который
**принимается и игнорируется**: плавность задаёт fade time самого прибора
(`write-attributes`).

| Нарушение | Ответ |
|---|---|
| Поле `RuntimeObservation`, `included` / `setpoint` строки сцены, `waf` (read-only имя шести каналов) | `422 unsupported_field` |
| Неизвестное поле, в том числе `brightness` и `mired` | `400 unknown_field` |
| `level > 254`, кельвины вне `1000..20000`, `xy` вне `0..1` или `(0, 0)` | `422 invalid_value` |
| Режим, которого нет в effective capabilities цели | `422 unsupported_capability` |
| Неизвестное значение enum'а | `422 invalid_enum` |

## `SceneRow` — строка матрицы сцены

`included` + `LightSetpoint` (или `null`) + derived `capabilities` + derived `dirty`.
В JSON строка несёт `desired` и `applied`; поля setpoint раскрыты внутри них. При
`included = false` каждое поле setpoint — `null`: «сцена не запрограммирована» не
равно ни `power: off`, ни `level: 0`.

- **`applied` гибридный.** `included` и `level` — физический readback
  `QUERY SCENE LEVEL` (MASK ⇒ не участвует). Цвет — readback регистров REPORT DT8,
  если сцена читалась (группа атрибутов `scene_colours` или проверка после
  программирования), иначе эхо последнего успешного программирования. `power` — только
  эхо: на проводе «off» и «уровень 0» в слоте сцены неотличимы.
- `dirty` сравнивает `desired` и `applied` в точности провода: сошедшаяся строка
  равна по построению, округление кельвин↔mirek её не пачкает.
- **Засев `desired := applied`.** Первый readback для строки, которую оператор ещё не
  трогал, засевает `desired.included` / `desired.level` из `applied`; флаг засева
  хранится вместе со слайсом сцен. Засеянная строка чтением больше не
  перезаписывается. Без засева первый `apply` стёр бы уже запрограммированные сцены.
- `capabilities` и `dirty` в теле записи — `422 unsupported_field`.

## Три канала и шесть: `rgb` и `rgbwaf`

- **Запись.** `color_mode: rgb` + `rgb {r,g,b}` — три канала, а белый, янтарный и
  свободный продукт гонит в ноль ([09](../../../architecture/09-dali-protocol-rules.md)
  §DT8 colour). `color_mode: rgbwaf` + `rgbwaf {r,g,b,w,a,f}` — все шесть обязательны.
- **Чтение.** `rgb` несёт три канала в обоих режимах, `waf {w,a,f}` — только при
  `color_mode: rgbwaf`, и только если измерены все шесть.
- **Возможности.** `capabilities.rgb` — каналов не меньше трёх, `capabilities.rgbwaf`
  — больше трёх (из ответа `QUERY COLOUR TYPE FEATURES`).
- Home Assistant получает `rgb` в обоих случаях: в его словаре нет шести каналов.
- Строка сцены подчиняется тому же правилу и программируется всеми шестью каналами.
