# Сквозные state-контракты

Общие перечисления и структуры состояния, которые переиспользуют шина, реестр, REST,
WebSocket и мост Home Assistant: из чего они состоят на шине и чем шинная сторона
отличается от JSON.

Границы: сами типы — `dali2rust-contracts::msg` (`kinds.rs`, `state.rs`), они
канонические; JSON-написания, значения перечислений и смысл полей для клиента —
[`../rest-api/contracts/enums.md`](../rest-api/contracts/enums.md) и
[`../rest-api/contracts/state-contracts.md`](../rest-api/contracts/state-contracts.md),
правило написания — [`../rest-api/stability-and-versioning.md`](../rest-api/stability-and-versioning.md);
как реестр сливает runtime —
[`../../architecture/06-registry-and-persistence.md`](../../architecture/06-registry-and-persistence.md);
единицы цвета — [`../glossary-and-invariants.md`](../glossary-and-invariants.md).

## Перечисления: чем шина отличается от JSON

- **`DeviceType`** на шине шире REST: всё, что не DT6 и не DT8, на границе читается как
  `unknown`. Поддерживаемые гиром типы — отдельный набор `DeviceTypeSet`
  ([`dali-attribute-taxonomy.md`](dali-attribute-taxonomy.md)).
- **`ColorMode`** на шине знает `None`; «режим заявляет цвет» — всё, кроме `None` и
  `Unknown`.
- **`RuntimeSource`** выводится из `Origin` команды и на шине знает `Readback` — коммит
  чтения, которое ничего не командовало. Какой факт какой источник получает, решает
  проектор ([`../runtime-modules/state-fanout/README.md`](../runtime-modules/state-fanout/README.md)).
- **`LastDapcSource`**: значения выставляет проектор (там же); `Unknown` сериализуется
  как `null`.
- **Источник override** (`device_type_source`, `color_mode_source`) — только
  JSON-проекция, отдельного Rust-типа нет.

## Структуры

**Уровень** — сырой arc power `0..254`, одно представление яркости на всех
поверхностях ([`state-contracts.md`](../rest-api/contracts/state-contracts.md)
§`LightSetpoint`).

**`LightSetpoint`** — питание, уровень и необязательный цвет. Что значит отсутствующее
поле на входе реестра, почему цвет едет целиком и почему запись держит одно
представление цвета — [06](../../architecture/06-registry-and-persistence.md) §Merge
rules.

**`ColorValue`** — авторитетен только активный режим; `w`/`a`/`f` несёт только
`Rgbwaf`, а `Rgb` гонит их в ноль; цветовая температура `0` — признак отсутствия, а не
значение. Провенанса внутри нет: его несут конверт, `source` записи RRUC и
`value_source` наблюдения.

**`StatusFlags`** — все восемь бит `QUERY STATUS`, названные по переменным 102 §9.16,
плюс сырой байт. `lamp_on` отличает «гир говорит, что лампа горит» от «уровень > 0»:
они расходятся при старте и при отказе лампы. Когда `power_cycle_seen` — доказательство,
а когда нет — [`../../architecture/09-dali-protocol-rules.md`](../../architecture/09-dali-protocol-rules.md)
§Faults and identification.

**`FailureStatus`** — сводка отказов для всех поверхностей (из флагов статуса, отказов
чтения и отсутствия ответа); конфигурацией не является и не персистится.

**`RuntimeObservation`** — статус, отказы, источник значения, `last_seen_ms`, источник
последнего DAPC и ошибка. На входе реестра (`RuntimeRegistryUpdateEntry.observation`)
это частичный отчёт, на выходе (`RuntimeStateChangedEvent`) — снимок записи
([`events.md`](events.md)); правила слияния и единственная хранимая ошибка
(`device_absent`) — [06](../../architecture/06-registry-and-persistence.md) §Merge
rules.

**`CapabilityFlags`** — `brightness`, `cct`, `xy`, `rgb`, `rgbwaf` (больше трёх каналов
RGBWAF), `scenes`, `groups`. Цветовые биты — липкие доказательства из
`QUERY COLOUR TYPE FEATURES`. Capability группы — объединение по привязанным членам; как
с цветом обходятся заявленный человеком режим и групповая проекция —
[06](../../architecture/06-registry-and-persistence.md) §Colour capability gate.

Override типа и цветового режима и их наследование лампой —
[`../rest-api/resources/physical-devices.md`](../rest-api/resources/physical-devices.md)
и [`../rest-api/resources/virtual-lamps.md`](../rest-api/resources/virtual-lamps.md).
