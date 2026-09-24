# Ресурс: Groups

Шестнадцать групп DALI адаптера (`group_id` `0..15`), матрица членства «виртуальная
лампа × группа» и управление светом группы.

**Границы:** правила `PATCH` / `PUT` матриц и чанковой записи —
[`../stability-and-versioning.md`](../stability-and-versioning.md); цепочка
apply — [`../workflows.md`](../workflows.md); оркестратор —
[`../../runtime-modules/apply-orchestrator/README.md`](../../runtime-modules/apply-orchestrator/README.md).
BDD — [`groups`](../../../../tests/dali2rust-bdd/features/groups/).

## Маршруты

| Метод | Путь (`/api/v1/adapters/{adapter_id}/…`) | Назначение | Ответ |
|---|---|---|---|
| `GET` | `groups` | Список групп | `200` |
| `GET` | `groups/{group_id}` | Одна группа | `200` |
| `PATCH` | `groups/{group_id}` | `name`, `ha_entity_enabled` | `200` |
| `GET` | `group-membership-matrix` | Матрица членства | `200` |
| `PATCH` | `group-membership-matrix` | Изменить затронутые строки `desired` | `202` `config_write` |
| `PUT` | `group-membership-matrix` | Заменить `desired` целиком (64 строки) | `202` `config_write` |
| `POST` | `groups/apply` | Запрограммировать diff в приборы | `202` `group_apply` или `200` |
| `PUT` | `groups/{group_id}/target-state` | Свет группы | `202` после подтверждения |

`group_id` вне `0..15` — `400 invalid_resource_id`.

## Матрица: `desired` и `applied`

- Матрица всегда содержит все 64 слота ламп адаптера и по 16 флагов на строку
  (индекс — `group_id`).
- `desired` — намерение оператора, хранится в реестре. `applied` — **производная**
  от физического readback членства привязанного прибора (`QUERY GROUPS` отдаёт маску
  всех 16 групп); отдельно не хранится.
- `desired` строки, которую оператор ещё не трогал, засевается из первого
  наблюдённого членства — иначе первый apply снял бы группы, уже запрограммированные
  в приборах.
- `dirty` группы — есть строки, где `desired` ≠ `applied` в её колонке;
  `member_count_desired` / `member_count_applied` — число `true` в колонке;
  `capabilities_summary` — объединение effective capabilities строк, где группа в
  `desired`.
- Матрица не несёт ни уровня, ни цвета: свет группы — только через target-state.

## Запись матрицы

`desired` строки — массив ровно из 16 bool (индекс — `group_id`), заменяется целиком.
`applied`, `name` в строке и `groups`, `dirty` в корне — `422 unsupported_field`;
неверная длина, число строк или дубли — `422 invalid_value`. Ответ — `202`
`config_write`, чанковая запись.

## `POST groups/apply`

Общая цепочка (пустой diff, одна команда оркестратору, ячейки по одной, `applied` из
readback, частичный отказ) — [`../workflows.md`](../workflows.md) §Матрицы групп и сцен.

- Ячейка — пара «лампа × группа». Непривязанные строки остаются грязными
  (`vl_unbound` в результате). Результат суммирует запрограммированные, пропущенные и
  отказавшие ячейки.
- Второй apply на адаптере, пока первый идёт, — `409 conflict`; активная операция не
  отменяется.

## `PATCH groups/{group_id}`

Merge-patch: `name`, `ha_entity_enabled`. Derived-поля и идентификаторы — `422
unsupported_field`. Мост Home Assistant переобъявляет или отзывает сущность группы.

## `PUT groups/{group_id}/target-state`

Тело — [`../contracts/state-contracts.md`](../contracts/state-contracts.md), валидация
против `capabilities_summary`. Публикуется одна групповая команда DALI; ответ после
подтверждения — `202 {correlation_id, status: "accepted"}`: провод команду исполнил, а
состояние участников приходит позже, через проекцию state-fanout по
**applied**-членству.

Ошибки — как у target-state виртуальной лампы, кроме `vl_unbound`. Какие кадры
DALI выражают цвет и питание группы — правило протокола, а не контракт ресурса:
[`../../../architecture/09-dali-protocol-rules.md`](../../../architecture/09-dali-protocol-rules.md).
