# Ресурс: Adapters

DALI-адаптеры контроллера: имя и выключатель приёма команд.

**Границы:** счётчики провода и загрузка шины — [`stats.md`](stats.md) и
[`diagnostics.md`](diagnostics.md); воркер адаптера —
[`../../runtime-modules/dali-worker/README.md`](../../runtime-modules/dali-worker/README.md).
BDD — [`adapters`](../../../../tests/dali2rust-bdd/features/adapters/).

`adapter_id` — число `0..N-1`, назначается при загрузке и совпадает с сегментом пути
всех адаптерных ресурсов. `name` — метка для людей, а не идентичность.

## Маршруты

| Метод | Путь | Назначение |
|---|---|---|
| `GET` | `/api/v1/adapters` | Список |
| `GET` | `/api/v1/adapters/{adapter_id}` | Один адаптер |
| `PATCH` | `/api/v1/adapters/{adapter_id}` | `name`, `enabled` |

## Чтение

- `limits` — постоянные пределы адаптера: 64 виртуальные лампы, 16 групп, 16 сцен.
- `bus_status` — **не наблюдение провода**: `idle` при `enabled: true` и `disabled`
  при `false`. Загрузка и отказы провода — в `stats.dali`.
- `counters` — сводные счётчики команд, таймаутов и ошибок адаптера на момент чтения.
- Нечисловой `adapter_id` — `400 invalid_resource_id`; адаптера с таким номером нет —
  `404 not_found` (так на всех адаптерных маршрутах).

## `PATCH`

Merge-patch; пишутся только `name` (1..64 байта) и `enabled`. Запись с
read-after-write (`AdapterSettingsUpdateCommand` → реестр → `AdapterSettingsChangedEvent`),
ответ — обновлённый адаптер.

- `limits`, `bus_status`, `counters`, `adapter_id` в теле — `422 unsupported_field`;
  неизвестное поле — `400 unknown_field`; пустое или слишком длинное имя — `422
  invalid_value`.

## Выключенный адаптер

`enabled: false` отказывает **любой** команде, которой нужен провод этого адаптера —
target-state, чтения и записи атрибутов, discovery, commissioning, — до `DaliTransport`.
Отказ доставляется так, как отвечает маршрут:

- синхронный маршрут получает `409 conflict`;
- операция с `202` завершается `failed` с `error.code = conflict` и
  `message = adapter_disabled` — причина названа, чтобы её не спутали с
  неотвечающим прибором.

Счётчики при выключении не обнуляются.
