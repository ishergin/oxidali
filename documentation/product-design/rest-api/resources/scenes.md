# Ресурс: Scenes

Шестнадцать сцен DALI адаптера (`scene_id` `0..15`): метаданные, матрица «виртуальная
лампа → setpoint сцены», программирование в приборы и нативный вызов.

**Границы:** строка матрицы (`SceneRow`: `desired`/`applied`, засев, сравнение) —
[`../contracts/state-contracts.md`](../contracts/state-contracts.md) §`SceneRow`;
правила `PATCH` / `PUT` матриц и чанковой записи —
[`../stability-and-versioning.md`](../stability-and-versioning.md); цепочки apply и
вызова — [`../workflows.md`](../workflows.md). BDD —
[`scenes`](../../../../tests/dali2rust-bdd/features/scenes/).

## Маршруты

| Метод | Путь (`/api/v1/adapters/{adapter_id}/scenes/…`) | Назначение | Ответ |
|---|---|---|---|
| `GET` | `` (коллекция) | Список сцен | `200` |
| `GET` | `{scene_id}` | Одна сцена | `200` |
| `PATCH` | `{scene_id}` | `name`, `ha_select_enabled` | `200` |
| `GET` | `{scene_id}/matrix` | Матрица сцены | `200` |
| `PATCH` | `{scene_id}/matrix` | Изменить затронутые строки `desired` | `202` `config_write` |
| `PUT` | `{scene_id}/matrix` | Заменить `desired` целиком (64 строки) | `202` `config_write` |
| `POST` | `{scene_id}/apply` | Запрограммировать diff в приборы | `202` `scene_apply` или `200` |
| `POST` | `{scene_id}/recall` | Нативный вызов сцены | `200` |

`scene_id` вне `0..15` — `400 invalid_resource_id`.

## Сцена и матрица

- `row_count_included` — число строк с `desired.included`; `dirty` — есть строки, где
  `desired` ≠ `applied`; `ha_select_enabled` — участвует ли сцена в выборе сцен адаптера
  в Home Assistant.
- Матрица всегда содержит все 64 слота ламп; ответ стримится построчно.
- Строка — `SceneRow` ([state-контракты](../contracts/state-contracts.md) §`SceneRow`):
  без runtime-наблюдений и без `transition`.

## Запись матрицы

Внутри строки `desired` действует merge-patch объекта.

- `included: true` — setpoint валидируется по capabilities строки (`422
  unsupported_capability`); `included: false` — все поля setpoint обязаны быть `null`
  (`422 invalid_value`).
- `applied`, `capabilities`, `dirty`, `name`, runtime-поля и `transition` в теле —
  `422 unsupported_field`; `virtual_lamp_id` вне `0..63`, неверное число строк или
  дубли — `422 invalid_value`.
- Ответ — `202` `config_write`, чанковая запись.

## `POST …/apply`

Общая цепочка (пустой diff, одна команда оркестратору, строки по одной, `applied` из
readback, частичный отказ) — [`../workflows.md`](../workflows.md) §Матрицы групп и сцен.

- Ячейка — строка: `Write` или `Update` для участвующих строк, `Clear` для тех, что
  больше не участвуют; readback — уровень и цвет сцены в приборе. Результат суммирует
  записанные, обновлённые, очищенные, пропущенные и отказавшие строки.
- Новый apply сцены, пока на адаптере идёт apply любой сцены, — `409 conflict`:
  два прогона на одной последовательной шине перемежались бы.

## `POST …/recall`

Нативный `GO TO SCENE` одним кадром — без развёртки по лампам и без проверки реестра:
прибор исполнит то, что в нём запрограммировано, даже если в реестре нет ни одной
applied-строки.

- Тело необязательно. Пустое — вызов для всего адаптера (broadcast);
  `{"scope": "group", "group_id": N}` — вызов для группы. Разбор строгий, потому что
  ошибка нестрогого разбора — молчаливый broadcast: неизвестный ключ — `422
  unsupported_field`, иной `scope` или `group_id` вне `0..15` — `422 invalid_value`.
- Синхронное подтверждение: `200 {correlation_id, status: "confirmed"}`; операции нет.
  `504 confirmation_timeout`, `503` — обычные; более новый вызов той же сцены с той же
  областью вытесняет неисполненный (`409 superseded`).
- Состояние ламп после вызова проецирует state-fanout по **applied**-строкам сцены.
