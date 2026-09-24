# Enum'ы JSON-контракта

Enum'ы, которые встречаются в нескольких ресурсах и чьё JSON-написание — публичный
контракт, с тем смыслом значений, который не выводится из имени.

**Границы:** набор значений задают Rust-enum'ы в `dali2rust-contracts::msg` (и
`dali2rust-domain::registry` для проекций реестра) — они канон; JSON-имя даёт их
`rest_name()` / `as_str()`, общее для REST и WebSocket. Enum'ы одного ресурса
описаны в нём самом: HCL — [`../resources/hcl.md`](../resources/hcl.md), шаги и
области commissioning — [`../resources/commissioning.md`](../resources/commissioning.md),
причины переходов роли — [`../resources/redundancy.md`](../resources/redundancy.md),
состояния обновления прошивки — [`../resources/firmware.md`](../resources/firmware.md).
Чисто шинные enum'ы (`Origin`, `DaliTargetScope`, …) — в
[`../../bus-contracts/`](../../bus-contracts/). Правило написания (PascalCase →
snake_case) — [`../stability-and-versioning.md`](../stability-and-versioning.md).

## Тип устройства — `device_type_*`

Продуктовая **классификация** для выбора контролов, а не набор типов прибора.
REST знает три значения: `dt6_led`, `dt8_color`, `unknown`; любой другой тип DALI
читается как `unknown`. `device_type_override` принимает те же три. Полный набор
объявленных прибором типов — числовое поле `supported_device_types`
([`../resources/physical-devices.md`](../resources/physical-devices.md)).

## Цветовой режим — `color_mode`, `color_mode_*`

`brightness` · `cct` · `xy` · `rgb` · `rgbwaf` · `unknown`.

- `brightness` — прибор без цвета: утверждение «цвета нет», а не отсутствие данных.
- `unknown` — режим не наблюдался или не моделируется.
- `rgb` и `rgbwaf` — один тип цвета DALI (RGBWAF); различие продуктовое: три канала
  или шесть — [`state-contracts.md`](state-contracts.md) §Три канала и шесть.

## Питание — `power`

`on` · `off` · `unknown`. `unknown` — наблюдения ещё не было, или чтение не смогло
назвать уровень; что значат `power` и `level` вместе —
[`state-contracts.md`](state-contracts.md) §`LightSetpoint`.

## Происхождение значения — `*_source` и `value_source`

- `device_type_source`, `color_mode_source`: `discovered` (из скана или чтения
  атрибутов) · `manual_override` (оператор задал override на физическом устройстве;
  виртуальная лампа наследует и значение, и источник).
- `value_source`: `api` · `mqtt` · `hcl` · `rules` · `poller` · `sniffer` · `cluster` ·
  `adapter_proxy`. `poller` — значение прочитано с прибора фоновым опросом; `sniffer`
  — эхо чужой команды с провода. Чтение атрибутов по запросу оператора называет того,
  кто спросил (`api`), хотя на шине коммит такого чтения помечен отдельным источником
  `Readback`, чтобы планировщик HCL не принял его за ручное вмешательство. `cluster` и
  `adapter_proxy` зарезервированы за нереализованными подсистемами.
- `last_dapc_source`: `sniffer` (чужой адресный DAPC/recall) · `scene` (вызов сцены) ·
  `group` (групповой или широковещательный уровень); `null`, пока такого не было.

## Операции — `type`, `status`

`status`: `accepted` → `running` → `succeeded` | `failed` | `timed_out` |
`cancelled`; переходы и смысл каждого статуса —
[`../resources/operations.md`](../resources/operations.md) §Жизненный цикл.

`type`:

| Значение | Что запустило |
|---|---|
| `discovery` | скан control gear или Part 103 устройств |
| `attribute_read` | чтение атрибутов и пресетов банков памяти |
| `attribute_write` | `write-attributes` |
| `memory_bank_read` | не создаётся REST; банки читаются через `attribute_read` |
| `group_apply`, `scene_apply` | apply матриц |
| `config_write` | запись конфигурации: матрицы, HCL, документ правил, конфигурация input devices; также ручной запуск правила |
| `ha_discovery_publish` | повторная публикация HA discovery |
| `commissioning_identify`, `commissioning_address_change`, `commissioning_replace_device` | commissioning control gear; identify и commissioning Part 103 используют те же типы |
| `firmware_update` | обновление прошивки; заканчивается перезагрузкой |
| `policy_apply` | запись политик отказоустойчивости в приборы |

## Исход чтения атрибутов — `attribute_read_outcomes`

По одному значению на группу атрибутов в терминальной операции `attribute_read`:

| Значение | Смысл |
|---|---|
| `not_requested` | группу не просили |
| `success` | секция прочитана (отдельные атрибуты могут честно отсутствовать) |
| `contended_abort` | чтение прервано после бюджета повторов на занятой шине |
| `transport_abort` | прервано ошибкой транспорта |
| `not_attempted` | не начиналось, потому что раньше прервалась другая группа |
| `preempted` | чтение уступило более приоритетной команде оператора — штатно, прерванное не закоммичено |
| `device_absent` | на адресе никто не ответил — прибора нет, транспорт исправен |
| `sequence_incomplete` | последовательность перечисления типов прибора дважды порвана на тихой шине — неисправен гир или невидимый чужой мастер, а не наш провод |

## Группы атрибутов — `attribute_groups`, `attribute_groups_default`

План чтения провода: `runtime_status` · `common_102` · `dt6_led` · `dt8_color` ·
`groups` · `scenes` · `extended` · `scene_colours`. Что читает `scene_colours` и
почему только по запросу —
[`../../bus-contracts/dali-attribute-taxonomy.md`](../../bus-contracts/dali-attribute-taxonomy.md)
§`SceneColours`. Это **не** имена секций ответа `GET …/attributes`: словари
пересекаются по написанию, но различаются по смыслу.

## Пресеты банков памяти — `memory_banks`

`none` (по умолчанию) · `identity` (банк 0 и идентичность банка 1) · `profile` (банки
0 и 1 целиком) · `all` (все объявленные в банке 0) · `power` / `energy` (банки DiiA
Part 252: живая мощность и накопленная энергия) · `diagnostics` (банки Part 253
205/206) · `luminaire_data` (банк 207, константы производителя). Какие банки и зачем
раздельно — [`../resources/physical-devices.md`](../resources/physical-devices.md).

## Режим discovery — `mode`

`scan_known_short_addresses` (опрос всех 64 адресов) · `refresh_known` (только
известные реестру) · `commission_unaddressed` (адресация неадресованных приборов).
