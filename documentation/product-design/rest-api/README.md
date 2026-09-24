# REST API

Внешний HTTP/WebSocket-контракт контроллера: общие правила, индекс ресурсов и
контрактов. Каждый ресурс описан своим файлом в [`resources/`](resources/).

**Границы:** здесь — маршруты, семантика и дисциплина ответа. Форма полей — это
Rust DTO в `crates/dali2rust-api` (они канон); шинная сторона —
[`../bus-contracts/`](../bus-contracts/); поведение воркеров —
[`../runtime-modules/`](../runtime-modules/); экраны —
[`../web-ui/README.md`](../web-ui/README.md).

## Общие правила HTTP

- Префикс `/api/v1`, тела — JSON.
- `GET` читает read-модель и ничего не публикует; мутирующий маршрут публикует
  typed-команду на шину ([API gateway](../runtime-modules/api-gateway/README.md)).
- `PATCH` — `application/merge-patch+json` (RFC 7396), `PUT` — полная замена ресурса
  или матрицы, `POST` — действие или запуск операции, `DELETE` — удаление записи или
  связи. Правила merge-patch и матриц —
  [`stability-and-versioning.md`](stability-and-versioning.md); форма и словарь ошибок —
  [`contracts/error-dto.md`](contracts/error-dto.md).
- Каждый маршрут отвечает по одной из трёх дисциплин —
  [`stability-and-versioning.md`](stability-and-versioning.md) §Дисциплины ответа.
- Роль контроллера в каждом ответе и отказ записи на стэндбае —
  [`resources/redundancy.md`](resources/redundancy.md).
- Безопасность: `/api/v1` и `/api/v1/ws` — trusted-local, аутентификации нет;
  секреты (пароль брокера MQTT) только на запись.
- `GET /` и любой другой не-API `GET` отдают встроенный web UI из флеша.

## Ресурсы

Каждый документ ресурса ссылается на свой каталог исполняемых BDD-фич в
`tests/dali2rust-bdd/features/`.

| Ресурс | Маршруты (`/api/v1/…`) |
|---|---|
| [Controller](resources/controller.md) | `/controller`, `/health`, `/time` |
| [Adapters](resources/adapters.md) | `/adapters[/{adapter_id}]` |
| [Physical devices](resources/physical-devices.md) | `/adapters/{id}/physical-devices/…`, `/discovery-runs` |
| [Commissioning](resources/commissioning.md) | `/adapters/{id}/commissioning/…` |
| [Virtual lamps](resources/virtual-lamps.md) | `/adapters/{id}/virtual-lamps/…` |
| [Groups](resources/groups.md) | `/adapters/{id}/groups/…`, `/group-membership-matrix` |
| [Scenes](resources/scenes.md) | `/adapters/{id}/scenes/…` |
| [HCL schedules](resources/hcl.md) | `/hcl-schedules/…` |
| [Input devices](resources/input-devices.md) | `/adapters/{id}/input-devices/…` |
| [Rules](resources/rules.md) | `/rules`, `/rules/parse`, `/rules/{name}[/run]` |
| [Operations](resources/operations.md) | `/operations[/{id}]` |
| [Settings — Poller](resources/settings-poller.md) | `/settings/poller` |
| [Settings — DALI](resources/settings-dali.md) | `/settings/dali` |
| [Settings — Home Assistant](resources/settings-home-assistant.md) | `/settings/home-assistant[/discovery-publish]` |
| [Settings — Redundancy](resources/settings-redundancy.md) | `/settings/redundancy` |
| [Redundancy](resources/redundancy.md) | `/redundancy[/switchover]` |
| [Policies](resources/policies.md) | `/policies[/apply]` |
| [Config transfer](resources/config-transfer.md) | `/config/slices[/{slice}]` |
| [Firmware](resources/firmware.md) | `/firmware`, `/firmware/updates` |
| [Stats](resources/stats.md) | `/stats` |
| [Diagnostics](resources/diagnostics.md) | `/diagnostics` |
| [Diagnostic DALI](resources/diagnostic-dali.md) | `/dali/command`, `/dali/level`, `/dali/raw` |
| [WebSocket](resources/websocket.md) ([payloads](resources/websocket-payloads.md)) | `/ws` |

## Контракты

- [`contracts/state-contracts.md`](contracts/state-contracts.md) — общие формы
  состояния света: `LightSetpoint`, `RuntimeObservation`, `RuntimeStateContract`,
  тело target-state, строка сцены.
- [`contracts/enums.md`](contracts/enums.md) — enum'ы, чьё JSON-написание — контракт.
- [`contracts/error-dto.md`](contracts/error-dto.md) — формы ошибок и словарь кодов.
- [`stability-and-versioning.md`](stability-and-versioning.md) — стабильность,
  merge-patch, дисциплины ответа.
- [`workflows.md`](workflows.md) — сквозные цепочки REST → шина → воркер → реестр.
