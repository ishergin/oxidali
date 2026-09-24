# WebSocket: типы кадров и полезная нагрузка

Какой тип кадра приходит на какой канал и что он несёт: полное состояние или только
триггер на перечитывание.

**Границы:** протокол, каналы, потери и то, что форма кадра — форма REST, —
[`websocket.md`](websocket.md); формы состояния —
[`../contracts/state-contracts.md`](../contracts/state-contracts.md). Форму полей задаёт
проекция в `dali2rust-api` (`ws/`), а не дамп шинных типов.

## Конверт

`{"type", "channel", "ts_ms", "payload"}`: `type` — имя события шины или серверного
кадра, `ts_ms` — метка события (для снимков — момент выборки), `payload` — см. ниже.

## Проекция: полное состояние или триггер

Воркер WebSocket не держит портов чтения реестра: собрать полный GET DTO значило бы
строить тяжёлые представления на стеке воркера. Поэтому:

- **Полная проекция** — там, где шинное событие уже несёт всё или где перечитывать
  нечего: состояние света, операции, события устройств ввода, активации правил.
- **Тонкий триггер** — для изменений записей: полезная нагрузка — только
  идентификаторы (`adapter_id` и, где применимо, `virtual_lamp_id`, `short_address`,
  `group_id`, `scene_id`; отсутствующие опущены, а не `null`). Клиент дебаунсит и
  перечитывает затронутый ресурс по REST. Такие события редки, и триггеры одной записи,
  не успевшие уйти, сливаются.

## Типы кадров

| `type` | Канал | Полезная нагрузка |
|---|---|---|
| `RuntimeStateChangedEvent` | `virtual_lamps`, `physical_devices` | Полная: идентификаторы плюс `state` = `RuntimeStateContract`. |
| `OperationStatusChangedEvent` | `operations` | Полная, в форме `OperationView` (строковый `operation_id`, `type`, `status`, в терминальном статусе — те же поля, что у REST). |
| `AdapterSettingsChangedEvent` | `adapters` | Триггер. |
| `PhysicalDeviceChangedEvent` | `physical_devices` | Триггер. |
| `VirtualLampChangedEvent` | `virtual_lamps` | Триггер. |
| `GroupChangedEvent`, `GroupMatrixChangedEvent` | `groups` | Триггер. |
| `SceneChangedEvent`, `SceneMatrixChangedEvent` | `scenes` | Триггер. |
| `InputDeviceChangedEvent`, `DaliInputDeviceLifecycleEvent` | `input` | Триггер (адресное пространство устройств ввода, не control gear). |
| `DaliInputEventObservedEvent` | `input` | Полная — см. ниже. |
| `RulesChangedEvent` | `rules` | `revision`, `rule_count`, `lang_id`: документ сменился, перечитать `/rules`. |
| `RulesActivationEvent` | `rules` | `rule_name`, `dry`, `effects`, `partial`, `trigger_to_publish_ms`. |
| `StatsSnapshot` | `stats` | Тело `GET /api/v1/stats`; серверный снимок, а не событие шины. |
| `DiagnosticsSnapshot` | `diagnostics` | Тело `GET /api/v1/diagnostics`. |
| `SnifferBatch` | `sniffer` | Записи кадров (время, направление, ширина, hex, цель, имя команды, запрос ли) и `dropped_since`. |
| `LogBatch` | `logs` | Строки лога (`seq`, время, уровень, модуль, текст) и `dropped_since`. |
| `DropNotice` | любой или `*` | Признание потери — [`websocket.md`](websocket.md). |

`dropped_since` батчей — сколько записей источник потерял до этого кадра (переполнение
кольца или байтовый бюджет батча): отставание клиента выражается одним счётчиком, а не
двумя способами.

## `DaliInputEventObservedEvent`

Нажатие — **происшествие**, а не состояние: «триггер + перечитать» для него не работает,
перечитывать нечего. Поэтому проекция полная, и два нажатия никогда не сливаются в
одно: ключ слияния несёт монотонную метку происшествия. Кадр можно потерять (это видно
в `dropped_since` / `DropNotice`), но нельзя молча склеить с другим.

- `scheme` — схема событий устройства, и это **диагноз**: при схеме 0 событие не несёт
  идентичности устройства (`short_address: null`), и ни одно правило по конкретной
  панели под ней не сработает.
- `event` — продуктовое имя события (то же написание, что в REST, Home Assistant и языке
  правил), `null` для немоделированного типа; тогда клиент показывает сырые десять бит
  `event_info`. `value` — типизированная величина (код кнопки, битовое поле
  присутствия, сырое значение датчика), `0` при `event: null`.
